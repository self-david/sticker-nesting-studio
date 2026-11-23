//! Nesting por silueta: cada copia se convierte en una máscara de su contorno
//! real y se coloca contra las demás máscaras, no contra su caja. Así el aire
//! de un sticker con forma de espada lo puede ocupar otro, que es justo lo que
//! un empaquetado de rectángulos no puede hacer.
//!
//! La rejilla es de 1 mm: suficiente para respetar la separación y el relleno
//! del contorno, y lo bastante basta para que 977 copias se coloquen en
//! segundos. Cada copia se prueba en los ocho giros de 45°.

use crate::contour;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Side of one cell, in millimetres.
const RES: f64 = 1.0;
/// Turns tried for every copy. 45° keeps the artwork readable and lets long or
/// slanted stickers use the slants of the ones already placed.
const ANGLES: [f64; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestCopy {
    pub id: String,
    pub path: String,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Angle the copy was turned to by hand. It is tried first, so pressing the
    /// rotate button does something and a reopened sheet keeps its turns.
    #[serde(default)]
    pub rotation: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestSettings {
    pub paper: String,
    pub orientation: String,
    pub margin_mm: f64,
    pub gap_mm: f64,
    pub cut_lines: bool,
    pub outline_width_mm: f64,
    pub outline_pad_mm: f64,
    #[serde(default = "default_page_w")]
    pub custom_width_mm: f64,
    #[serde(default = "default_page_h")]
    pub custom_height_mm: f64,
}

fn default_page_w() -> f64 {
    210.0
}

fn default_page_h() -> f64 {
    297.0
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placed {
    pub copy_id: String,
    pub x_mm: f64,
    pub y_mm: f64,
    /// Box the copy occupies on the sheet, cut line included.
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotation: f64,
    /// Where the artwork box sits inside that footprint, before rotating.
    pub art_x_mm: f64,
    pub art_y_mm: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NestResult {
    pub page_w: f64,
    pub page_h: f64,
    pub area_x: f64,
    pub area_y: f64,
    pub area_w: f64,
    pub area_h: f64,
    pub pages: Vec<Vec<Placed>>,
    pub unplaced: Vec<String>,
    /// Silhouette area actually stuck on the paper, in mm².
    pub ink_mm2: f64,
}

fn page_size(settings: &NestSettings) -> (f64, f64) {
    let (w, h) = match settings.paper.as_str() {
        "letter" => (215.9, 279.4),
        "custom" => (
            settings.custom_width_mm.clamp(20.0, 2000.0),
            settings.custom_height_mm.clamp(20.0, 2000.0),
        ),
        _ => (210.0, 297.0),
    };
    if settings.orientation == "landscape" {
        (h, w)
    } else {
        (w, h)
    }
}

/// Half the gap around every piece, plus the room the cut line needs. Two
/// silhouettes end up exactly `gap` apart and their outlines never touch.
fn clearance(settings: &NestSettings) -> f64 {
    let gap = settings.gap_mm.clamp(0.0, 30.0) / 2.0;
    if settings.cut_lines {
        gap + settings.outline_pad_mm.clamp(0.0, 30.0)
            + settings.outline_width_mm.clamp(0.05, 5.0) / 2.0
    } else {
        gap
    }
}

/// One copy ready to be placed: its artwork box and silhouette in millimetres.
pub struct Piece {
    pub id: String,
    pub art_w: f64,
    pub art_h: f64,
    /// Outline in millimetres from the top-left of the artwork box, y down.
    pub polys: Polygons,
    /// Preferred angle, in degrees clockwise.
    pub rotation: f64,
}

struct Mask {
    w: usize,
    h: usize,
    rows: Vec<Vec<u64>>,
}

struct AngleMask {
    angle: f64,
    mask: Mask,
    /// Where the mask grid starts inside the footprint, in cells.
    ox: i64,
    oy: i64,
    /// Footprint (art box plus cut line), in cells.
    uw: i64,
    uh: i64,
    /// Artwork box inside the footprint, in millimetres.
    art_x: f64,
    art_y: f64,
}

struct Prepared {
    id: String,
    ink: f64,
    masks: Vec<AngleMask>,
    /// The copy carries a turn chosen by hand, so `masks[0]` is not a choice
    /// the nester may trade for a tighter fit.
    pinned: bool,
}

fn rotate(point: (f64, f64), around: (f64, f64), degrees: f64) -> (f64, f64) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (dx, dy) = (point.0 - around.0, point.1 - around.1);
    (
        around.0 + dx * cos - dy * sin,
        around.1 + dx * sin + dy * cos,
    )
}

fn bbox(points: impl Iterator<Item = (f64, f64)>) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for (x, y) in points {
        out = Some(match out {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    out
}

/// Fills the polygons on a grid of `RES` millimetres whose cell (0, 0) sits at
/// (`min_x`, `min_y`). Even-odd rule: overlapping parts of one sticker are not
/// holes, but nothing here relies on it either way.
fn rasterize(
    polys: &[Vec<(f64, f64)>],
    min_x: f64,
    min_y: f64,
    w: usize,
    h: usize,
) -> Vec<bool> {
    let mut grid = vec![false; w * h];
    let mut crossings: Vec<f64> = Vec::new();
    for row in 0..h {
        let y = min_y + (row as f64 + 0.5) * RES;
        crossings.clear();
        for poly in polys {
            if poly.len() < 3 {
                continue;
            }
            for i in 0..poly.len() {
                let (x1, y1) = poly[i];
                let (x2, y2) = poly[(i + 1) % poly.len()];
                if (y1 <= y) == (y2 <= y) {
                    continue;
                }
                crossings.push(x1 + (y - y1) * (x2 - x1) / (y2 - y1));
            }
        }
        if crossings.is_empty() {
            continue;
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            let start = ((pair[0] - min_x) / RES).ceil() as i64;
            let end = ((pair[1] - min_x) / RES).floor() as i64;
            for cell in start.max(0)..=end.min(w as i64 - 1) {
                grid[row * w + cell as usize] = true;
            }
        }
    }
    grid
}

/// Grows the filled cells outwards by `radius` millimetres (chamfer distance),
/// which is how a sticker carries its gap and its cut padding with it.
fn dilate(grid: &mut [bool], w: usize, h: usize, radius: f64) {
    if radius <= 0.0 || w == 0 || h == 0 {
        return;
    }
    const NEIGHBOUR: f64 = 1.0;
    const DIAGONAL: f64 = std::f64::consts::SQRT_2;
    let mut dist = vec![0.0f64; w * h];
    for (i, &filled) in grid.iter().enumerate() {
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
    for i in 0..w * h {
        if dist[i] <= radius {
            grid[i] = true;
        }
    }
}

fn mask_from(grid: &[bool], w: usize, h: usize) -> Mask {
    let words = w.div_ceil(64).max(1);
    let mut rows = vec![vec![0u64; words]; h];
    for y in 0..h {
        for x in 0..w {
            if grid[y * w + x] {
                rows[y][x / 64] |= 1u64 << (x % 64);
            }
        }
    }
    Mask { w, h, rows }
}

impl Prepared {
    fn new(piece: &Piece, radius: f64) -> Prepared {
        let half = (piece.art_w / 2.0, piece.art_h / 2.0);
        let mut masks = Vec::with_capacity(ANGLES.len());
        let mut ink = 0.0;
        // The turn the copy already has comes first; the rest follow in 45°
        // steps, so a piece without a chosen turn still nests freely.
        let turned = (piece.rotation / 45.0).fract().abs() < 0.01;
        let start = if turned {
            ((piece.rotation / 45.0).round() as i64).rem_euclid(8) as usize
        } else {
            0
        };
        let pinned = turned && piece.rotation.rem_euclid(360.0) != 0.0;
        let angles: Vec<f64> = (0..ANGLES.len())
            .map(|step| ANGLES[(start + step) % ANGLES.len()])
            .collect();
        for (index, angle) in angles.iter().enumerate() {
            // Everything is measured from the artwork centre, so a turn never
            // moves the piece and the footprint stays around the origin.
            let rotated: Vec<Vec<(f64, f64)>> = piece
                .polys
                .iter()
                .map(|poly| {
                    poly.iter()
                        .map(|&(px, py)| {
                            rotate(
                                (px - half.0, py - half.1),
                                (0.0, 0.0),
                                *angle,
                            )
                        })
                        .collect()
                })
                .collect();
            let Some(shape_box) = bbox(rotated.iter().flatten().copied()) else {
                continue;
            };
            // The art box itself, turned: the footprint never depends on which
            // silhouette happened to be traced.
            let corners = [
                (-half.0, -half.1),
                (half.0, -half.1),
                (half.0, half.1),
                (-half.0, half.1),
            ];
            let Some(art_box) = bbox(
                corners
                    .iter()
                    .map(|&p| rotate(p, (0.0, 0.0), *angle)),
            ) else {
                continue;
            };

            // Grid over the silhouette plus its clearance, one cell of margin so
            // the dilation has room to grow.
            let min_x = (shape_box.0 - radius).floor() - 1.0;
            let min_y = (shape_box.1 - radius).floor() - 1.0;
            let w = ((shape_box.2 + radius).ceil() + 1.0 - min_x).max(1.0) as usize;
            let h = ((shape_box.3 + radius).ceil() + 1.0 - min_y).max(1.0) as usize;
            let mut grid = rasterize(&rotated, min_x, min_y, w, h);
            let ink_cells = grid.iter().filter(|&&filled| filled).count();
            if index == 0 {
                ink = ink_cells as f64 * RES * RES;
            }
            dilate(&mut grid, w, h, radius);

            // Cropped to its own ink, so no empty border is dragged along and
            // the mask never starts before the footprint it belongs to.
            let (mut c0, mut r0) = (w, h);
            let (mut c1, mut r1) = (0usize, 0usize);
            let mut any = false;
            for row in 0..h {
                for col in 0..w {
                    if !grid[row * w + col] {
                        continue;
                    }
                    any = true;
                    c0 = c0.min(col);
                    c1 = c1.max(col);
                    r0 = r0.min(row);
                    r1 = r1.max(row);
                }
            }
            if !any {
                continue;
            }
            let (mw, mh) = (c1 - c0 + 1, r1 - r0 + 1);
            let mut cropped = vec![false; mw * mh];
            for row in 0..mh {
                for col in 0..mw {
                    cropped[row * mw + col] = grid[(row + r0) * w + col + c0];
                }
            }

            let mask_x0 = min_x + c0 as f64 * RES;
            let mask_y0 = min_y + r0 as f64 * RES;
            let mask_x1 = mask_x0 + mw as f64 * RES;
            let mask_y1 = mask_y0 + mh as f64 * RES;

            // Footprint: whatever the artwork or its cut line reaches.
            let ux0 = art_box.0.min(mask_x0);
            let uy0 = art_box.1.min(mask_y0);
            let ux1 = art_box.2.max(mask_x1);
            let uy1 = art_box.3.max(mask_y1);
            let uw = ((ux1 - ux0) / RES).ceil().max(1.0) as i64;
            let uh = ((uy1 - uy0) / RES).ceil().max(1.0) as i64;

            masks.push(AngleMask {
                angle: *angle,
                mask: mask_from(&cropped, mw, mh),
                ox: ((mask_x0 - ux0) / RES).round() as i64,
                oy: ((mask_y0 - uy0) / RES).round() as i64,
                uw,
                uh,
                art_x: -ux0 - piece.art_w / 2.0,
                art_y: -uy0 - piece.art_h / 2.0,
            });
        }
        Prepared {
            id: piece.id.clone(),
            ink,
            masks,
            pinned,
        }
    }
}

struct Page {
    rows: Vec<Vec<u64>>,
    w_bits: usize,
}

fn overlaps(page_row: &[u64], mask_row: &[u64], shift: usize) -> bool {
    let word = shift / 64;
    let bits = shift % 64;
    for (i, &value) in mask_row.iter().enumerate() {
        if value == 0 {
            continue;
        }
        let at = word + i;
        if at >= page_row.len() {
            return true;
        }
        if page_row[at] & (value << bits) != 0 {
            return true;
        }
        if bits > 0 {
            let carry = if bits == 0 { 0 } else { value >> (64 - bits) };
            if carry != 0 && (at + 1 >= page_row.len() || page_row[at + 1] & carry != 0) {
                return true;
            }
        }
    }
    false
}

fn paint(page_row: &mut [u64], mask_row: &[u64], shift: usize) {
    let word = shift / 64;
    let bits = shift % 64;
    for (i, &value) in mask_row.iter().enumerate() {
        if value == 0 {
            continue;
        }
        let at = word + i;
        if at < page_row.len() {
            page_row[at] |= value << bits;
        }
        if bits > 0 {
            let carry = if bits == 0 { 0 } else { value >> (64 - bits) };
            if carry != 0 && at + 1 < page_row.len() {
                page_row[at + 1] |= carry;
            }
        }
    }
}

impl Page {
    fn new(w_bits: usize, h_bits: usize) -> Page {
        let words = w_bits.div_ceil(64).max(1);
        Page {
            rows: vec![vec![0u64; words]; h_bits],
            w_bits,
        }
    }

    fn fits(&self, mask: &AngleMask, x: i64, y: i64) -> bool {
        let gx = x + mask.ox;
        let gy = y + mask.oy;
        if gx < 0 || gy < 0 {
            return false;
        }
        let (gx, gy) = (gx as usize, gy as usize);
        if gx + mask.mask.w > self.w_bits || gy + mask.mask.h > self.rows.len() {
            return false;
        }
        (0..mask.mask.h).all(|row| !overlaps(&self.rows[gy + row], &mask.mask.rows[row], gx))
    }

    fn paint(&mut self, mask: &AngleMask, x: i64, y: i64) {
        let gx = (x + mask.ox).max(0) as usize;
        let gy = (y + mask.oy).max(0) as usize;
        for row in 0..mask.mask.h {
            paint(&mut self.rows[gy + row], &mask.mask.rows[row], gx);
        }
    }
}

/// The same sheet laid out three times, once per order of the copies, and the
/// run that needs the fewest pages wins. Trying them costs a few hundred
/// milliseconds and plainly pays for itself on a full sheet.
pub fn pack(
    pieces: &[Piece],
    page_w: f64,
    page_h: f64,
    margin: f64,
    radius: f64,
) -> (Vec<Vec<Placed>>, Vec<String>, f64) {
    let mut best: Option<(Vec<Vec<Placed>>, Vec<String>, f64)> = None;
    for mode in 0..3 {
        let run = pack_in_order(pieces, page_w, page_h, margin, radius, mode);
        let fewer_pages = best.as_ref().is_none_or(|(pages, _, _)| run.0.len() < pages.len());
        if fewer_pages {
            best = Some(run);
        }
    }
    best.unwrap_or_default()
}

/// Bottom-left-fill sobre siluetas: cada copia va a la esquina de lo ya
/// colocado que quede más abajo, y a la izquierda dentro de esa altura.
fn pack_in_order(
    pieces: &[Piece],
    page_w: f64,
    page_h: f64,
    margin: f64,
    radius: f64,
    mode: u8,
) -> (Vec<Vec<Placed>>, Vec<String>, f64) {
    let w_bits = (page_w / RES).floor().max(1.0) as usize;
    let h_bits = (page_h / RES).floor().max(1.0) as usize;
    let margin_cells = (margin / RES).round().max(0.0) as i64;

    // Nothing larger than the printable area can be placed, and its mask would
    // be built only to be thrown away.
    let usable_w = page_w - margin * 2.0 + 1.0;
    let usable_h = page_h - margin * 2.0 + 1.0;
    let mut oversized: Vec<String> = Vec::new();
    let mut prepared: Vec<Prepared> = pieces
        .iter()
        .filter(|piece| {
            if piece.art_w <= 0.0 || piece.art_h <= 0.0 || piece.polys.is_empty() {
                return false;
            }
            let fits = (piece.art_w <= usable_w && piece.art_h <= usable_h)
                || (piece.art_h <= usable_w && piece.art_w <= usable_h);
            if !fits {
                oversized.push(piece.id.clone());
            }
            fits
        })
        .map(|piece| Prepared::new(piece, radius))
        .collect();
    prepared.retain(|piece| !piece.masks.is_empty());
    // Largest first, by ink, by longest side or by footprint: the awkward
    // pieces get the open paper, and which measure is "largest" is not obvious.
    prepared.sort_by(|a, b| match mode {
        0 => b.ink.total_cmp(&a.ink),
        1 => side(b).total_cmp(&side(a)),
        _ => bulk(b).total_cmp(&bulk(a)),
    });

    let mut pages: Vec<Page> = Vec::new();
    let mut placed: Vec<Vec<Placed>> = Vec::new();
    let mut spots: Vec<Vec<(i64, i64)>> = Vec::new();
    let mut unplaced: Vec<String> = oversized;
    let mut ink = 0.0;

    for piece in &prepared {
        // A turn chosen by hand is a demand, not a hint: it is kept whenever
        // the copy finds a spot at that angle. 0 means "turn it as you like".
        let pinned = piece.pinned.then_some(0);
        let mut done = false;
        for page_index in 0..pages.len() {
            // Copied out: placing a piece adds candidates to this same list.
            let candidates = spots[page_index].clone();
            let Some((mask_index, x, y)) =
                best_spot(piece, &pages[page_index], &candidates, pinned)
            else {
                continue;
            };
            commit(
                piece,
                mask_index,
                x,
                y,
                &mut pages[page_index],
                &mut placed[page_index],
                &mut spots[page_index],
            );
            ink += piece.ink;
            done = true;
            break;
        }
        if done {
            continue;
        }

        // New page: the copy goes where it fits an empty sheet, or nowhere.
        let mut page = Page::new(w_bits, h_bits);
        let start = (margin_cells, margin_cells);
        let Some(mask_index) = (0..piece.masks.len())
            .find(|&index| page.fits(&piece.masks[index], start.0, start.1))
        else {
            unplaced.push(piece.id.clone());
            continue;
        };
        let mut page_placed: Vec<Placed> = Vec::new();
        let mut list = Vec::new();
        commit(
            piece,
            mask_index,
            start.0,
            start.1,
            &mut page,
            &mut page_placed,
            &mut list,
        );
        ink += piece.ink;
        placed.push(page_placed);
        pages.push(page);
        spots.push(list);
    }

    // Pages are handed back in the order they were opened, empty ones dropped.
    let mut out: Vec<Vec<Placed>> = Vec::new();
    for page in placed {
        if !page.is_empty() {
            out.push(page);
        }
    }
    (out, unplaced, ink)
}

/// Paints one mask on a page and records the placement, so the first copy of a
/// page and the rest go through exactly the same steps.
fn commit(
    piece: &Prepared,
    mask_index: usize,
    x: i64,
    y: i64,
    page: &mut Page,
    placed: &mut Vec<Placed>,
    spots: &mut Vec<(i64, i64)>,
) {
    let mask = &piece.masks[mask_index];
    page.paint(mask, x, y);
    spots.push((x + mask.uw, y));
    spots.push((x, y + mask.uh));
    spots.push((x + mask.uw, y + mask.uh));
    dedupe(spots);
    placed.push(Placed {
        copy_id: piece.id.clone(),
        x_mm: x as f64 * RES,
        y_mm: y as f64 * RES,
        width_mm: mask.uw as f64 * RES,
        height_mm: mask.uh as f64 * RES,
        rotation: mask.angle,
        art_x_mm: mask.art_x,
        art_y_mm: mask.art_y,
    });
}

/// Lowest spot the piece can take on this page, leftmost among equals. With a
/// chosen turn only that angle is tried; without one, all eight compete.
fn best_spot(
    piece: &Prepared,
    page: &Page,
    candidates: &[(i64, i64)],
    pinned: Option<usize>,
) -> Option<(usize, i64, i64)> {
    if let Some(index) = pinned {
        let mask = &piece.masks[index];
        let mut best: Option<(i64, i64)> = None;
        for &(x, y) in candidates {
            if let Some((_, by)) = best {
                if y + mask.uh > by + mask.uh {
                    continue;
                }
            }
            if page.fits(mask, x, y) {
                let better = match best {
                    None => true,
                    Some((bx, by)) => (y + mask.uh, x) < (by + mask.uh, bx),
                };
                if better {
                    best = Some((x, y));
                }
            }
        }
        return best.map(|(x, y)| (index, x, y));
    }

    let min_uh = piece.masks.iter().map(|mask| mask.uh).min().unwrap_or(1);
    let mut hit: Option<(usize, i64, i64)> = None;
    for &(x, y) in candidates {
        if let Some((best, _, by)) = hit {
            if y + min_uh > by + piece.masks[best].uh {
                continue;
            }
        }
        for (mask_index, mask) in piece.masks.iter().enumerate() {
            let top = y + mask.uh;
            let better = match hit {
                None => true,
                Some((best, bx, by)) => {
                    top < by + piece.masks[best].uh
                        || (top == by + piece.masks[best].uh && x < bx)
                }
            };
            if better && page.fits(mask, x, y) {
                hit = Some((mask_index, x, y));
            }
        }
    }
    hit
}

fn side(piece: &Prepared) -> f64 {
    piece
        .masks
        .iter()
        .map(|mask| mask.uw.max(mask.uh) as f64)
        .fold(0.0, f64::max)
}

fn bulk(piece: &Prepared) -> f64 {
    piece
        .masks
        .iter()
        .map(|mask| (mask.uw * mask.uh) as f64)
        .fold(0.0, f64::max)
}

fn dedupe(spots: &mut Vec<(i64, i64)>) {
    let mut seen = HashSet::new();
    spots.retain(|spot| seen.insert(*spot));
    spots.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
}

/// A traced outline can carry tens of thousands of points from the pixel
/// staircase of a preview. One mask cell is 1 mm, so a third of a millimetre
/// between points is already finer than anything the nester can use.
fn decimate(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    const STEP: f64 = 0.3;
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len() / 8 + 4);
    for point in points {
        if let Some(last) = out.last() {
            let (dx, dy) = (point.0 - last.0, point.1 - last.1);
            if dx * dx + dy * dy < STEP * STEP {
                continue;
            }
        }
        out.push(point);
    }
    if out.len() < 3 {
        return Vec::new();
    }
    out
}

/// One silhouette: the outlines traced for it, in millimetres.
type Polygons = Vec<Vec<(f64, f64)>>;
type ShapeMap = HashMap<String, Polygons>;
static SHAPES: std::sync::OnceLock<std::sync::Mutex<ShapeMap>> = std::sync::OnceLock::new();

fn shape_cache() -> &'static std::sync::Mutex<ShapeMap> {
    SHAPES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Canvas side used when tracing for the nester. One millimetre of a sticker
/// is a few pixels here, which is all a 1 mm mask can tell apart.
const NEST_TRACE_SIDE: u32 = 320;
const MAX_CACHED_SHAPES: usize = 6000;

/// Traces the silhouette of every distinct file and size once, in parallel.
fn shapes_for(copies: &[NestCopy], padding: f64) -> ShapeMap {
    let mut wanted: Vec<(String, String, f64, f64)> = Vec::new();
    let mut seen = HashSet::new();
    for copy in copies {
        let key = format!(
            "{}|{:.2}x{:.2}|{padding:.2}",
            copy.path.to_ascii_lowercase(),
            copy.width_mm,
            copy.height_mm
        );
        if seen.insert(key.clone()) {
            wanted.push((key, copy.path.clone(), copy.width_mm, copy.height_mm));
        }
    }

    let mut shapes: ShapeMap = HashMap::new();
    let mut missing: Vec<(String, String, f64, f64)> = Vec::new();
    {
        let cache = shape_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for item in wanted {
            match cache.get(&item.0) {
                Some(shape) => {
                    shapes.insert(item.0.clone(), shape.clone());
                }
                None => missing.push(item),
            }
        }
    }
    if missing.is_empty() {
        return shapes;
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk = missing.len().div_ceil(workers).max(1);
    let traced: Vec<(String, Polygons)> = std::thread::scope(|scope| {
        let handles: Vec<_> = missing
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|(key, path, width, height)| {
                            let shape = contour::shape_from_file_at(
                                Path::new(path),
                                *width,
                                *height,
                                padding,
                                NEST_TRACE_SIDE,
                            )
                            .unwrap_or_else(|_| contour::Shape::rectangle());
                            let scaled = shape
                                .contours
                                .into_iter()
                                .map(|poly| {
                                    decimate(
                                        poly.into_iter()
                                            .map(|(nx, ny)| (nx * width, ny * height))
                                            .collect(),
                                    )
                                })
                                .filter(|poly| !poly.is_empty())
                                .collect();
                            (key.clone(), scaled)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });

    {
        let mut cache = shape_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() + traced.len() > MAX_CACHED_SHAPES {
            cache.clear();
        }
        for (key, shape) in &traced {
            cache.insert(key.clone(), shape.clone());
        }
    }
    for (key, shape) in traced {
        shapes.insert(key, shape);
    }
    shapes
}

pub fn nest(copies: &[NestCopy], settings: &NestSettings) -> Result<NestResult, String> {
    if copies.is_empty() {
        let (page_w, page_h) = page_size(settings);
        let margin = settings.margin_mm.clamp(0.0, 100.0);
        return Ok(NestResult {
            page_w,
            page_h,
            area_x: margin,
            area_y: margin,
            area_w: (page_w - margin * 2.0).max(0.0),
            area_h: (page_h - margin * 2.0).max(0.0),
            pages: Vec::new(),
            unplaced: Vec::new(),
            ink_mm2: 0.0,
        });
    }

    let (page_w, page_h) = page_size(settings);
    let margin = settings.margin_mm.clamp(0.0, 100.0);
    let padding = if settings.cut_lines {
        settings.outline_pad_mm.clamp(0.0, 30.0)
    } else {
        0.0
    };
    let shapes = shapes_for(copies, padding);

    let pieces: Vec<Piece> = copies
        .iter()
        .filter(|copy| copy.width_mm > 0.0 && copy.height_mm > 0.0 && !copy.path.is_empty())
        .map(|copy| {
            let key = format!(
                "{}|{:.2}x{:.2}|{padding:.2}",
                copy.path.to_ascii_lowercase(),
                copy.width_mm,
                copy.height_mm
            );
            Piece {
                id: copy.id.clone(),
                art_w: copy.width_mm,
                art_h: copy.height_mm,
                polys: shapes.get(&key).cloned().unwrap_or_else(|| {
                    let (w, h) = (copy.width_mm, copy.height_mm);
                    vec![vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]]
                }),
                rotation: copy.rotation,
            }
        })
        .collect();

    let (pages, unplaced, ink_mm2) =
        pack(&pieces, page_w, page_h, margin, clearance(settings));

    Ok(NestResult {
        page_w,
        page_h,
        area_x: margin,
        area_y: margin,
        area_w: (page_w - margin * 2.0).max(0.0),
        area_h: (page_h - margin * 2.0).max(0.0),
        pages,
        unplaced,
        ink_mm2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(id: &str, size: f64) -> Piece {
        Piece {
            id: id.into(),
            art_w: size,
            art_h: size,
            polys: vec![vec![(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)]],
            rotation: 0.0,
        }
    }

    fn landscape(width: f64, height: f64) -> Piece {
        Piece {
            id: "wide".into(),
            art_w: width,
            art_h: height,
            polys: vec![vec![(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]],
            rotation: 0.0,
        }
    }

    #[test]
    fn two_pieces_share_the_page_and_never_overlap() {
        let pieces = [square("a", 40.0), square("b", 40.0), square("c", 40.0)];
        let (pages, unplaced, ink) = pack(&pieces, 210.0, 297.0, 8.0, 1.0);
        assert_eq!(pages.len(), 1);
        assert!(unplaced.is_empty());
        assert!((ink - 4800.0).abs() < 200.0, "ink = {ink}");

        let placed = &pages[0];
        assert_eq!(placed.len(), 3);
        for (i, a) in placed.iter().enumerate() {
            for b in placed.iter().skip(i + 1) {
                let apart = a.x_mm + a.width_mm <= b.x_mm
                    || b.x_mm + b.width_mm <= a.x_mm
                    || a.y_mm + a.height_mm <= b.y_mm
                    || b.y_mm + b.height_mm <= a.y_mm;
                assert!(apart, "footprints overlap: {:?} / {:?}", a, b);
            }
        }
    }

    #[test]
    fn the_gap_is_kept_between_silhouettes() {
        // Two 20 mm squares with 4 mm of clearance each: their masks may not
        // come closer than 8 mm, even if their footprints are 20 mm apart.
        let pieces = [square("a", 20.0), square("b", 20.0)];
        let (pages, _, _) = pack(&pieces, 120.0, 60.0, 0.0, 4.0);
        assert_eq!(pages.len(), 1);
        let placed = &pages[0];
        let (a, b) = (placed[0].clone(), placed[1].clone());
        // Footprints already carry the clearance, so what must be far apart is
        // the artwork itself: 8 mm, minus the 1 mm grid.
        let (ax, ay) = (a.x_mm + a.art_x_mm, a.y_mm + a.art_y_mm);
        let (bx, by) = (b.x_mm + b.art_x_mm, b.y_mm + b.art_y_mm);
        let gap_x = (ax - (bx + 20.0)).max(bx - (ax + 20.0));
        let gap_y = (ay - (by + 20.0)).max(by - (ay + 20.0));
        assert!(
            gap_x.max(gap_y) >= 7.0,
            "artwork boxes are {gap_x}/{gap_y} mm apart"
        );
    }

    #[test]
    fn a_long_piece_is_turned_to_make_it_fit() {
        // 90 × 40 does not fit the 80 mm width, a quarter turn does.
        let pieces = [landscape(90.0, 40.0)];
        let (pages, unplaced, _) = pack(&pieces, 80.0, 120.0, 0.0, 0.0);
        assert!(unplaced.is_empty());
        assert_eq!(pages.len(), 1);
        let placed = &pages[0][0];
        assert_eq!(placed.rotation, 90.0);
        assert!(placed.width_mm <= 80.0 && placed.height_mm <= 120.0);
    }

    #[test]
    fn a_copy_keeps_the_turn_it_was_given() {
        // The turn chosen by hand is tried first, so the rotate button and a
        // reopened sheet show the same angle.
        let mut piece = landscape(90.0, 40.0);
        piece.rotation = 45.0;
        let (pages, unplaced, _) = pack(&[piece], 200.0, 200.0, 0.0, 0.0);
        assert!(unplaced.is_empty());
        assert_eq!(pages[0][0].rotation, 45.0);
    }

    #[test]
    fn a_chosen_turn_survives_a_shared_page() {
        // Both fit one page; the turned copy must not be straightened out just
        // because upright leaves less air above it.
        let mut turned = landscape(60.0, 30.0);
        turned.id = "turned".into();
        turned.rotation = 45.0;
        let plain = square("plain", 60.0);
        let (pages, unplaced, _) = pack(&[plain, turned], 120.0, 120.0, 0.0, 2.0);
        assert!(unplaced.is_empty());
        let placed = pages
            .iter()
            .flatten()
            .find(|item| item.copy_id == "turned")
            .expect("the turned copy is placed");
        assert_eq!(placed.rotation, 45.0);
    }

    #[test]
    fn something_larger_than_the_page_is_reported() {
        let pieces = [square("huge", 300.0)];
        let (pages, unplaced, _) = pack(&pieces, 210.0, 297.0, 8.0, 0.0);
        assert!(pages.is_empty());
        assert_eq!(unplaced, vec!["huge".to_string()]);
    }

    /// Cost of a full sheet: 977 copies of sticker-sized blobs. Run with
    /// `cargo test --release --lib nest::tests::timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_a_big_sheet() {
        let mut pieces = Vec::new();
        for i in 0..977usize {
            let w = 32.0 + (i % 7) as f64 * 3.0;
            let h = 26.0 + (i % 5) as f64 * 4.0;
            let points = 36;
            let polys = vec![(0..points)
                .map(|k| {
                    let angle = k as f64 / points as f64 * std::f64::consts::TAU;
                    (
                        w / 2.0 + 0.34 * w * angle.cos(),
                        h / 2.0 + 0.34 * h * angle.sin(),
                    )
                })
                .collect()];
            pieces.push(Piece {
                id: format!("c{i}"),
                art_w: w,
                art_h: h,
                polys,
                rotation: 0.0,
            });
        }
        let start = std::time::Instant::now();
        let (pages, unplaced, ink) = pack(&pieces, 210.0, 297.0, 4.0, 3.7);
        println!(
            "977 copias: {} páginas, {} sin sitio, {:.1} cm² de tinta, {} ms",
            pages.len(),
            unplaced.len(),
            ink / 100.0,
            start.elapsed().as_millis()
        );
    }

    #[test]
    fn the_artwork_box_sits_inside_the_footprint() {
        let pieces = [square("a", 40.0)];
        let (pages, _, _) = pack(&pieces, 210.0, 297.0, 8.0, 2.0);
        let placed = &pages[0][0];
        // 2 mm of clearance on both sides: the footprint is the art plus 4 mm.
        assert!((placed.art_x_mm - 2.0).abs() < 1.5, "art_x = {}", placed.art_x_mm);
        assert!((placed.art_y_mm - 2.0).abs() < 1.5, "art_y = {}", placed.art_y_mm);
        assert!(placed.width_mm >= 40.0);
    }
}
