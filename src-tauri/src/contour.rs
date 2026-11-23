use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Longest side used to trace silhouettes. Enough for print-sized outlines.
pub(crate) const MAX_SIDE: u32 = 800;
/// Solid artwork only: soft drop shadows, glows and compression noise are not
/// the shape. Sticker artwork usually carries a wide, near-invisible halo, and
/// tracing it is what made the cut line wander far from the artwork.
const ALPHA_THRESHOLD: u8 = 200;
const SIMPLIFY_PX: f64 = 0.8;
const MIN_AREA_PX: f64 = 6.0;
/// Isolated pixels would be blown up by the padding into random blobs.
const MIN_SPECKS_PX: usize = 16;
/// Padding larger than this share of the artwork is clamped, so a huge value
/// on a small copy cannot balloon the traced canvas.
const MAX_PAD_FRACTION: f64 = 0.5;

pub type Point = (f64, f64);

/// Silhouette outlines in image-normalized coordinates (0..1, y down).
pub struct Shape {
    pub contours: Vec<Vec<Point>>,
}

impl Shape {
    pub fn rectangle() -> Self {
        Shape {
            contours: vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]],
        }
    }
}

/// Traces the visible silhouette of a sticker, grown outwards by `padding_mm`.
pub fn shape_from_file(
    path: &Path,
    width_mm: f64,
    height_mm: f64,
    padding_mm: f64,
) -> Result<Shape, String> {
    shape_from_file_at(path, width_mm, height_mm, padding_mm, MAX_SIDE)
}

/// Same trace, worked out on a coarser image. Nesting only needs the shape to
/// fit a one millimetre grid, and a small canvas is many times faster.
pub fn shape_from_file_at(
    path: &Path,
    width_mm: f64,
    height_mm: f64,
    padding_mm: f64,
    max_side: u32,
) -> Result<Shape, String> {
    let (rgba, w, h) = load_rgba(path, max_side)?;
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 {
        return Err("La imagen está vacía".into());
    }

    let mask: Vec<bool> = (0..w * h)
        .map(|i| rgba[i * 4 + 3] >= ALPHA_THRESHOLD)
        .collect();
    if mask.iter().all(|&value| value) {
        // Opaque artwork has no silhouette to follow.
        return Ok(Shape::rectangle());
    }

    let pad_px = if padding_mm > 0.0 && width_mm > 0.0 && height_mm > 0.0 {
        let scale_x = w as f64 / width_mm;
        let scale_y = h as f64 / height_mm;
        let raw = padding_mm * (scale_x + scale_y) / 2.0;
        raw.min(w.max(h) as f64 * MAX_PAD_FRACTION)
    } else {
        0.0
    };
    let mask = drop_specks(&mask, w, h);
    // Trace on a canvas with a margin: artwork that bleeds to the file edge
    // must still let the padding grow outwards, and the outline must never be
    // clipped flush against the image bounds.
    let margin = pad_px.ceil() as usize + 2;
    let (pw, ph) = (w + margin * 2, h + margin * 2);
    let mut canvas = vec![false; pw * ph];
    for y in 0..h {
        let src = y * w;
        let dst = (y + margin) * pw + margin;
        canvas[dst..dst + w].copy_from_slice(&mask[src..src + w]);
    }
    let canvas = if pad_px > 0.4 {
        grow(&canvas, pw, ph, pad_px)
    } else {
        canvas
    };

    let mut loops: Vec<(f64, Vec<Point>)> = Vec::new();
    for polygon in trace(&canvas, pw, ph) {
        let area = signed_area(&polygon);
        if polygon.len() < 3 || area.abs() < MIN_AREA_PX {
            continue;
        }
        let simplified = simplify(&polygon);
        if simplified.len() < 3 {
            continue;
        }
        loops.push((
            area,
            simplified
                .into_iter()
                .map(|(x, y)| {
                    (
                        (x - margin as f64) / w as f64,
                        (y - margin as f64) / h as f64,
                    )
                })
                .collect(),
        ));
    }

    // A sticker is cut along its outer edge only. Gaps inside the artwork (the
    // space between an arm and the body, the middle of a letter) are painted,
    // not cut, and outlining them drew a second line inside the sticker. Those
    // holes run the other way round, so keeping the dominant winding leaves the
    // silhouettes and drops the holes.
    let dominant = loops
        .iter()
        .map(|(area, _)| *area)
        .fold(0.0f64, |best: f64, area: f64| {
            if area.abs() > best.abs() {
                area
            } else {
                best
            }
        });
    let contours: Vec<Vec<Point>> = loops
        .into_iter()
        .filter(|(area, _)| area * dominant > 0.0)
        .map(|(_, polygon)| polygon)
        .collect();

    if contours.is_empty() {
        return Ok(Shape::rectangle());
    }
    Ok(Shape { contours })
}

