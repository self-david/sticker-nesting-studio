use image::GenericImageView;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const THUMB_MAX: u32 = 420;
/// Pixels at least this opaque count as content when trimming the margins.
pub const TRIM_ALPHA: u8 = 8;

pub struct Probe {
    pub kind: &'static str,
    /// The stored file lost empty margins on the way in.
    pub trimmed: bool,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: f64,
    pub has_alpha: bool,
}

pub fn allowed_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "svg")
}

/// Parse a CSS-like length in an SVG attribute into millimetres.
/// Bare numbers and `px` are interpreted at the given DPI.
pub fn parse_length(raw: &str, dpi: f64) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() || raw.ends_with('%') {
        return None;
    }
    let split = raw
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+' || *c == 'e' || *c == 'E'))
        .map(|(i, _)| i)
        .unwrap_or(raw.len());
    let (num, unit) = raw.split_at(split);
    let value: f64 = num.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let mm = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => value * 25.4 / dpi,
        "mm" => value,
        "cm" => value * 10.0,
        "in" => value * 25.4,
        "pt" => value * 25.4 / 72.0,
        "pc" => value * 25.4 / 6.0,
        _ => return None,
    };
    if mm.is_finite() && mm > 0.0 {
        Some(mm)
    } else {
        None
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{}=", name);
    let bytes = tag.as_bytes();
    let idx = tag.find(&needle)?;
    // make sure it is a real attribute name (preceded by whitespace)
    if idx > 0 {
        let prev = bytes[idx - 1] as char;
        if !prev.is_whitespace() {
            return None;
        }
    }
    let rest = &tag[idx + needle.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Physical size of an SVG in millimetres, from width/height or the viewBox.
pub fn svg_size(text: &str) -> Option<(f64, f64)> {
    let start = text.find("<svg")?;
    let end = text[start..].find('>')? + start;
    let tag = &text[start..end];
    let dpi = 96.0;
    let w = attr(tag, "width").and_then(|v| parse_length(&v, dpi));
    let h = attr(tag, "height").and_then(|v| parse_length(&v, dpi));
    let view = attr(tag, "viewBox").or_else(|| attr(tag, "viewbox"));
    let ratio = view.and_then(|v| {
        let nums: Vec<f64> = v
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();
        if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
            Some((nums[2], nums[3]))
        } else {
            None
        }
    });

    match (w, h, ratio) {
        (Some(w), Some(h), _) => Some((w, h)),
        (Some(w), None, Some((vw, vh))) => Some((w, w * vh / vw)),
        (None, Some(h), Some((vw, vh))) => Some((h * vw / vh, h)),
        (None, None, Some((vw, vh))) => Some((vw * 25.4 / dpi, vh * 25.4 / dpi)),
        _ => None,
    }
}

/// Physical resolution the file itself declares, in dots per inch. `None` means
/// the artwork carries no density, so the caller falls back to 300 dpi.
fn declared_dpi(bytes: &[u8], ext: &str) -> Option<f64> {
    match ext {
        "png" => png_dpi(bytes),
        "jpg" | "jpeg" => jpeg_dpi(bytes),
        _ => None,
    }
}

/// PNG stores its density in the `pHYs` chunk, in pixels per metre.
fn png_dpi(bytes: &[u8]) -> Option<f64> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return None;
    }
    let mut at = 8usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
        let kind = &bytes[at + 4..at + 8];
        let start = at + 8;
        let end = start.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        if kind == &b"pHYs"[..] && len >= 9 {
            let x = u32::from_be_bytes(bytes[start..start + 4].try_into().ok()?) as f64;
            let unit = bytes[start + 8];
            // Unit 1 means metres; 0 is an aspect ratio, not a physical size.
            return (unit == 1 && x > 0.0).then_some(x * 0.0254);
        }
        // The rest of the file is pixels: no density will appear now.
        if kind == &b"IDAT"[..] || kind == &b"IEND"[..] {
            return None;
        }
        at = end.checked_add(4)?; // skip the CRC
    }
    None
}

