//! How the artwork is kept on disk. Every sticker lives as a PNG: an SVG is
//! baked at 300 dpi for the size it declares, and every PNG goes through
//! oxipng, which only rewrites the compression stream. Pixels, colours and
//! alpha are untouched, so nothing on the sheet changes.

use crate::{db, importer};
use serde::Serialize;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Print resolution used when an SVG becomes pixels: the PDF draws at 300 dpi,
/// so nothing is lost when the vector disappears.
const SVG_DPI: f64 = 300.0;
/// Same ceiling as the PDF raster, so a huge SVG cannot allocate a giant canvas.
const MAX_SIDE: u32 = 4000;
/// Above this many decoded bytes a PNG is left alone. A 60-megapixel export
/// takes oxipng several seconds to walk and still shrinks by a fraction of a
/// percent: the time is worth far more than those kilobytes.
const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;

/// Decoded size of a PNG straight from its header, without decoding a pixel.
fn decoded_bytes(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let word = |at: usize| -> Option<u64> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as u64)
    };
    let (w, h) = (word(16)?, word(20)?);
    let bit_depth = bytes[24] as u64;
    let channels = match bytes[25] {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    Some(w * h * channels * bit_depth / 8)
}

fn svg_to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
        .map_err(|e| format!("SVG no válido: {e}"))?;
    let size = tree.size();
    let (tw, th) = (size.width() as f64, size.height() as f64);
    if tw <= 0.0 || th <= 0.0 {
        return Err("El SVG no declara un tamaño utilizable".into());
    }

    let text = String::from_utf8_lossy(bytes);
    // The physical size the file declares, or 96 dpi pixels when it has none.
    let (mm_w, mm_h) =
        importer::svg_size(&text).unwrap_or((tw * 25.4 / 96.0, th * 25.4 / 96.0));
    let target_w = mm_w.max(0.1) / 25.4 * SVG_DPI;
    let target_h = mm_h.max(0.1) / 25.4 * SVG_DPI;
    let scale = (target_w / tw)
        .min(target_h / th)
        .min(MAX_SIDE as f64 / tw.max(th));
    let w = ((tw * scale).round() as u32).max(1);
    let h = ((th * scale).round() as u32).max(1);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "No se pudo crear el lienzo del SVG".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / tw as f32, h as f32 / th as f32),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied alpha; PNG stores it straight.
    let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for px in pixmap.data().chunks_exact(4) {
        let a = px[3] as u32;
        if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            rgba.push((px[0] as u32 * 255 / a).min(255) as u8);
            rgba.push((px[1] as u32 * 255 / a).min(255) as u8);
            rgba.push((px[2] as u32 * 255 / a).min(255) as u8);
            rgba.push(a as u8);
        }
    }
    let image = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| "No se pudo componer el PNG del SVG".to_string())?;
    let mut out = Vec::new();
    {
        let mut cursor = Cursor::new(&mut out);
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| format!("No se pudo escribir el PNG: {e}"))?;
    }
    Ok(out)
}

/// Rewrites the PNG's compression stream at a level that is quick enough for a
/// whole library and still plainly smaller than the encoder's default.
pub fn compress_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if decoded_bytes(bytes).is_some_and(|size| size > MAX_DECODED_BYTES) {
        return Ok(bytes.to_vec());
    }
    let mut options = oxipng::Options::from_preset(2);
    // A sticker is not worth a minute of CPU: once the budget is gone the
    // best stream found so far is kept, which is still lossless.
    options.timeout = Some(std::time::Duration::from_millis(800));
    oxipng::optimize_from_memory(bytes, &options)
        .map_err(|e| format!("No se pudo comprimir el PNG: {e}"))
}

/// What an import writes: an SVG travels as a PNG, everything else as it came.
pub fn prepare_bytes(bytes: &[u8], ext: &str) -> Result<(Vec<u8>, String), String> {
    if ext.eq_ignore_ascii_case("svg") {
        return Ok((svg_to_png(bytes)?, "png".to_string()));
    }
    Ok((bytes.to_vec(), ext.to_ascii_lowercase()))
}

/// Unique name next to the file, so a failure never leaves it half written.
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "La imagen no tiene carpeta".to_string())?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sticker");
    let temp = dir.join(format!("{name}.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, bytes).map_err(|e| format!("No se pudo escribir la imagen: {e}"))?;
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("No se pudo guardar la imagen: {e}"));
    }
    Ok(())
}

/// Recompresses a PNG in place when the smaller stream is a real win. Returns
/// the bytes saved, 0 when the file was already tight.
pub fn optimize_file(path: &Path) -> Result<u64, String> {
    let original = fs::read(path).map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    let smaller = compress_png(&original)?;
    if smaller.len() >= original.len() {
        return Ok(0);
    }
    write_replacing(path, &smaller)?;
    Ok((original.len() - smaller.len()) as u64)
}