pub(crate) fn load_rgba(path: &Path, max_side: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "svg" {
        return load_svg(path, max_side);
    }

    let img = image::open(path).map_err(|e| format!("No se pudo abrir la imagen: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let longest = w.max(h);
    let img = if longest > max_side {
        let scale = max_side as f64 / longest as f64;
        img.resize_exact(
            ((w as f64 * scale).round() as u32).max(1),
            ((h as f64 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok((rgba.into_raw(), w, h))
}

/// SVG rendered with alpha, at a size suitable for tracing.
fn load_svg(path: &Path, max_side: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let data = fs::read(path).map_err(|e| format!("No se pudo leer el SVG: {e}"))?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("SVG no válido: {e}"))?;
    let size = tree.size();
    let (tw, th) = (size.width() as f64, size.height() as f64);
    if tw <= 0.0 || th <= 0.0 {
        return Err("El SVG no declara un tamaño utilizable".into());
    }

    let scale = max_side as f64 / tw.max(th);
    let w = ((tw * scale).round() as u32).max(1);
    let h = ((th * scale).round() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "No se pudo crear el lienzo del SVG".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / tw as f32, h as f32 / th as f32),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied alpha; convert back before thresholding.
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
    Ok((rgba, w, h))
}

/// Grows the mask outwards by `pad_px` using a chamfer distance transform.
fn grow(mask: &[bool], w: usize, h: usize, pad_px: f64) -> Vec<bool> {
    const NEIGHBOUR: f64 = 1.0;
    const DIAGONAL: f64 = std::f64::consts::SQRT_2;
    let mut dist = vec![0.0f64; w * h];
    for (i, &filled) in mask.iter().enumerate() {
        if !filled {
            dist[i] = f64::INFINITY;
        }
    }

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut d = dist[i];
            if x > 0 {
                d = d.min(dist[i - 1] + NEIGHBOUR);
            }
            if y > 0 {
                d = d.min(dist[i - w] + NEIGHBOUR);
                if x > 0 {
                    d = d.min(dist[i - w - 1] + DIAGONAL);
                }
                if x + 1 < w {
                    d = d.min(dist[i - w + 1] + DIAGONAL);
                }
            }
            dist[i] = d;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut d = dist[i];
            if x + 1 < w {
                d = d.min(dist[i + 1] + NEIGHBOUR);
            }
            if y + 1 < h {
                d = d.min(dist[i + w] + NEIGHBOUR);
                if x + 1 < w {
                    d = d.min(dist[i + w + 1] + DIAGONAL);
                }
                if x > 0 {
                    d = d.min(dist[i + w - 1] + DIAGONAL);
                }
            }
            dist[i] = d;
        }
    }

    (0..w * h).map(|i| mask[i] || dist[i] <= pad_px).collect()
}

/// Removes tiny isolated islands before padding: a few noisy pixels become
/// large blobs once grown, which is what makes the outline wander far away.
fn drop_specks(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let mut island: Vec<usize> = Vec::new();

    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        island.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(index) = stack.pop() {
            island.push(index);
            let (x, y) = (index % w, index / w);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let next = ny as usize * w + nx as usize;
                    if mask[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        if island.len() < MIN_SPECKS_PX {
            for index in &island {
                out[*index] = false;
            }
        }
    }
    out
}

/// Chains the boundary edges of the mask into closed polygons.
fn trace(mask: &[bool], w: usize, h: usize) -> Vec<Vec<Point>> {
    let filled = |x: i32, y: i32| -> bool {
        x >= 0 && y >= 0 && x < w as i32 && y < h as i32 && mask[y as usize * w + x as usize]
    };

    let mut edges: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if !filled(x, y) {
                continue;
            }
            if !filled(x, y - 1) {
                edges.entry((x, y)).or_default().push((x + 1, y));
            }
            if !filled(x + 1, y) {
                edges.entry((x + 1, y)).or_default().push((x + 1, y + 1));
            }
            if !filled(x, y + 1) {
                edges.entry((x + 1, y + 1)).or_default().push((x, y + 1));
            }
            if !filled(x - 1, y) {
                edges.entry((x, y + 1)).or_default().push((x, y));
            }
        }
    }

    let mut loops: Vec<Vec<Point>> = Vec::new();
    while let Some(start) = edges
        .iter()
        .find(|(_, outgoing)| !outgoing.is_empty())
        .map(|(point, _)| *point)
    {
        let mut polygon: Vec<Point> = Vec::new();
        let mut current = start;
        let mut guard = 0usize;
        loop {
            let Some(outgoing) = edges.get_mut(&current) else {
                break;
            };
            if outgoing.is_empty() {
                edges.remove(&current);
                break;
            }
            let next = outgoing.remove(0);
            polygon.push((current.0 as f64, current.1 as f64));
            current = next;
            guard += 1;
            if current == start || guard > w * h * 8 {
                break;
            }
        }
        if !polygon.is_empty() {
            loops.push(polygon);
        }
    }
    loops
}

/// Shoelace sum. Its sign tells an outer edge from a hole.
fn signed_area(points: &[Point]) -> f64 {
    let mut sum = 0.0;
    for i in 0..points.len() {
        let (x1, y1) = points[i];
        let (x2, y2) = points[(i + 1) % points.len()];
        sum += x1 * y2 - x2 * y1;
    }
    sum / 2.0
}

fn simplify(points: &[Point]) -> Vec<Point> {
    if points.len() < 4 {
        return points.to_vec();
    }
    // Start at an extreme vertex so the closing edge is not a long chord.
    let mut start = 0;
    for (i, p) in points.iter().enumerate() {
        if p.0 < points[start].0 || (p.0 == points[start].0 && p.1 < points[start].1) {
            start = i;
        }
    }
    let mut ordered = Vec::with_capacity(points.len());
    ordered.extend_from_slice(&points[start..]);
    ordered.extend_from_slice(&points[..start]);
    ordered.push(ordered[0]);
    let mut simplified = rdp(&ordered, SIMPLIFY_PX);
    simplified.pop();
    simplified
}

fn rdp(points: &[Point], epsilon: f64) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let mut farthest = 0.0;
    let mut split = 0;
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let distance = point_line_distance(*p, a, b);
        if distance > farthest {
            farthest = distance;
            split = i;
        }
    }
    if farthest <= epsilon {
        return vec![a, b];
    }
    let mut left = rdp(&points[..=split], epsilon);
    let right = rdp(&points[split..], epsilon);
    left.pop();
    left.extend(right);
    left
}

