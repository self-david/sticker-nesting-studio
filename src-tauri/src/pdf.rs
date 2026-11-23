use crate::contour;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MM_TO_PT: f64 = 72.0 / 25.4;
const PRINT_DPI: f64 = 300.0;
const MAX_RASTER_PX: u32 = 4000;

pub struct Item {
    pub path: PathBuf,
    /// Top-left of the footprint on the page, the cut line included.
    pub x_mm: f64,
    /// Distance from the top edge of the page.
    pub y_mm: f64,
    /// Clockwise rotation in degrees: a multiple of 45.
    pub rotation: f64,
    /// Artwork box inside the footprint, and where it starts, before rotating.
    pub art_x_mm: f64,
    pub art_y_mm: f64,
    pub art_w_mm: f64,
    pub art_h_mm: f64,
}

impl Item {
    /// Artwork with no size cannot be drawn, and its file is never opened.
    fn has_art(&self) -> bool {
        self.art_w_mm > 0.0 && self.art_h_mm > 0.0
    }
}

pub struct PageSpec {
    pub width_mm: f64,
    pub height_mm: f64,
    pub items: Vec<Item>,
}

/// Cut outline drawn around each sticker's silhouette.
pub struct Outline {
    pub width_mm: f64,
    pub padding_mm: f64,
}

struct Raster {
    width: u32,
    height: u32,
    stream: Vec<u8>,
}

/// A unique source file: its real aspect ratio, the largest size it is printed
/// at, and its XObject index once loaded.
struct Source {
    aspect: f64,
    fitted_w: f64,
    fitted_h: f64,
    index: Option<usize>,
}

fn mm_to_pt(value: f64) -> f64 {
    value * MM_TO_PT
}

/// Pixels needed at 300 dpi for a given printed size.
fn px_for(mm: f64) -> u32 {
    ((mm.max(0.1) / 25.4 * PRINT_DPI).round() as i64).clamp(1, MAX_RASTER_PX as i64) as u32
}

/// Centre of the artwork in page points, plus the cosine and sine of its
/// clockwise rotation. The artwork is turned around its own centre inside the
/// footprint, so a 45° copy still lands where the sheet shows it.
fn item_transform(item: &Item, page_h_mm: f64) -> (f64, f64, f64, f64) {
    let cx = mm_to_pt(item.x_mm + item.art_x_mm + item.art_w_mm / 2.0);
    let cy = mm_to_pt(page_h_mm - item.y_mm - item.art_y_mm - item.art_h_mm / 2.0);
    let angle = item.rotation.to_radians();
    (cx, cy, angle.cos(), angle.sin())
}

fn key_of(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Fits an image of the given aspect ratio inside a box, keeping it centred.
fn fit_inside(box_w: f64, box_h: f64, aspect: f64) -> (f64, f64) {
    if box_w <= 0.0 || box_h <= 0.0 || aspect <= 0.0 || !aspect.is_finite() {
        return (box_w.max(0.0), box_h.max(0.0));
    }
    let (w, h) = (box_h * aspect, box_h);
    if w <= box_w {
        (w, h)
    } else {
        (box_w, box_w / aspect)
    }
}

fn deflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// Flattens RGBA over white so transparent artwork prints correctly.
fn flatten(img: &image::DynamicImage) -> Vec<u8> {
    if !img.color().has_alpha() {
        return img.to_rgb8().into_raw();
    }
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in rgba.pixels() {
        let a = px[3] as u32;
        let blend = |c: u8| -> u8 { ((c as u32 * a + 255 * (255 - a)) / 255) as u8 };
        out.push(blend(px[0]));
        out.push(blend(px[1]));
        out.push(blend(px[2]));
    }
    out
}

fn svg_tree(path: &Path) -> Result<resvg::usvg::Tree, String> {
    let data = fs::read(path).map_err(|e| format!("No se pudo leer el SVG: {e}"))?;
    resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("SVG no válido: {e}"))
}

