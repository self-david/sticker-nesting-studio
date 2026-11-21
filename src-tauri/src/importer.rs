use image::GenericImageView;
use std::fs;
use std::path::{Path, PathBuf};

pub const THUMB_MAX: u32 = 420;

pub struct Probe {
    pub kind: &'static str,
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
            width_px: 0.0,
            height_px: 0.0,
            width_mm: round2(w),
            height_mm: round2(h),
            dpi: 96.0,
            has_alpha: true,
        });
    }

    let img = image::open(src).map_err(|e| format!("No se pudo decodificar la imagen: {e}"))?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("La imagen está vacía".into());
    }
    let has_alpha = img.color().has_alpha();
    let dpi = 300.0;

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

#[cfg(test)]
mod tests {
    use super::*;

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