fn point_line_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / length
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sns-contour-{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn circle(path: &Path) {
        let mut img = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));
        for y in 0..200i32 {
            for x in 0..200i32 {
                let (dx, dy) = (x as f64 - 100.0, y as f64 - 100.0);
                if dx * dx + dy * dy <= 80.0 * 80.0 {
                    img.put_pixel(x as u32, y as u32, Rgba([255, 0, 0, 255]));
                }
            }
        }
        img.save(path).unwrap();
    }

    fn bounds(shape: &Shape) -> (f64, f64, f64, f64) {
        let mut min_x: f64 = 1.0;
        let mut min_y: f64 = 1.0;
        let mut max_x: f64 = 0.0;
        let mut max_y: f64 = 0.0;
        for polygon in &shape.contours {
            for (x, y) in polygon {
                min_x = min_x.min(*x);
                min_y = min_y.min(*y);
                max_x = max_x.max(*x);
                max_y = max_y.max(*y);
            }
        }
        (min_x, min_y, max_x, max_y)
    }

    #[test]
    fn follows_a_round_silhouette() {
        let dir = temp_dir("circle");
        let path = dir.join("circle.png");
        circle(&path);

        let shape = shape_from_file(&path, 50.0, 50.0, 0.0).unwrap();
        assert_eq!(shape.contours.len(), 1);
        let (min_x, min_y, max_x, max_y) = bounds(&shape);
        // Tighter than a pixel at the 800 px trace size: the outline must sit
        // on the artwork, not merely near it.
        assert!((min_x - 0.1).abs() < 0.008, "min_x = {min_x}");
        assert!((max_x - 0.9).abs() < 0.008, "max_x = {max_x}");
        assert!((min_y - 0.1).abs() < 0.008, "min_y = {min_y}");
        assert!((max_y - 0.9).abs() < 0.008, "max_y = {max_y}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hole_inside_the_artwork_is_not_a_cut_line() {
        let dir = temp_dir("donut");
        let path = dir.join("donut.png");
        let mut img = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));
        for y in 0..200i32 {
            for x in 0..200i32 {
                let (dx, dy) = (x as f64 - 100.0, y as f64 - 100.0);
                let d = (dx * dx + dy * dy).sqrt();
                if (40.0..=80.0).contains(&d) {
                    img.put_pixel(x as u32, y as u32, Rgba([255, 0, 0, 255]));
                }
            }
        }
        img.save(&path).unwrap();

        let shape = shape_from_file(&path, 50.0, 50.0, 0.0).unwrap();
        // One line around the ring: the see-through middle is not outlined.
        assert_eq!(shape.contours.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn padding_grows_the_outline() {
        let dir = temp_dir("padding");
        let path = dir.join("circle.png");
        circle(&path);

        let tight = shape_from_file(&path, 50.0, 50.0, 0.0).unwrap();
        let padded = shape_from_file(&path, 50.0, 50.0, 3.0).unwrap();
        let (tight_min, _, tight_max, _) = bounds(&tight);
        let (padded_min, _, padded_max, _) = bounds(&padded);
        assert!(padded_min < tight_min - 0.02, "padding should extend left");
        assert!(padded_max > tight_max + 0.02, "padding should extend right");
        assert!(padded_min >= 0.0 && padded_max <= 1.0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn padding_is_not_clipped_by_the_image_bounds() {
        let dir = temp_dir("bleed");
        let path = dir.join("bleed.png");
        // Artwork that runs off the right edge of the file.
        let mut img = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));
        for y in 0..200u32 {
            for x in 0..200u32 {
                let dx = x as f64 - 200.0;
                let dy = y as f64 - 100.0;
                if dx * dx + dy * dy <= 90.0 * 90.0 {
                    img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
                }
            }
        }
        img.save(&path).unwrap();

        let tight = shape_from_file(&path, 80.0, 80.0, 0.0).unwrap();
        let (_, _, tight_max, _) = bounds(&tight);
        let padded = shape_from_file(&path, 80.0, 80.0, 4.0).unwrap();
        let (_, _, padded_max, _) = bounds(&padded);

        // The silhouette stops at the file edge, the padded outline must not.
        assert!(tight_max <= 1.0 + 1e-9, "tight max = {tight_max}");
        assert!(padded_max > 1.02, "padding should leave the art, got {padded_max}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn noise_specks_do_not_distort_a_large_image() {
        let dir = temp_dir("specks");
        let path = dir.join("big.png");
        // Large artwork downscaled for tracing, with stray alpha dots around it.
        let mut img = RgbaImage::from_pixel(1600, 1600, Rgba([0, 0, 0, 0]));
        for y in 0..1600u32 {
            for x in 0..1600u32 {
                let dx = x as f64 - 800.0;
                let dy = y as f64 - 800.0;
                if dx * dx + dy * dy <= 500.0 * 500.0 {
                    img.put_pixel(x, y, Rgba([200, 40, 90, 255]));
                }
            }
        }
        for i in 0..400u32 {
            let x = (i * 97 + 13) % 1600;
            let y = (i * 53 + 29) % 1600;
            let dx = x as f64 - 800.0;
            let dy = y as f64 - 800.0;
            if dx * dx + dy * dy > 700.0 * 700.0 {
                img.put_pixel(x, y, Rgba([200, 40, 90, 140]));
            }
        }
        img.save(&path).unwrap();

        // 2 mm of padding is 16 px at this size, so 150-16 .. 650+16 in pixels.
        let shape = shape_from_file(&path, 100.0, 100.0, 2.0).unwrap();
        assert_eq!(shape.contours.len(), 1, "specks became their own outlines");
        let (min_x, _, max_x, _) = bounds(&shape);
        assert!((min_x - (134.0 / 800.0)).abs() < 0.015, "min_x = {min_x}");
        assert!((max_x - (666.0 / 800.0)).abs() < 0.015, "max_x = {max_x}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn huge_padding_on_a_small_copy_is_clamped() {
        let dir = temp_dir("clamp");
        let path = dir.join("circle.png");
        circle(&path);

        // 20 mm of padding on a 2 mm copy would need 8000 px of canvas.
        let shape = shape_from_file(&path, 2.0, 2.0, 20.0).unwrap();
        let (min_x, min_y, max_x, max_y) = bounds(&shape);
        assert!(min_x.is_finite() && max_x.is_finite());
        assert!(min_x < 0.0 && max_x > 1.0, "padding should still grow outwards");
        assert!(min_x > -3.0 && max_x < 4.0, "padding ran away: {min_x}..{max_x}");
        assert!(min_y < 0.0 && max_y > 1.0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_soft_halo_does_not_move_the_cut_line() {
        let dir = temp_dir("halo");
        let path = dir.join("halo.png");
        // Solid disc with a wide, half-transparent glow around it.
        let mut img = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));
        for y in 0..200i32 {
            for x in 0..200i32 {
                let (dx, dy) = (x as f64 - 100.0, y as f64 - 100.0);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= 50.0 {
                    img.put_pixel(x as u32, y as u32, Rgba([200, 40, 40, 255]));
                } else if d <= 90.0 {
                    img.put_pixel(x as u32, y as u32, Rgba([200, 40, 40, 120]));
                }
            }
        }
        img.save(&path).unwrap();

        let shape = shape_from_file(&path, 50.0, 50.0, 0.0).unwrap();
        assert_eq!(shape.contours.len(), 1);
        let (min_x, _, max_x, _) = bounds(&shape);
        // Halfway through the glow would be ~0.05; the solid disc is 0.25..0.75.
        assert!((min_x - 0.25).abs() < 0.01, "min_x = {min_x}");
        assert!((max_x - 0.75).abs() < 0.01, "max_x = {max_x}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn opaque_artwork_stays_a_rectangle() {
        let dir = temp_dir("opaque");
        let path = dir.join("photo.jpg");
        image::RgbImage::from_pixel(40, 40, image::Rgb([120, 40, 200]))
            .save(&path)
            .unwrap();

        let shape = shape_from_file(&path, 20.0, 20.0, 2.0).unwrap();
        assert_eq!(shape.contours.len(), 1);
        assert_eq!(shape.contours[0].len(), 4);

        let _ = fs::remove_dir_all(&dir);
    }

    /// Measures the cost of tracing a full sheet: eight files at five sizes
    /// each, the way distinct copy sizes multiply the work.
    #[test]
    #[ignore]
    fn timing_varied_copies() {
        let dir = temp_dir("bench");
        let mut files = Vec::new();
        for i in 0..8u32 {
            let path = dir.join(format!("art{i}.png"));
            let mut img = RgbaImage::from_pixel(800, 640, Rgba([0, 0, 0, 0]));
            let base = 210.0 + i as f64 * 14.0;
            for y in 0..640u32 {
                for x in 0..800u32 {
                    let dx = x as f64 - 400.0;
                    let dy = y as f64 - 320.0;
                    let theta = dy.atan2(dx);
                    let radius = base * (1.0 + 0.16 * (theta * 7.0 + i as f64).sin());
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        img.put_pixel(x, y, Rgba([20, 120, 200, 255]));
                    }
                }
            }
            img.save(&path).unwrap();
            files.push(path);
        }

        let sizes = [35.0, 48.0, 62.0, 75.0, 92.0];
        let mut traced = 0usize;
        let mut worst = 0.0f64;
        let start = std::time::Instant::now();
        for path in &files {
            for size in sizes {
                let timer = std::time::Instant::now();
                let shape = shape_from_file(path, size, size * 0.8, 1.5).unwrap();
                worst = worst.max(timer.elapsed().as_secs_f64() * 1000.0);
                assert!(!shape.contours.is_empty());
                traced += 1;
            }
        }
        let total = start.elapsed().as_secs_f64() * 1000.0;

        // Same sheet again, eight tracers at once: the app now runs these on
        // blocking threads, so this is what the user actually waits for.
        let parallel_start = std::time::Instant::now();
        std::thread::scope(|scope| {
            for path in &files {
                for size in sizes {
                    scope.spawn(move || {
                        shape_from_file(path, size, size * 0.8, 1.5).unwrap();
                    });
                }
            }
        });
        let parallel = parallel_start.elapsed().as_secs_f64() * 1000.0;

        println!(
            "{traced} trazos distintos: serie {total:.1} ms · media {:.1} ms · peor {worst:.1} ms · en paralelo {parallel:.1} ms",
            total / traced as f64
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn traces_svg_alpha() {
        let dir = temp_dir("svg");
        let path = dir.join("star.svg");
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 100 100"><circle cx="50" cy="50" r="45" fill="#8b5cf6"/></svg>"##,
        )
        .unwrap();

        let shape = shape_from_file(&path, 40.0, 40.0, 1.0).unwrap();
        assert_eq!(shape.contours.len(), 1);
        let (min_x, _, max_x, _) = bounds(&shape);
        assert!(min_x > 0.0 && max_x < 1.0);

        let _ = fs::remove_dir_all(&dir);
    }
}