fn render_svg(path: &Path, width_mm: f64, height_mm: f64) -> Result<(u32, u32, Vec<u8>), String> {
    let tree = svg_tree(path)?;
    let size = tree.size();
    let (tw, th) = (size.width() as f64, size.height() as f64);
    if tw <= 0.0 || th <= 0.0 {
        return Err("El SVG no declara un tamaño utilizable".into());
    }

    let scale_x = width_mm.max(0.1) / 25.4 * PRINT_DPI / tw;
    let scale_y = height_mm.max(0.1) / 25.4 * PRINT_DPI / th;
    // One shared scale keeps the aspect ratio exact and the size cap safe.
    let scale = scale_x.min(scale_y).min(MAX_RASTER_PX as f64 / tw.max(th));
    let w = ((tw * scale).round() as u32).max(1);
    let h = ((th * scale).round() as u32).max(1);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "No se pudo crear el lienzo del SVG".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / tw as f32, h as f32 / th as f32),
        &mut pixmap.as_mut(),
    );

    // tiny-skia pixels are premultiplied, so compositing over white is `c + (255 - a)`.
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in pixmap.data().chunks_exact(4) {
        let a = px[3] as u32;
        rgb.push((px[0] as u32 + 255 - a).min(255) as u8);
        rgb.push((px[1] as u32 + 255 - a).min(255) as u8);
        rgb.push((px[2] as u32 + 255 - a).min(255) as u8);
    }
    Ok((w, h, rgb))
}

fn intrinsic_aspect(path: &Path) -> Result<f64, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "svg" {
        let tree = svg_tree(path)?;
        let size = tree.size();
        let (w, h) = (size.width() as f64, size.height() as f64);
        if w <= 0.0 || h <= 0.0 {
            return Err("El SVG no declara un tamaño utilizable".into());
        }
        return Ok(w / h);
    }

    let (w, h) = image::image_dimensions(path)
        .map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    if w == 0 || h == 0 {
        return Err("La imagen está vacía".into());
    }
    Ok(w as f64 / h as f64)
}

/// Loads an image at no more than 300 dpi for the printed size it will occupy.
fn load_raster(path: &Path, width_mm: f64, height_mm: f64) -> Result<Raster, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "svg" {
        let (width, height, rgb) = render_svg(path, width_mm, height_mm)?;
        return Ok(Raster {
            width,
            height,
            stream: deflate(&rgb)?,
        });
    }

    let img = image::open(path).map_err(|e| format!("No se pudo abrir la imagen: {e}"))?;
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 {
        return Err("La imagen está vacía".into());
    }

    // Never upscale; one shared scale so the stored aspect matches the source.
    let scale = (px_for(width_mm) as f64 / width as f64)
        .min(px_for(height_mm) as f64 / height as f64)
        .min(1.0);
    let img = if scale < 1.0 {
        let target_w = ((width as f64 * scale).round() as u32).max(1);
        let target_h = ((height as f64 * scale).round() as u32).max(1);
        img.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let (width, height) = (img.width(), img.height());
    let rgb = flatten(&img);
    Ok(Raster {
        width,
        height,
        stream: deflate(&rgb)?,
    })
}