/// Stores one artwork at its final place. An SVG is turned into a PNG next to
/// it and the vector removed; a PNG is recompressed where it is.
pub fn prepare_file(path: &Path, ext: &str) -> Result<(PathBuf, String), String> {
    if ext.eq_ignore_ascii_case("svg") {
        let bytes = fs::read(path).map_err(|e| format!("No se pudo leer el SVG: {e}"))?;
        let (png, stored_ext) = prepare_bytes(&bytes, "svg")?;
        let png = compress_png(&png).unwrap_or(png);
        let target = path.with_extension(&stored_ext);
        write_replacing(&target, &png)?;
        if target != path {
            let _ = fs::remove_file(path);
        }
        return Ok((target, stored_ext));
    }
    if ext.eq_ignore_ascii_case("png") {
        optimize_file(path)?;
    }
    Ok((path.to_path_buf(), ext.to_ascii_lowercase()))
}

// --- whole library sweep ---------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepFailure {
    pub name: String,
    pub message: String,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    pub checked: usize,
    pub converted: usize,
    pub compressed: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub failures: Vec<SweepFailure>,
}

pub struct Sweep {
    pub report: SweepReport,
    pub updates: Vec<db::ArtworkUpdate>,
}

struct Done {
    before: u64,
    after: u64,
    converted: bool,
    compressed: bool,
    update: Option<db::ArtworkUpdate>,
}

fn sweep_one(
    row: &db::ArtworkRow,
    files: &Path,
    thumbs: &Path,
) -> Result<Done, String> {
    let path = files.join(&row.file_name);
    let ext = row.ext.to_ascii_lowercase();
    let bytes = fs::read(&path).map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    let before = bytes.len() as u64;

    if ext == "svg" {
        let (png, stored_ext) = prepare_bytes(&bytes, "svg")?;
        let png = compress_png(&png).unwrap_or(png);
        let file_name = format!("{}.{stored_ext}", row.id);
        let target = files.join(&file_name);
        write_replacing(&target, &png)?;
        if target != path {
            let _ = fs::remove_file(&path);
        }
        let (width_px, height_px, has_alpha) = importer::thumbnail(&target, thumbs, &row.id)?;
        if has_alpha {
            let _ = optimize_file(&thumbs.join(format!("{}.png", row.id)));
        }
        let size_bytes = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        return Ok(Done {
            before,
            after: png.len() as u64,
            converted: true,
            compressed: false,
            update: Some(db::ArtworkUpdate {
                id: row.id.clone(),
                file_name,
                ext: stored_ext,
                kind: "raster".to_string(),
                width_px: width_px as f64,
                height_px: height_px as f64,
                dpi: SVG_DPI,
                has_alpha,
                size_bytes: size_bytes as i64,
            }),
        });
    }

    if ext == "png" {
        let smaller = compress_png(&bytes)?;
        let compressed = smaller.len() < bytes.len();
        if compressed {
            write_replacing(&path, &smaller)?;
        }
        // The grid loads the preview far more often than the artwork, so the
        // same pass tightens it as well.
        let thumb = thumbs.join(format!("{}.png", row.id));
        if thumb.is_file() {
            let _ = optimize_file(&thumb);
        }
        let after = fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(before);
        return Ok(Done {
            before,
            after,
            converted: false,
            compressed,
            update: Some(db::ArtworkUpdate {
                id: row.id.clone(),
                file_name: row.file_name.clone(),
                ext: ext.clone(),
                kind: row.kind.clone(),
                width_px: row.width_px,
                height_px: row.height_px,
                dpi: row.dpi,
                has_alpha: row.has_alpha,
                size_bytes: after as i64,
            }),
        });
    }

    // JPEG and WebP are already compressed lossily: re-encoding them would
    // lose quality, so they are left byte for byte as they are.
    Ok(Done {
        before,
        after: before,
        converted: false,
        compressed: false,
        update: None,
    })
}