/// JPEG declares density in the JFIF header (APP0) or, more precisely, in EXIF
/// (APP1). Both are read: exporters write one, the other or both.
fn jpeg_dpi(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut at = 2usize;
    while at + 1 < bytes.len() {
        if bytes[at] != 0xFF {
            at += 1; // resync on a damaged stream
            continue;
        }
        let mut mark = at + 1;
        while mark < bytes.len() && bytes[mark] == 0xFF {
            mark += 1;
        }
        if mark >= bytes.len() {
            return None;
        }
        let marker = bytes[mark];
        at = mark + 1;
        match marker {
            // Markers without a length field, and the start of the scan, where
            // no more metadata can appear.
            0x01 | 0xD0..=0xD8 | 0xD9 | 0xDA => {
                if marker == 0xD9 || marker == 0xDA {
                    return None;
                }
                continue;
            }
            _ => {}
        }
        if at + 2 > bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        if len < 2 || at + len > bytes.len() {
            return None;
        }
        let segment = &bytes[at + 2..at + len];
        if marker == 0xE0 && segment.starts_with(b"JFIF\0") && segment.len() >= 12 {
            let units = segment[7];
            let density = u16::from_be_bytes([segment[8], segment[9]]) as f64;
            let dpi = match units {
                1 => density,          // dots per inch
                2 => density * 2.54,   // dots per centimetre
                _ => 0.0,              // only an aspect ratio
            };
            return (dpi > 0.0).then_some(dpi);
        }
        if marker == 0xE1 && segment.starts_with(b"Exif\0\0") {
            if let Some(dpi) = exif_dpi(&segment[6..]) {
                return Some(dpi);
            }
        }
        at += len;
    }
    None
}