fn stream_object(data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

pub fn export(dest: &Path, pages: &[PageSpec], outline: Option<&Outline>) -> Result<(), String> {
    if pages.is_empty() {
        return Err("La hoja no tiene páginas que exportar".into());
    }
    for page in pages {
        if !(page.width_mm > 0.0 && page.height_mm > 0.0) {
            return Err("La página tiene un tamaño no válido".into());
        }
    }

    // Unique files, in order of appearance.
    let mut unique: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for page in pages {
        for item in &page.items {
            if !item.has_art() {
                continue;
            }
            let key = key_of(&item.path);
            if seen.insert(key.clone()) {
                unique.push((key, item.path.clone()));
            }
        }
    }

    // Real aspect ratio per file, then the largest size it is printed at; that
    // footprint decides the raster resolution.
    let mut sources: HashMap<String, Source> = HashMap::new();
    for (key, path) in &unique {
        let aspect = intrinsic_aspect(path).map_err(|e| format!("{}: {e}", path.display()))?;
        sources.insert(
            key.clone(),
            Source {
                aspect,
                fitted_w: 0.0,
                fitted_h: 0.0,
                index: None,
            },
        );
    }
    for page in pages {
        for item in &page.items {
            if !item.has_art() {
                continue;
            }
            let source = sources
                .get_mut(&key_of(&item.path))
                .expect("file registered above");
            let (w_mm, h_mm) = fit_inside(item.art_w_mm, item.art_h_mm, source.aspect);
            source.fitted_w = source.fitted_w.max(w_mm);
            source.fitted_h = source.fitted_h.max(h_mm);
        }
    }

    let mut shapes: HashMap<String, contour::Shape> = HashMap::new();
    let mut images: Vec<Raster> = Vec::new();
    for (key, path) in &unique {
        let source = sources.get_mut(key).expect("file registered above");
        let raster = load_raster(path, source.fitted_w, source.fitted_h)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        source.index = Some(images.len());
        images.push(raster);
    }

    let page_count = pages.len();
    let page_ids: Vec<usize> = (0..page_count).map(|i| 3 + i * 2).collect();
    let content_ids: Vec<usize> = (0..page_count).map(|i| 4 + i * 2).collect();
    let image_base = 3 + page_count * 2;

    let mut objects: Vec<Vec<u8>> = Vec::with_capacity(2 + page_count * 2 + images.len());
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());

    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>").into_bytes());

    let xobjects = images
        .iter()
        .enumerate()
        .map(|(i, _)| format!("/Im{i} {} 0 R", image_base + i))
        .collect::<Vec<_>>()
        .join(" ");

    for (page_idx, page) in pages.iter().enumerate() {
        let width_pt = mm_to_pt(page.width_mm);
        let height_pt = mm_to_pt(page.height_mm);

        let mut content = String::new();
        for item in &page.items {
            let Some(source) = sources.get(&key_of(&item.path)) else {
                continue;
            };
            let (Some(idx), aspect) = (source.index, source.aspect) else {
                continue;
            };
            if !item.has_art() {
                continue;
            }
            // Same "contain" behaviour as the on-screen preview.
            let (w_mm, h_mm) = fit_inside(item.art_w_mm, item.art_h_mm, aspect);
            let w = mm_to_pt(w_mm);
            let h = mm_to_pt(h_mm);
            let (cx, cy, cos, sin) = item_transform(item, page.height_mm);
            // Turn around the footprint centre, then centre the image on it.
            content.push_str(&format!(
                "q\n{cos:.6} {:.6} {sin:.6} {cos:.6} {cx:.4} {cy:.4} cm\n{w:.4} 0 0 {h:.4} {:.4} {:.4} cm\n/Im{} Do\nQ\n",
                -sin,
                -w / 2.0,
                -h / 2.0,
                idx
            ));
        }

        if let Some(outline) = outline {
            // One continuous line, so the cut follows the sticker without a
            // dash pattern restarting on every contour.
            content.push_str(&format!(
                "{:.4} w 0.55 0.55 0.55 RG\n",
                mm_to_pt(outline.width_mm.max(0.05))
            ));
            let mut stroked = false;
            for item in &page.items {
                let Some(source) = sources.get(&key_of(&item.path)) else {
                    continue;
                };
                if !item.has_art() {
                    continue;
                }
                let (w_mm, h_mm) = fit_inside(item.art_w_mm, item.art_h_mm, source.aspect);
                let (cx, cy, cos, sin) = item_transform(item, page.height_mm);
                let art_w = mm_to_pt(w_mm);
                let art_h = mm_to_pt(h_mm);

                let cache_key = format!(
                    "{}|{w_mm:.3}x{h_mm:.3}|{:.2}",
                    key_of(&item.path),
                    outline.padding_mm
                );
                let shape = shapes.entry(cache_key).or_insert_with(|| {
                    contour::shape_from_file(&item.path, w_mm, h_mm, outline.padding_mm)
                        .unwrap_or_else(|_| contour::Shape::rectangle())
                });

                for polygon in &shape.contours {
                    if polygon.len() < 2 {
                        continue;
                    }
                    for (i, (nx, ny)) in polygon.iter().enumerate() {
                        // Artwork-local point, y up, relative to the art centre.
                        let lx = nx * art_w - art_w / 2.0;
                        let ly = art_h / 2.0 - ny * art_h;
                        let px = cx + cos * lx + sin * ly;
                        let py = cy - sin * lx + cos * ly;
                        content.push_str(&format!(
                            "{px:.4} {py:.4} {}\n",
                            if i == 0 { "m" } else { "l" }
                        ));
                    }
                    content.push_str("h\n");
                    stroked = true;
                }
            }
            if stroked {
                content.push_str("S\n");
            }
        }

        let content_bytes = content.into_bytes();
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.4} {:.4}] /Resources << /XObject << {} >> >> /Contents {} 0 R >>",
                width_pt, height_pt, xobjects, content_ids[page_idx]
            )
            .into_bytes(),
        );
        objects.push(stream_object(&content_bytes));
    }

    for image in &images {
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            image.width,
            image.height,
            image.stream.len()
        )
        .into_bytes();
        body.extend_from_slice(&image.stream);
        body.extend_from_slice(b"\nendstream");
        objects.push(body);
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let mut offsets: Vec<usize> = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref_pos = out.len();
    let total = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {total}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {total} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n")
            .as_bytes(),
    );

    fs::write(dest, &out).map_err(|e| format!("No se pudo guardar el PDF: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sns-pdf-{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_multi_page_pdf() {
        let dir = temp_dir("pages");
        let img_path = dir.join("sticker.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 128]))
            .save(&img_path)
            .unwrap();

        let page = || PageSpec {
            width_mm: 210.0,
            height_mm: 297.0,
            items: vec![Item {
                path: img_path.clone(),
                x_mm: 10.0,
                y_mm: 20.0,
                rotation: 0.0,
                art_x_mm: 0.0,
                art_y_mm: 0.0,
                art_w_mm: 50.0,
                art_h_mm: 50.0,
            }],
        };
        let dest = dir.join("out.pdf");
        export(&dest, &[page(), page(), page()], None).unwrap();

        let bytes = fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 3"));
        // Every page carries its own drawing, never just the first one.
        assert_eq!(text.matches(" Do\n").count(), 3);
        assert!(text.contains("%%EOF"));
        assert!(text.contains("/Subtype /Image"));
        let startxref = text.rsplit("startxref").next().unwrap();
        let offset: usize = startxref
            .trim()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(bytes[offset..].starts_with(b"xref"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn downscales_raster_to_print_resolution() {
        let dir = temp_dir("downscale");
        let img_path = dir.join("big.png");
        image::RgbaImage::from_pixel(1200, 1200, image::Rgba([0, 0, 255, 255]))
            .save(&img_path)
            .unwrap();

        let dest = dir.join("down.pdf");
        export(
            &dest,
            &[PageSpec {
                width_mm: 100.0,
                height_mm: 100.0,
                items: vec![Item {
                    path: img_path,
                    x_mm: 0.0,
                    y_mm: 0.0,
                    rotation: 0.0,
                    art_x_mm: 0.0,
                    art_y_mm: 0.0,
                    art_w_mm: 25.4,
                    art_h_mm: 25.4,
                }],
            }],
            None,
        )
        .unwrap();

        let bytes = fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        // 25.4 mm at 300 dpi is exactly 300 px, down from the 1200 px source.
        assert!(text.contains("/Width 300 /Height 300"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_aspect_ratio_inside_the_cut_box() {
        assert_eq!(fit_inside(100.0, 100.0, 2.0), (100.0, 50.0));
        assert_eq!(fit_inside(100.0, 100.0, 0.5), (50.0, 100.0));
        assert_eq!(fit_inside(100.0, 50.0, 1.0), (50.0, 50.0));
        assert_eq!(fit_inside(80.0, 40.0, 80.0 / 40.0), (80.0, 40.0));
        // Degenerate values must not produce NaN or negative sizes.
        assert_eq!(fit_inside(100.0, 100.0, 0.0), (100.0, 100.0));
        assert_eq!(fit_inside(0.0, 50.0, 1.0), (0.0, 50.0));
        assert_eq!(fit_inside(100.0, 100.0, f64::NAN), (100.0, 100.0));
    }

    #[test]
    fn pixel_math_edges() {
        assert_eq!(px_for(25.4), 300);
        assert_eq!(px_for(0.0), 1);
        // A4 height at 300 dpi, below the safety cap.
        assert_eq!(px_for(297.0), 3508);
        assert_eq!(px_for(10_000.0), MAX_RASTER_PX);
    }

    #[test]
    fn downscales_wide_raster_without_distorting_it() {
        let dir = temp_dir("wide");
        let img_path = dir.join("wide.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([10, 200, 30, 255]))
            .save(&img_path)
            .unwrap();

        let dest = dir.join("wide.pdf");
        export(
            &dest,
            &[PageSpec {
                width_mm: 100.0,
                height_mm: 100.0,
                items: vec![Item {
                    path: img_path,
                    x_mm: 0.0,
                    y_mm: 0.0,
                    // Square box for a 2:1 image: the art is letterboxed.
                    rotation: 0.0,
                    art_x_mm: 0.0,
                    art_y_mm: 0.0,
                    art_w_mm: 25.4,
                    art_h_mm: 25.4,
                }],
            }],
            None,
        )
        .unwrap();

        let bytes = fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Width 300 /Height 150"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn embeds_each_file_once_at_its_largest_size() {
        let dir = temp_dir("reuse");
        let img_path = dir.join("reuse.png");
        image::RgbaImage::from_pixel(1200, 1200, image::Rgba([40, 40, 40, 255]))
            .save(&img_path)
            .unwrap();

        let dest = dir.join("reuse.pdf");
        let item = |mm: f64| Item {
            path: img_path.clone(),
            x_mm: 0.0,
            y_mm: 0.0,
            rotation: 0.0,
            art_x_mm: 0.0,
            art_y_mm: 0.0,
            art_w_mm: mm,
            art_h_mm: mm,
        };
        export(
            &dest,
            &[PageSpec {
                width_mm: 210.0,
                height_mm: 297.0,
                items: vec![item(25.4), item(50.8)],
            }],
            None,
        )
        .unwrap();

        let bytes = fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        // 50.8 mm needs 600 px, so the shared XObject is stored at that size.
        assert!(text.contains("/Width 600 /Height 600"));
        assert_eq!(text.matches("/Subtype /Image").count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn outline_follows_the_silhouette() {
        let dir = temp_dir("outline");
        let path = dir.join("circle.png");
        let mut img = image::RgbaImage::from_pixel(120, 120, image::Rgba([0, 0, 0, 0]));
        for y in 0..120i32 {
            for x in 0..120i32 {
                let (dx, dy) = (x as f64 - 60.0, y as f64 - 60.0);
                if dx * dx + dy * dy <= 50.0 * 50.0 {
                    img.put_pixel(x as u32, y as u32, image::Rgba([200, 40, 40, 255]));
                }
            }
        }
        img.save(&path).unwrap();

        let dest = dir.join("outline.pdf");
        export(
            &dest,
            &[PageSpec {
                width_mm: 100.0,
                height_mm: 100.0,
                items: vec![Item {
                    path,
                    x_mm: 10.0,
                    y_mm: 10.0,
                    rotation: 0.0,
                    art_x_mm: 0.0,
                    art_y_mm: 0.0,
                    art_w_mm: 50.0,
                    art_h_mm: 50.0,
                }],
            }],
            Some(&Outline {
                width_mm: 0.3,
                padding_mm: 1.0,
            }),
        )
        .unwrap();

        let bytes = fs::read(&dest).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(" m\n"), "outline path should start with a move");
        assert!(text.matches(" l\n").count() > 12, "outline should have many segments");
        assert!(text.contains("h\nS\n"), "outline path should be stroked");

        let _ = fs::remove_dir_all(&dir);
    }

    fn item(rotation: f64) -> Item {
        Item {
            path: PathBuf::from("x.png"),
            x_mm: 10.0,
            y_mm: 20.0,
            rotation,
            art_x_mm: 5.0,
            art_y_mm: 3.0,
            art_w_mm: 50.0,
            art_h_mm: 34.0,
        }
    }

    #[test]
    fn the_rotation_turns_the_artwork_around_its_own_centre() {
        let (cx, cy, cos, sin) = item_transform(&item(90.0), 297.0);
        assert!((cx - mm_to_pt(40.0)).abs() < 1e-9);
        assert!((cy - mm_to_pt(297.0 - 20.0 - 20.0)).abs() < 1e-9);
        assert!(cos.abs() < 1e-9 && (sin - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_missing_file() {
        let dir = temp_dir("missing");
        let dest = dir.join("out.pdf");
        let result = export(
            &dest,
            &[PageSpec {
                width_mm: 100.0,
                height_mm: 100.0,
                items: vec![Item {
                    path: dir.join("nope.png"),
                    x_mm: 0.0,
                    y_mm: 0.0,
                    rotation: 0.0,
                    art_x_mm: 0.0,
                    art_y_mm: 0.0,
                    art_w_mm: 10.0,
                    art_h_mm: 10.0,
                }],
            }],
            None,
        );
        assert!(result.is_err());
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}