/// Applies the storage rules to everything the library already holds, a few
/// files at a time on every core: the files are independent, and a whole
/// library of large artwork would otherwise take a whole afternoon. One
/// failing file never stops the sweep.
pub fn sweep(files: &Path, thumbs: &Path, rows: Vec<db::ArtworkRow>) -> Sweep {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
        .min(rows.len().max(1));
    let per = rows.len().div_ceil(workers).max(1);

    let parts: Vec<(Vec<db::ArtworkUpdate>, SweepReport)> = std::thread::scope(|scope| {
        let handles: Vec<_> = rows
            .chunks(per)
            .map(|part| {
                scope.spawn(move || {
                    let mut report = SweepReport::default();
                    let mut updates = Vec::new();
                    for row in part {
                        report.checked += 1;
                        if !files.join(&row.file_name).is_file() {
                            report.failures.push(SweepFailure {
                                name: row.name.clone(),
                                message: "El archivo de la imagen no está".into(),
                            });
                            continue;
                        }
                        match sweep_one(row, files, thumbs) {
                            Ok(done) => {
                                report.bytes_before += done.before;
                                report.bytes_after += done.after;
                                if done.converted {
                                    report.converted += 1;
                                }
                                if done.compressed {
                                    report.compressed += 1;
                                }
                                if let Some(update) = done.update {
                                    updates.push(update);
                                }
                            }
                            Err(message) => report.failures.push(SweepFailure {
                                name: row.name.clone(),
                                message,
                            }),
                        }
                    }
                    (updates, report)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });

    let mut report = SweepReport::default();
    let mut updates = Vec::new();
    for (part_updates, part_report) in parts {
        report.checked += part_report.checked;
        report.converted += part_report.converted;
        report.compressed += part_report.compressed;
        report.bytes_before += part_report.bytes_before;
        report.bytes_after += part_report.bytes_after;
        report.failures.extend(part_report.failures);
        updates.extend(part_updates);
    }
    Sweep { report, updates }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_becomes_a_png_at_its_declared_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm" viewBox="0 0 100 100">
            <rect width="100" height="100" fill="#ff0000"/>
        </svg>"##;
        let (png, ext) = prepare_bytes(svg, "svg").unwrap();
        assert_eq!(ext, "png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        // 10 mm at 300 dpi is 118 px on each side.
        assert_eq!((image.width(), image.height()), (118, 118));
        assert_eq!(image.get_pixel(59, 59), &image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn a_huge_png_is_left_untouched() {
        let mut header = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        header.extend_from_slice(&13u32.to_be_bytes());
        header.extend_from_slice(b"IHDR");
        header.extend_from_slice(&8000u32.to_be_bytes());
        header.extend_from_slice(&8000u32.to_be_bytes());
        header.push(8); // bit depth
        header.push(6); // RGBA
        header.extend_from_slice(&[0, 0, 0]);
        header.extend_from_slice(&[0, 0, 0, 0]); // CRC, not read

        assert_eq!(decoded_bytes(&header), Some(256_000_000));
        // The size guard answers before oxipng decodes a single pixel.
        assert_eq!(compress_png(&header).unwrap(), header);
    }

    #[test]
    fn compressing_a_png_keeps_every_pixel() {
        let dir = std::env::temp_dir().join(format!("sns-opt-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("arte.png");
        let mut img = image::RgbaImage::new(64, 64);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgba([(x * 4) as u8, (y * 4) as u8, 90, 255]);
        }
        img.save(&path).unwrap();
        let before = image::open(&path).unwrap().to_rgba8();

        optimize_file(&path).unwrap();

        let after = image::open(&path).unwrap().to_rgba8();
        assert_eq!(before.dimensions(), after.dimensions());
        assert!(
            before.pixels().eq(after.pixels()),
            "oxipng must not change a single pixel"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sweep_turns_a_stored_svg_into_a_png_row() {
        let root = std::env::temp_dir().join(format!("sns-sweep-{}", uuid::Uuid::new_v4()));
        let files = root.join("files");
        let thumbs = root.join("thumbs");
        fs::create_dir_all(&files).unwrap();
        fs::create_dir_all(&thumbs).unwrap();
        fs::write(
            files.join("s1.svg"),
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 200 100"><circle cx="50" cy="50" r="40" fill="#0a0"/></svg>"##,
        )
        .unwrap();

        let sweep = sweep(
            &files,
            &thumbs,
            vec![db::ArtworkRow {
                id: "s1".into(),
                name: "Círculo".into(),
                file_name: "s1.svg".into(),
                ext: "svg".into(),
                kind: "vector".into(),
                width_px: 0.0,
                height_px: 0.0,
                dpi: 96.0,
                has_alpha: true,
            }],
        );

        assert_eq!(sweep.report.converted, 1);
        assert!(sweep.report.failures.is_empty());
        assert!(!files.join("s1.svg").exists());
        assert!(files.join("s1.png").is_file());
        let update = &sweep.updates[0];
        assert_eq!(update.ext, "png");
        assert_eq!(update.kind, "raster");
        assert!((update.dpi - 300.0).abs() < 1e-6);
        assert!(update.width_px > 0.0 && update.height_px > 0.0);
        assert!(Path::new(&format!("{}/s1.png", thumbs.display())).is_file());
        let _ = fs::remove_dir_all(&root);
    }
}