/// Reads `XResolution` and `ResolutionUnit` from the first IFD of a TIFF block.
fn exif_dpi(tiff: &[u8]) -> Option<f64> {
    if tiff.len() < 8 {
        return None;
    }
    let big = match &tiff[..2] {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let u16_at = |offset: usize| -> Option<u16> {
        let bytes = tiff.get(offset..offset + 2)?;
        Some(if big {
            u16::from_be_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_le_bytes([bytes[0], bytes[1]])
        })
    };
    let u32_at = |offset: usize| -> Option<u32> {
        let bytes = tiff.get(offset..offset + 4)?;
        Some(if big {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        })
    };
    let rational_at = |offset: usize| -> Option<f64> {
        let num = u32_at(offset)? as f64;
        let den = u32_at(offset + 4)? as f64;
        (den > 0.0).then_some(num / den)
    };

    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    let mut resolution: Option<f64> = None;
    let mut unit: u16 = 2; // inches, the EXIF default
    for index in 0..count {
        let entry = ifd.checked_add(2 + index * 12)?;
        let tag = u16_at(entry)?;
        let value = entry + 8;
        match tag {
            0x011A => resolution = rational_at(u32_at(value)? as usize),
            0x0128 => unit = u16_at(value).unwrap_or(2),
            _ => {}
        }
    }
    let dpi = resolution?;
    if !(dpi > 0.0) {
        return None;
    }
    Some(if unit == 3 { dpi * 2.54 } else { dpi })
}

/// Density within the range a sticker can sensibly carry, rounded to hundredths.
fn sane_dpi(value: f64) -> f64 {
    round2(value.clamp(30.0, 2400.0))
}

/// The density the file on disk declares. `None` means the artwork carries no
/// physical size: WebP always, and PNG or JPEG without density metadata.
/// Used to correct stickers imported before the density was read.
pub fn declared_density(path: &Path, ext: &str) -> Result<Option<f64>, String> {
    let bytes = fs::read(path).map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    Ok(declared_dpi(&bytes, ext).map(sane_dpi))
}

pub fn probe(src: &Path, ext: &str, thumbs: &Path, id: &str) -> Result<Probe, String> {
    if ext == "svg" {
        let text = fs::read_to_string(src).map_err(|e| format!("No se pudo leer el SVG: {e}"))?;
        if !text.contains("<svg") {
            return Err("El archivo no parece un SVG válido".into());
        }
        let (w, h) = svg_size(&text).ok_or_else(|| {
            "El SVG no declara tamaño (width/height) ni viewBox".to_string()
        })?;
        return Ok(Probe {
            kind: "vector",
            trimmed: false,
            width_px: 0.0,
            height_px: 0.0,
            width_mm: round2(w),
            height_mm: round2(h),
            dpi: 96.0,
            has_alpha: true,
        });
    }

    // Read once: the density lives in the container, not in the pixels.
    let bytes = fs::read(src).map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
    let loaded = image::load_from_memory(&bytes)
        .map_err(|e| format!("No se pudo decodificar la imagen: {e}"))?;
    let has_alpha = loaded.color().has_alpha();
    // Transparent margins are not part of the sticker: cropping them here
    // makes every sheet denser without touching the nester.
    let trimmed_img = if ext == "png" && has_alpha {
        trim_margins(&loaded, src)
    } else {
        None
    };
    let trimmed = trimmed_img.is_some();
    let img = trimmed_img.unwrap_or(loaded);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("La imagen está vacía".into());
    }
    // The file's own density when it declares a sane one; 300 dpi is only the
    // fallback for artwork that carries no physical size at all.
    let dpi = declared_dpi(&bytes, ext).map(sane_dpi).unwrap_or(300.0);

    let thumb = img.thumbnail(THUMB_MAX, THUMB_MAX);
    let thumb_ext = if has_alpha { "png" } else { "jpg" };
    let thumb_path = thumbs.join(format!("{id}.{thumb_ext}"));
    let saved = if has_alpha {
        thumb.save_with_format(&thumb_path, image::ImageFormat::Png)
    } else {
        thumb.save_with_format(&thumb_path, image::ImageFormat::Jpeg)
    };
    saved.map_err(|e| format!("No se pudo crear la miniatura: {e}"))?;

    Ok(Probe {
        kind: "raster",
        trimmed,
        width_px: w as f64,
        height_px: h as f64,
        width_mm: round2(w as f64 * 25.4 / dpi),
        height_mm: round2(h as f64 * 25.4 / dpi),
        dpi,
        has_alpha,
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('_').replace('_', " ");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        "Sticker".to_string()
    } else {
        cleaned
    }
}

pub fn target_file(files: &Path, id: &str, ext: &str) -> PathBuf {
    files.join(format!("{id}.{ext}"))
}

/// SHA-256 of a file's bytes: the same artwork imported twice gives the same
/// value, so the library does not collect copies of what it already has.
pub fn hash_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Box around the pixels that carry ink, or `None` when the whole image is
/// empty. Empty canvas is common in exported artwork and costs nesting room.
fn content_box(img: &image::DynamicImage) -> Option<(u32, u32, u32, u32)> {
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if rgba.get_pixel(x, y)[3] > TRIM_ALPHA {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

/// Drops the transparent border of a PNG and writes the result back over the
/// stored copy. Other formats are left alone: only PNG is both lossless and
/// written by this crate.
fn trim_margins(img: &image::DynamicImage, path: &Path) -> Option<image::DynamicImage> {
    let (x, y, w, h) = content_box(&img)?;
    if x == 0 && y == 0 && w == img.width() && h == img.height() {
        return None;
    }
    let cropped = image::DynamicImage::ImageRgba8(
        image::imageops::crop_imm(img, x, y, w, h).to_image(),
    );
    cropped
        .save_with_format(path, image::ImageFormat::Png)
        .ok()?;
    Some(cropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal PNG signature plus one `pHYs` chunk.
    fn png_with_density(ppm: u32, unit: u8) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&9u32.to_be_bytes());
        bytes.extend_from_slice(b"pHYs");
        bytes.extend_from_slice(&ppm.to_be_bytes());
        bytes.extend_from_slice(&ppm.to_be_bytes());
        bytes.push(unit);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC, not read
        bytes
    }

    fn jpeg_with_jfif(units: u8, density: u16) -> Vec<u8> {
        let mut segment = b"JFIF\0".to_vec();
        segment.extend_from_slice(&[1, 1]);
        segment.push(units);
        segment.extend_from_slice(&density.to_be_bytes());
        segment.extend_from_slice(&density.to_be_bytes());
        segment.extend_from_slice(&[0, 0]);
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend_from_slice(&((segment.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(&segment);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        bytes
    }

    /// APP1/EXIF with `XResolution` stored out of line, as cameras do.
    fn jpeg_with_exif(dpi: u32, unit: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        // XResolution -> RATIONAL at offset 38 (after the two entries).
        tiff.extend_from_slice(&0x011Au16.to_le_bytes());
        tiff.extend_from_slice(&5u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&38u32.to_le_bytes());
        // ResolutionUnit -> SHORT, inline.
        tiff.extend_from_slice(&0x0128u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&(unit as u32).to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        tiff.extend_from_slice(&dpi.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());

        let mut segment = b"Exif\0\0".to_vec();
        segment.extend_from_slice(&tiff);
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1];
        bytes.extend_from_slice(&((segment.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(&segment);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        bytes
    }

    #[test]
    fn reads_the_density_the_file_declares() {
        // 11811 px/m is 300 dpi; PNG stores metres, JPEG stores inches.
        let png = png_with_density(11811, 1);
        assert!((declared_dpi(&png, "png").unwrap() - 300.0).abs() < 0.1);

        let jfif = jpeg_with_jfif(1, 300);
        assert!((declared_dpi(&jfif, "jpg").unwrap() - 300.0).abs() < 1e-6);

        // Dots per centimetre: 118 dpcm is 299.72 dpi.
        let cm = jpeg_with_jfif(2, 118);
        assert!((declared_dpi(&cm, "jpeg").unwrap() - 299.72).abs() < 0.01);

        let exif = jpeg_with_exif(150, 2);
        assert!((declared_dpi(&exif, "jpg").unwrap() - 150.0).abs() < 1e-6);

        // EXIF resolution in centimetres.
        let exif_cm = jpeg_with_exif(60, 3);
        assert!((declared_dpi(&exif_cm, "jpg").unwrap() - 152.4).abs() < 1e-6);

        // An aspect-ratio-only pHYs and a file without metadata mean "unknown".
        assert!(declared_dpi(&png_with_density(11811, 0), "png").is_none());
        assert!(declared_dpi(b"\xFF\xD8\xFF\xD9", "jpg").is_none());
        assert!(declared_dpi(&png, "webp").is_none());
    }

    #[test]
    fn probe_reads_the_file_and_falls_back_to_300() {
        let dir = std::env::temp_dir().join(format!("sns-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cuadrado.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();

        // A fresh PNG from the encoder declares no density: 300 dpi is used.
        let probe = probe(&path, "png", &dir, "prueba").unwrap();
        assert!((probe.dpi - 300.0).abs() < 1e-6);
        assert!((probe.width_mm - 8.0 * 25.4 / 300.0).abs() < 0.01);
        assert!(probe.has_alpha);
        assert!(dir.join("prueba.png").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trims_the_transparent_margins_of_a_png() {
        let dir = std::env::temp_dir().join(format!("sns-trim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("margen.png");
        let mut img = image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 0]));
        for y in 5..15 {
            for x in 5..15 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        img.save(&path).unwrap();

        let trimmed_probe = probe(&path, "png", &dir, "recorte").unwrap();
        assert!(trimmed_probe.trimmed);
        assert!((trimmed_probe.width_px - 10.0).abs() < 1e-6);
        assert!((trimmed_probe.height_px - 10.0).abs() < 1e-6);
        // The stored copy lost the border too, so the PDF sees the same artwork.
        let stored = image::open(&path).unwrap();
        assert_eq!((stored.width(), stored.height()), (10, 10));

        // Artwork that fills its canvas is left exactly as it came.
        let full = dir.join("lleno.png");
        image::RgbaImage::from_pixel(12, 12, image::Rgba([0, 0, 255, 255]))
            .save(&full)
            .unwrap();
        let untouched = probe(&full, "png", &dir, "lleno").unwrap();
        assert!(!untouched.trimmed);
        assert!((untouched.width_px - 12.0).abs() < 1e-6);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lengths() {
        assert!((parse_length("25.4mm", 96.0).unwrap() - 25.4).abs() < 1e-6);
        assert!((parse_length("2cm", 96.0).unwrap() - 20.0).abs() < 1e-6);
        assert!((parse_length("1in", 96.0).unwrap() - 25.4).abs() < 1e-6);
        assert!((parse_length("96px", 96.0).unwrap() - 25.4).abs() < 1e-6);
        assert!((parse_length("72pt", 72.0).unwrap() - 25.4).abs() < 1e-6);
        assert!(parse_length("100%", 96.0).is_none());
    }

    #[test]
    fn svg_dimensions() {
        let a = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 400 300"></svg>"#;
        let (w, h) = svg_size(a).unwrap();
        assert!((w - 40.0).abs() < 1e-6 && (h - 30.0).abs() < 1e-6);

        // Only viewBox: falls back to 96 dpi.
        let b = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 192"></svg>"#;
        let (w, h) = svg_size(b).unwrap();
        assert!((w - 25.4).abs() < 1e-6 && (h - 50.8).abs() < 1e-6);

        // Width only + viewBox ratio.
        let c = r#"<svg width="50mm" viewBox="0 0 100 50"></svg>"#;
        let (w, h) = svg_size(c).unwrap();
        assert!((w - 50.0).abs() < 1e-6 && (h - 25.0).abs() < 1e-6);

        assert!(svg_size(r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#).is_none());
    }
}
