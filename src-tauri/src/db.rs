use crate::importer;
use rusqlite::backup::Backup;
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sticker {
    pub id: String,
    pub name: String,
    pub ext: String,
    /// "raster" o "vector"
    pub kind: String,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: f64,
    pub has_alpha: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub file_name: String,
    pub file_path: String,
    pub thumb_path: String,
    pub tags: Vec<String>,
    /// The work or franchise in one canonical spelling ("" = not classified yet).
    pub serie: String,
    /// One of the closed list of families in `ai::TIPOS` ("" = not classified yet).
    pub tipo: String,
    /// When the AI last named and tagged this sticker (0 = never).
    pub ai_at: i64,
    /// When the text classifier looked at this sticker (0 = never).
    pub classified_at: i64,
}

/// What the AI needs to work on one sticker.
pub struct AiSource {
    pub id: String,
    pub path: std::path::PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS stickers (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            file_name   TEXT NOT NULL,
            ext         TEXT NOT NULL,
            kind        TEXT NOT NULL,
            width_px    REAL NOT NULL DEFAULT 0,
            height_px   REAL NOT NULL DEFAULT 0,
            width_mm    REAL NOT NULL,
            height_mm   REAL NOT NULL,
            dpi         REAL NOT NULL,
            has_alpha   INTEGER NOT NULL DEFAULT 0,
            size_bytes  INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            hash        TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS sticker_tags (
            sticker_id TEXT NOT NULL,
            tag        TEXT NOT NULL,
            PRIMARY KEY (sticker_id, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_sticker_tags_tag ON sticker_tags(tag);
        CREATE INDEX IF NOT EXISTS idx_stickers_created ON stickers(created_at DESC);
        -- Sheets saved inside the app: the layout lives here, not in a file.
        CREATE TABLE IF NOT EXISTS sheets (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            payload    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sheets_updated ON sheets(updated_at DESC);
        "#,
    )?;
    // Libraries created before the AI and classification columns.
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(stickers)")?
        .query_map([], |r| r.get::<_, String>(1))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    for (name, ddl) in [
        ("ai_at", "ALTER TABLE stickers ADD COLUMN ai_at INTEGER NOT NULL DEFAULT 0;"),
        ("serie", "ALTER TABLE stickers ADD COLUMN serie TEXT NOT NULL DEFAULT '';"),
        ("tipo", "ALTER TABLE stickers ADD COLUMN tipo TEXT NOT NULL DEFAULT '';"),
        (
            "classified_at",
            "ALTER TABLE stickers ADD COLUMN classified_at INTEGER NOT NULL DEFAULT 0;",
        ),
        // Filled from now on: rows from older libraries keep an empty hash and
        // are matched by size and pixel dimensions instead.
        ("hash", "ALTER TABLE stickers ADD COLUMN hash TEXT NOT NULL DEFAULT '';"),
    ] {
        if !columns.iter().any(|c| c == name) {
            conn.execute_batch(ddl)?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_stickers_serie ON stickers(serie);
         CREATE INDEX IF NOT EXISTS idx_stickers_tipo ON stickers(tipo);",
    )?;
    Ok(())
}

// --- copies of the library -------------------------------------------------

/// How many daily copies are kept before the oldest one is dropped.
const BACKUP_KEEP: usize = 5;

/// Civil date from a day count, so the file names need no date library.
/// Algorithm from Howard Hinnant's `chrono`-compatible `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `PRAGMA integrity_check`: the word `ok` when the file is sound, or the
/// first problem SQLite found.
pub fn check_integrity(conn: &Connection) -> Result<String, String> {
    conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())
}

/// Copies the database into `backups/library-YYYY-MM-DD.db`, once a day, and
/// keeps the newest few. Uses SQLite's backup API, so pending WAL writes are
/// included and the copy is never taken mid-transaction.
pub fn backup(root: &Path, conn: &Connection) -> Result<Option<PathBuf>, String> {
    let dir = root.join("backups");
    fs::create_dir_all(&dir).map_err(|e| format!("No se pudo crear la carpeta de copias: {e}"))?;
    let dest = dir.join(format!("library-{}.db", today_utc()));
    if dest.exists() {
        return Ok(None);
    }

    let tmp = dir.join("library.tmp.db");
    let _ = fs::remove_file(&tmp);
    {
        let mut out =
            Connection::open(&tmp).map_err(|e| format!("No se pudo preparar la copia: {e}"))?;
        let copy = Backup::new(conn, &mut out)
            .map_err(|e| format!("No se pudo abrir la copia: {e}"))?;
        copy.run_to_completion(256, Duration::from_millis(0), None)
            .map_err(|e| format!("No se pudo copiar la base de datos: {e}"))?;
    }
    fs::rename(&tmp, &dest).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("No se pudo guardar la copia: {e}")
    })?;
    prune_backups(&dir, BACKUP_KEEP);
    Ok(Some(dest))
}

/// Drops the oldest copies; the date in the name sorts chronologically.
fn prune_backups(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| name.starts_with("library-") && name.ends_with(".db"))
        .collect();
    names.sort();
    let extra = names.len().saturating_sub(keep);
    for name in names.into_iter().take(extra) {
        let _ = fs::remove_file(dir.join(name));
    }
}

// --- density of the artwork already in the library --------------------------

/// One raster sticker as the density check needs it.
pub struct DensityRow {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub ext: String,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: f64,
}

/// A sticker whose density, and possibly its size, has to be rewritten.
pub struct DensityFix {
    pub id: String,
    pub dpi: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpiReport {
    /// Raster stickers looked at.
    pub checked: usize,
    /// Stickers whose size followed the old density and was recomputed.
    pub updated: usize,
    /// Hand-sized stickers: their millimetres stay, only the density moves.
    pub kept_size: usize,
    /// The file declares the density already stored.
    pub unchanged: usize,
    /// No density in the file (WebP, or PNG/JPEG without metadata).
    pub without_density: usize,
    pub failures: Vec<DpiFailure>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpiFailure {
    pub name: String,
    pub message: String,
}

pub struct DensityPlan {
    pub report: DpiReport,
    pub fixes: Vec<DensityFix>,
}

/// Every raster sticker, oldest first.
pub fn density_rows(conn: &Connection) -> Result<Vec<DensityRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, file_name, ext, width_px, height_px, width_mm, height_mm, dpi
             FROM stickers WHERE kind = 'raster' ORDER BY created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DensityRow {
                id: r.get(0)?,
                name: r.get(1)?,
                file_name: r.get(2)?,
                ext: r.get(3)?,
                width_px: r.get(4)?,
                height_px: r.get(5)?,
                width_mm: r.get(6)?,
                height_mm: r.get(7)?,
                dpi: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Reads the density each file declares and works out the new values. Nothing
/// is written here, so the files can be read off the UI thread.
pub fn plan_densities(rows: Vec<DensityRow>, files: &Path) -> DensityPlan {
    let mut plan = DensityPlan {
        report: DpiReport::default(),
        fixes: Vec::new(),
    };
    for row in rows {
        plan.report.checked += 1;
        let ext = row.ext.to_ascii_lowercase();
        let density = match importer::declared_density(&files.join(&row.file_name), &ext) {
            Ok(Some(value)) => value,
            Ok(None) => {
                plan.report.without_density += 1;
                continue;
            }
            Err(message) => {
                plan.report.failures.push(DpiFailure {
                    name: row.name,
                    message,
                });
                continue;
            }
        };
        if (density - row.dpi).abs() < 0.01 {
            plan.report.unchanged += 1;
            continue;
        }
        // A size typed in the panel is kept: only the density it was measured
        // against changes. A size that still follows the stored density came
        // from the pixels, so it is safe to recompute.
        let derived = row.dpi > 0.0
            && row.width_px > 0.0
            && row.height_px > 0.0
            && (row.width_mm - row.width_px * 25.4 / row.dpi).abs() < 0.05
            && (row.height_mm - row.height_px * 25.4 / row.dpi).abs() < 0.05;
        if derived {
            plan.report.updated += 1;
        } else {
            plan.report.kept_size += 1;
        }
        plan.fixes.push(DensityFix {
            id: row.id,
            dpi: density,
            width_mm: if derived {
                round_mm(row.width_px * 25.4 / density)
            } else {
                row.width_mm
            },
            height_mm: if derived {
                round_mm(row.height_px * 25.4 / density)
            } else {
                row.height_mm
            },
        });
    }
    plan
}

/// Writes every fix in one transaction, so a failure leaves the library as it was.
pub fn apply_densities(conn: &mut Connection, fixes: &[DensityFix]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for fix in fixes {
        tx.execute(
            "UPDATE stickers SET dpi = ?1, width_mm = ?2, height_mm = ?3 WHERE id = ?4",
            params![fix.dpi, fix.width_mm, fix.height_mm, fix.id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn round_mm(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn normalize_tag(raw: &str) -> Option<String> {
    let tag = raw.trim().to_lowercase().replace(',', " ");
    let tag = tag.split_whitespace().collect::<Vec<_>>().join(" ");
    if tag.is_empty() || tag.chars().count() > 40 {
        None
    } else {
        Some(tag)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct NewSticker<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub file_name: &'a str,
    pub ext: &'a str,
    pub kind: &'a str,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: f64,
    pub has_alpha: bool,
    pub size_bytes: i64,
    /// SHA-256 of the source file, so a re-import can be spotted.
    pub hash: &'a str,
}

pub fn insert(conn: &Connection, s: &NewSticker<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO stickers (id, name, file_name, ext, kind, width_px, height_px, width_mm, height_mm, dpi, has_alpha, size_bytes, created_at, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            s.id,
            s.name,
            s.file_name,
            s.ext,
            s.kind,
            s.width_px,
            s.height_px,
            s.width_mm,
            s.height_mm,
            s.dpi,
            if s.has_alpha { 1 } else { 0 },
            s.size_bytes,
            now_ms(),
            s.hash,
        ],
    )?;
    Ok(())
}

/// What an import needs to know about the library to spot a repeat.
pub struct KnownFile {
    pub hash: String,
    pub ext: String,
    pub size_bytes: i64,
}

pub fn known_files(conn: &Connection) -> Result<Vec<KnownFile>, String> {
    let mut stmt = conn
        .prepare("SELECT hash, ext, size_bytes FROM stickers")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(KnownFile {
                hash: r.get(0)?,
                ext: r.get(1)?,
                size_bytes: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

fn tags_map(conn: &Connection) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT sticker_id, tag FROM sticker_tags ORDER BY tag") else {
        return map;
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }) else {
        return map;
    };
    for row in rows.flatten() {
        map.entry(row.0).or_default().push(row.1);
    }
    map
}

pub fn list(
    conn: &Connection,
    root: &Path,
    query: &str,
    tags: &[String],
    sort: &str,
) -> Result<Vec<Sticker>, String> {
    let mut sql = String::from(
        "SELECT id, name, file_name, ext, kind, width_px, height_px, width_mm, height_mm, dpi, has_alpha, size_bytes, created_at, ai_at, serie, tipo, classified_at FROM stickers",
    );
    let mut clauses: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        // The fields the search box promises: name, tag, series and type.
        // Only lowercased here; folding accents and forgiving typos is the
        // in-memory filter's job.
        clauses.push(
            "(lower(name) LIKE ? OR lower(coalesce(serie, '')) LIKE ? \
             OR lower(coalesce(tipo, '')) LIKE ? \
             OR id IN (SELECT sticker_id FROM sticker_tags WHERE lower(tag) LIKE ?))"
                .into(),
        );
        let like = format!("%{}%", q);
        for _ in 0..4 {
            vals.push(Box::new(like.clone()));
        }
    }
    let clean_tags: Vec<String> = tags.iter().filter_map(|t| normalize_tag(t)).collect();
    if !clean_tags.is_empty() {
        let holes = vec!["?"; clean_tags.len()].join(",");
        clauses.push(format!(
            "id IN (SELECT sticker_id FROM sticker_tags WHERE tag IN ({holes}) GROUP BY sticker_id HAVING COUNT(DISTINCT tag) = ?)"
        ));
        for t in &clean_tags {
            vals.push(Box::new(t.clone()));
        }
        vals.push(Box::new(clean_tags.len() as i64));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    match sort {
        "name" => sql.push_str(" ORDER BY lower(name) ASC"),
        "size" => sql.push_str(" ORDER BY (width_mm * height_mm) DESC"),
        _ => sql.push_str(" ORDER BY created_at DESC"),
    }

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(vals), |r| {
            Ok(Sticker {
                id: r.get(0)?,
                name: r.get(1)?,
                ext: r.get(3)?,
                kind: r.get(4)?,
                width_px: r.get(5)?,
                height_px: r.get(6)?,
                width_mm: r.get(7)?,
                height_mm: r.get(8)?,
                dpi: r.get(9)?,
                has_alpha: r.get::<_, i64>(10)? != 0,
                size_bytes: r.get(11)?,
                created_at: r.get(12)?,
                ai_at: r.get(13)?,
                serie: r.get(14)?,
                tipo: r.get(15)?,
                classified_at: r.get(16)?,
                file_path: String::new(),
                thumb_path: String::new(),
                tags: Vec::new(),
                file_name: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut stickers: Vec<(String, Sticker)> = Vec::new();
    for row in rows {
        let s = row.map_err(|e| e.to_string())?;
        stickers.push((s.id.clone(), s));
    }

    let mut tags_map = tags_map(conn);
    let files = root.join("files");
    let thumbs = root.join("thumbs");

    Ok(stickers
        .into_iter()
        .map(|(id, mut s)| {
            s.tags = tags_map.remove(&id).unwrap_or_default();
            let file = files.join(&s.file_name);
            let thumb = thumbs.join(format!("{}.png", id));
            let thumb_jpg = thumbs.join(format!("{}.jpg", id));
            s.file_path = file.to_string_lossy().to_string();
            s.thumb_path = if thumb_jpg.exists() {
                thumb_jpg.to_string_lossy().to_string()
            } else if thumb.exists() {
                thumb.to_string_lossy().to_string()
            } else {
                s.file_path.clone()
            };
            s
        })
        .collect())
}

pub fn update_meta(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    width_mm: Option<f64>,
    height_mm: Option<f64>,
) -> Result<(), String> {
    if let Some(n) = name {
        let n = n.trim();
        if n.is_empty() {
            return Err("El nombre no puede estar vacío".into());
        }
        conn.execute("UPDATE stickers SET name = ?1 WHERE id = ?2", params![n, id])
            .map_err(|e| e.to_string())?;
    }
    if let Some(w) = width_mm {
        if !(1.0..=2000.0).contains(&w) {
            return Err("El ancho debe estar entre 1 y 2000 mm".into());
        }
        conn.execute("UPDATE stickers SET width_mm = ?1 WHERE id = ?2", params![w, id])
            .map_err(|e| e.to_string())?;
    }
    if let Some(h) = height_mm {
        if !(1.0..=2000.0).contains(&h) {
            return Err("El alto debe estar entre 1 y 2000 mm".into());
        }
        conn.execute("UPDATE stickers SET height_mm = ?1 WHERE id = ?2", params![h, id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn set_tags(conn: &Connection, id: &str, tags: &[String]) -> Result<Vec<String>, String> {
    let mut clean: Vec<String> = tags.iter().filter_map(|t| normalize_tag(t)).collect();
    clean.sort();
    clean.dedup();
    conn.execute("DELETE FROM sticker_tags WHERE sticker_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    for tag in &clean {
        conn.execute(
            "INSERT OR IGNORE INTO sticker_tags (sticker_id, tag) VALUES (?1, ?2)",
            params![id, tag],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(clean)
}

/// Existing ids with their names: what the archive import uses to spot the
/// stickers it already has.
pub fn id_names(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM stickers")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(|e| e.to_string())
}

/// A sticker as an archive carries it: every column plus its tags.
pub struct RestoredSticker {
    pub id: String,
    pub name: String,
    pub file_name: String,
    pub ext: String,
    pub kind: String,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: f64,
    pub has_alpha: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub ai_at: i64,
    pub serie: String,
    pub tipo: String,
    pub classified_at: i64,
    pub tags: Vec<String>,
}

/// Puts an archive sticker that this library did not have.
pub fn insert_restored(conn: &mut Connection, s: &RestoredSticker) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO stickers (id, name, file_name, ext, kind, width_px, height_px, width_mm, \
         height_mm, dpi, has_alpha, size_bytes, created_at, ai_at, serie, tipo, classified_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            s.id,
            s.name,
            s.file_name,
            s.ext,
            s.kind,
            s.width_px,
            s.height_px,
            s.width_mm,
            s.height_mm,
            s.dpi,
            if s.has_alpha { 1 } else { 0 },
            s.size_bytes,
            s.created_at,
            s.ai_at,
            s.serie,
            s.tipo,
            s.classified_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    set_tags(&tx, &s.id, &s.tags)?;
    tx.commit().map_err(|e| e.to_string())
}

/// Overwrites an existing sticker with the archive version, tags included.
pub fn replace_restored(conn: &mut Connection, s: &RestoredSticker) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE stickers SET name = ?2, file_name = ?3, ext = ?4, kind = ?5, width_px = ?6, \
         height_px = ?7, width_mm = ?8, height_mm = ?9, dpi = ?10, has_alpha = ?11, \
         size_bytes = ?12, created_at = ?13, ai_at = ?14, serie = ?15, tipo = ?16, \
         classified_at = ?17 WHERE id = ?1",
        params![
            s.id,
            s.name,
            s.file_name,
            s.ext,
            s.kind,
            s.width_px,
            s.height_px,
            s.width_mm,
            s.height_mm,
            s.dpi,
            if s.has_alpha { 1 } else { 0 },
            s.size_bytes,
            s.created_at,
            s.ai_at,
            s.serie,
            s.tipo,
            s.classified_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    set_tags(&tx, &s.id, &s.tags)?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn file_name_of(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT file_name FROM stickers WHERE id = ?1",
        params![id],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.to_string()),
    })
}

pub fn remove(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    let file = file_name_of(conn, id)?;
    conn.execute("DELETE FROM sticker_tags WHERE sticker_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM stickers WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(file)
}

/// The sticker the AI should look at: its thumbnail, or the artwork itself.
pub fn ai_source(conn: &Connection, root: &Path, id: &str) -> Result<Option<AiSource>, String> {
    let row = conn.query_row(
        "SELECT file_name FROM stickers WHERE id = ?1",
        params![id],
        |r| r.get::<_, String>(0),
    );
    let file_name = match row {
        Ok(found) => found,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let thumbs = root.join("thumbs");
    let png = thumbs.join(format!("{id}.png"));
    let jpg = thumbs.join(format!("{id}.jpg"));
    let path = if jpg.exists() {
        jpg
    } else if png.exists() {
        png
    } else {
        root.join("files").join(file_name)
    };
    Ok(Some(AiSource {
        id: id.to_string(),
        path,
    }))
}

/// Stores what the AI returned and marks the sticker as already processed.
pub fn apply_ai(
    conn: &Connection,
    id: &str,
    name: &str,
    serie: &str,
    tipo: &str,
    tags: &[String],
) -> Result<Vec<String>, String> {
    conn.execute(
        "UPDATE stickers SET name = ?1, serie = ?2, tipo = ?3, ai_at = ?4 WHERE id = ?5",
        params![name, serie, tipo, now_ms(), id],
    )
    .map_err(|e| e.to_string())?;
    set_tags(conn, id, tags)
}

/// One sticker waiting to be classified: text only, no image.
pub struct ClassifyCandidate {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
}

/// The stickers the text classifier has not looked at yet, newest first.
pub fn classify_queue(conn: &Connection) -> Result<Vec<ClassifyCandidate>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM stickers WHERE classified_at = 0 ORDER BY created_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for row in rows {
        found.push(row.map_err(|e| e.to_string())?);
    }
    let mut tags_map = tags_map(conn);
    Ok(found
        .into_iter()
        .map(|(id, name)| ClassifyCandidate {
            tags: tags_map.remove(&id).unwrap_or_default(),
            id,
            name,
        })
        .collect())
}

/// The series already in use, most common first: the model reuses these names
/// instead of inventing a spelling of its own.
pub fn known_series(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT serie FROM stickers WHERE serie <> '' GROUP BY serie ORDER BY COUNT(*) DESC, serie ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Writes the classification only: the name, the tags and `ai_at` stay as they are.
pub fn set_classification(
    conn: &Connection,
    id: &str,
    serie: &str,
    tipo: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE stickers SET serie = ?1, tipo = ?2, classified_at = ?3 WHERE id = ?4",
        params![serie, tipo, now_ms(), id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Applies only the fields it is given, so a bulk edit of the type never wipes
/// the series. Returns how many stickers were written.
pub fn set_classifications(
    conn: &mut Connection,
    ids: &[String],
    serie: Option<&str>,
    tipo: Option<&str>,
) -> Result<usize, String> {
    let serie = serie.map(str::trim);
    let tipo = tipo.map(str::trim);
    if serie.is_none() && tipo.is_none() {
        return Ok(0);
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut changed = 0usize;
    {
        let mut stmt = tx
            .prepare(
                "UPDATE stickers SET serie = COALESCE(?1, serie), tipo = COALESCE(?2, tipo), \
                 classified_at = ?3 WHERE id = ?4",
            )
            .map_err(|e| e.to_string())?;
        let now = now_ms();
        for id in ids {
            changed += stmt
                .execute(params![serie, tipo, now, id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

/// Merges series: every sticker named `from[i]` ends up under `into`. That is
/// the whole rename story, because the series is a canonical name, not a row.
pub fn merge_series(conn: &mut Connection, from: &[String], into: &str) -> Result<usize, String> {
    let into = into.trim();
    if into.is_empty() {
        return Err("Escribe el nombre de la serie de destino".into());
    }
    if from.iter().all(|name| name.trim().is_empty()) {
        return Err("Elige al menos una serie para fusionar".into());
    }
    // A source that already carries the target name has nothing to move, so it
    // is left out of the count the toast reports.
    let sources: Vec<&String> = from
        .iter()
        .filter(|name| {
            let name = name.trim();
            !name.is_empty() && name != into
        })
        .collect();
    if sources.is_empty() {
        return Ok(0);
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut changed = 0usize;
    {
        let mut stmt = tx
            .prepare("UPDATE stickers SET serie = ?1 WHERE serie = ?2")
            .map_err(|e| e.to_string())?;
        for source in sources {
            changed += stmt
                .execute(params![into, source])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

/// One value of a facet (a series, a type) and how many stickers carry it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetCount {
    /// Canonical name; an empty string means «no series» / «no type».
    pub value: String,
    pub count: i64,
}

fn facet_counts(conn: &Connection, column: &str) -> Result<Vec<FacetCount>, String> {
    let sql = format!(
        "SELECT {column} AS value, COUNT(*) AS count FROM stickers \
         GROUP BY {column} ORDER BY ({column} = '') ASC, count DESC, value ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FacetCount {
                value: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Series in use, most used first; the unclassified ones come last.
pub fn series_counts(conn: &Connection) -> Result<Vec<FacetCount>, String> {
    facet_counts(conn, "serie")
}

/// Types in use, most used first; the unclassified ones come last.
pub fn tipo_counts(conn: &Connection) -> Result<Vec<FacetCount>, String> {
    facet_counts(conn, "tipo")
}

pub fn all_tags(conn: &Connection) -> Result<Vec<TagCount>, String> {
    let mut stmt = conn
        .prepare("SELECT tag, COUNT(*) AS c FROM sticker_tags GROUP BY tag ORDER BY c DESC, tag ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagCount {
                tag: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// A sheet kept inside the library, so it reopens without touching the disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSheet {
    pub id: String,
    pub name: String,
    pub updated_at: i64,
    pub copies: i64,
}

const MAX_SHEET_NAME: usize = 80;

fn sheet_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let name = if trimmed.is_empty() { "Hoja" } else { trimmed };
    name.chars().take(MAX_SHEET_NAME).collect()
}

fn copy_count(payload: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| value.get("copies")?.as_array().map(|a| a.len() as i64))
        .unwrap_or(0)
}

pub fn list_sheets(conn: &Connection) -> Result<Vec<SavedSheet>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, updated_at, payload FROM sheets ORDER BY updated_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let payload: String = r.get(3)?;
            Ok(SavedSheet {
                id: r.get(0)?,
                name: r.get(1)?,
                updated_at: r.get(2)?,
                copies: copy_count(&payload),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Writes a sheet, updating it in place when it was opened from the list.
pub fn save_sheet(
    conn: &Connection,
    id: Option<&str>,
    name: &str,
    payload: &str,
) -> Result<SavedSheet, String> {
    if serde_json::from_str::<serde_json::Value>(payload).is_err() {
        return Err("La hoja no tiene datos válidos".into());
    }
    let name = sheet_name(name);
    let updated_at = now_ms();
    let id = match id {
        Some(existing) if !existing.trim().is_empty() => existing.to_string(),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    conn.execute(
        "INSERT INTO sheets (id, name, updated_at, payload) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                                       updated_at = excluded.updated_at,
                                       payload = excluded.payload",
        params![id, name, updated_at, payload],
    )
    .map_err(|e| e.to_string())?;
    Ok(SavedSheet {
        id,
        name,
        updated_at,
        copies: copy_count(payload),
    })
}

pub fn open_sheet(conn: &Connection, id: &str) -> Result<String, String> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT payload FROM sheets WHERE id = ?1", [id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Esa hoja ya no está guardada en la app".to_string())
}

/// Changes only the name: the layout saved inside stays as it is.
pub fn rename_sheet(conn: &Connection, id: &str, name: &str) -> Result<SavedSheet, String> {
    let name = sheet_name(name);
    let changed = conn
        .execute("UPDATE sheets SET name = ?1 WHERE id = ?2", params![name, id])
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("Esa hoja ya no está guardada en la app".into());
    }
    conn.query_row(
        "SELECT updated_at, payload FROM sheets WHERE id = ?1",
        [id],
        |r| {
            let payload: String = r.get(1)?;
            Ok(SavedSheet {
                id: id.to_string(),
                name: name.clone(),
                updated_at: r.get(0)?,
                copies: copy_count(&payload),
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn delete_sheet(conn: &Connection, id: &str) -> Result<usize, String> {
    conn.execute("DELETE FROM sheets WHERE id = ?1", [id])
        .map_err(|e| e.to_string())
}

pub fn totals(conn: &Connection) -> Result<(i64, i64), String> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM stickers",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        insert(
            &conn,
            &NewSticker {
                id: "a",
                name: "Pikachu",
                file_name: "a.png",
                ext: "png",
                kind: "raster",
                width_px: 100.0,
                height_px: 100.0,
                width_mm: 25.0,
                height_mm: 25.0,
                dpi: 300.0,
                has_alpha: true,
                size_bytes: 10,
                hash: "",
            },
        )
        .unwrap();
        set_tags(&conn, "a", &["pokemon".into(), "amarillo".into()]).unwrap();
        conn
    }

    #[test]
    fn init_can_run_twice_and_still_leaves_the_columns_there() {
        let conn = library();
        init(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(stickers)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for name in ["serie", "tipo", "classified_at"] {
            assert!(cols.iter().any(|c| c == name), "falta la columna {name}");
        }
    }

    #[test]
    fn classifying_leaves_name_and_tags_untouched() {
        let conn = library();
        let queue = classify_queue(&conn).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].tags, vec!["amarillo", "pokemon"]);

        set_classification(&conn, "a", "Pokémon", "videojuego").unwrap();
        assert!(classify_queue(&conn).unwrap().is_empty());

        let list = list(&conn, Path::new("C:/lib"), "", &[], "recent").unwrap();
        assert_eq!(list[0].name, "Pikachu");
        assert_eq!(list[0].tags, vec!["amarillo", "pokemon"]);
        assert_eq!(list[0].serie, "Pokémon");
        assert_eq!(list[0].tipo, "videojuego");
        assert!(list[0].classified_at > 0);
        assert_eq!(known_series(&conn).unwrap(), vec!["Pokémon"]);
    }

    #[test]
    fn the_search_box_looks_in_name_serie_tipo_and_tags() {
        let conn = library();
        set_classification(&conn, "a", "Pokémon", "videojuego").unwrap();
        let search = |q: &str| -> Vec<String> {
            list(&conn, Path::new("C:/lib"), q, &[], "name")
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect()
        };

        assert_eq!(search("pikachu"), vec!["a"]);
        assert_eq!(search("poké"), vec!["a"], "la serie");
        assert_eq!(search("videojuego"), vec!["a"], "el tipo");
        assert_eq!(search("amarillo"), vec!["a"], "una etiqueta");
        assert!(search("goku").is_empty());
        // Nothing typed yet is not a filter.
        assert_eq!(search("  ").len(), 1);
    }

    fn second(conn: &Connection, id: &str, name: &str) {
        insert(
            conn,
            &NewSticker {
                id,
                name,
                file_name: &format!("{id}.png"),
                ext: "png",
                kind: "raster",
                width_px: 1.0,
                height_px: 1.0,
                width_mm: 10.0,
                height_mm: 10.0,
                dpi: 300.0,
                has_alpha: true,
                size_bytes: 1,
                hash: "",
            },
        )
        .unwrap();
    }

    #[test]
    fn a_bulk_edit_writes_only_the_field_it_is_given() {
        let mut conn = library();
        set_classification(&conn, "a", "Pokémon", "videojuego").unwrap();
        second(&conn, "b", "Mandala");
        set_classification(&conn, "b", "Mandala", "cotidiano").unwrap();

        let ids = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            set_classifications(&mut conn, &ids, Some("Kawaii"), None).unwrap(),
            2
        );
        assert_eq!(
            set_classifications(&mut conn, &ids, None, Some("kawaii")).unwrap(),
            2
        );

        let list = list(&conn, Path::new("C:/lib"), "", &[], "name").unwrap();
        assert_eq!(list[0].serie, "Kawaii");
        assert_eq!(list[0].tipo, "kawaii");
        assert_eq!(list[1].serie, "Kawaii");
        assert_eq!(list[1].tipo, "kawaii");
        // Nothing to write is not an error, it is a no-op.
        assert_eq!(set_classifications(&mut conn, &ids, None, None).unwrap(), 0);
    }

    #[test]
    fn merging_series_moves_every_sticker_to_the_one_name() {
        let mut conn = library();
        second(&conn, "b", "Mandala");
        set_classification(&conn, "a", "Pokemon", "anime").unwrap();
        set_classification(&conn, "b", "Pokémon", "anime").unwrap();

        let moved = merge_series(&mut conn, &["Pokemon".to_string()], "Pokémon").unwrap();
        assert_eq!(moved, 1);
        assert_eq!(known_series(&conn).unwrap(), vec!["Pokémon"]);
        let counts = series_counts(&conn).unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].count, 2);

        // Merging a series into its own name moves nothing.
        assert_eq!(
            merge_series(&mut conn, &["Pokémon".to_string()], " Pokémon ").unwrap(),
            0
        );
        assert!(merge_series(&mut conn, &[], "X").is_err());
        assert!(merge_series(&mut conn, &["Pokémon".to_string()], "  ").is_err());
    }

    #[test]
    fn known_series_lists_the_most_used_first() {
        let conn = library();
        set_classification(&conn, "a", "Pokémon", "videojuego").unwrap();
        insert(
            &conn,
            &NewSticker {
                id: "b",
                name: "Mandala",
                file_name: "b.png",
                ext: "png",
                kind: "raster",
                width_px: 1.0,
                height_px: 1.0,
                width_mm: 10.0,
                height_mm: 10.0,
                dpi: 300.0,
                has_alpha: true,
                size_bytes: 1,
                hash: "",
            },
        )
        .unwrap();
        set_classification(&conn, "b", "Mandala", "cotidiano").unwrap();
        assert_eq!(known_series(&conn).unwrap(), vec!["Mandala", "Pokémon"]);
    }

    #[test]
    fn sheets_are_saved_reopened_and_deleted_in_place() {
        let conn = library();
        assert!(list_sheets(&conn).unwrap().is_empty());

        let payload = r#"{"settings":{"paper":"custom"},"copies":[{"id":"c1"},{"id":"c2"}]}"#;
        let first = save_sheet(&conn, None, "  Cuentos  ", payload).unwrap();
        assert_eq!(first.name, "Cuentos");
        assert_eq!(first.copies, 2);
        assert_eq!(open_sheet(&conn, &first.id).unwrap(), payload);

        // Saving the same sheet again rewrites it instead of piling up copies.
        let again = save_sheet(&conn, Some(&first.id), "Cuentos 2", payload).unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.name, "Cuentos 2");
        assert_eq!(list_sheets(&conn).unwrap().len(), 1);

        // A blank name still gives the sheet something to show in the list.
        let unnamed = save_sheet(&conn, None, "   ", payload).unwrap();
        assert_eq!(unnamed.name, "Hoja");
        assert_eq!(list_sheets(&conn).unwrap().len(), 2);

        let renamed = rename_sheet(&conn, &first.id, "  Otro nombre  ").unwrap();
        assert_eq!(renamed.name, "Otro nombre");
        assert_eq!(renamed.copies, 2);
        assert_eq!(open_sheet(&conn, &first.id).unwrap(), payload);
        assert!(rename_sheet(&conn, "no-existe", "X").is_err());

        assert_eq!(delete_sheet(&conn, &first.id).unwrap(), 1);
        assert!(open_sheet(&conn, &first.id).is_err());
        assert_eq!(list_sheets(&conn).unwrap().len(), 1);

        // Anything that is not JSON never reaches the table.
        assert!(save_sheet(&conn, None, "Rota", "no es json").is_err());
    }

    #[test]
    fn backup_names_carry_a_real_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    /// PNG signature plus a `pHYs` chunk: enough for the density reader.
    fn png_declaring(ppm: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&9u32.to_be_bytes());
        bytes.extend_from_slice(b"pHYs");
        bytes.extend_from_slice(&ppm.to_be_bytes());
        bytes.extend_from_slice(&ppm.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn recalculating_reads_the_density_each_file_declares() {
        let dir = std::env::temp_dir().join(format!("sns-dpi-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("files")).unwrap();
        let mut conn = library();
        let files = dir.join("files");
        // The fixture measures 100 px at 300 dpi; make its size follow that, as
        // an imported sticker does.
        conn.execute("UPDATE stickers SET width_mm = 8.47, height_mm = 8.47 WHERE id = 'a'", [])
            .unwrap();

        // 5906 px/m is 150 dpi: the 100 px sticker is 16.93 mm, not 8.47.
        std::fs::write(files.join("a.png"), png_declaring(5906)).unwrap();
        let plan = plan_densities(density_rows(&conn).unwrap(), &files);
        assert_eq!(plan.report.checked, 1);
        assert_eq!(plan.report.updated, 1);
        assert_eq!(plan.report.kept_size, 0);
        // 5906 px/m is 150.01 dpi once converted and rounded.
        assert!((plan.fixes[0].dpi - 150.0).abs() < 0.05);
        apply_densities(&mut conn, &plan.fixes).unwrap();
        let stored = list(&conn, &dir, "", &[], "recent").unwrap();
        assert!((stored[0].width_mm - 16.93).abs() < 0.01);
        assert!((stored[0].dpi - 150.0).abs() < 0.05);

        // A size set by hand stays; only the density is corrected.
        conn.execute("UPDATE stickers SET width_mm = 40, height_mm = 40 WHERE id = 'a'", [])
            .unwrap();
        std::fs::write(files.join("a.png"), png_declaring(11811)).unwrap();
        let plan = plan_densities(density_rows(&conn).unwrap(), &files);
        assert_eq!(plan.report.kept_size, 1);
        assert_eq!(plan.report.updated, 0);
        apply_densities(&mut conn, &plan.fixes).unwrap();
        let stored = list(&conn, &dir, "", &[], "recent").unwrap();
        assert!((stored[0].width_mm - 40.0).abs() < 1e-6);
        assert!((stored[0].dpi - 300.0).abs() < 1e-6);

        // A file without density is left alone.
        std::fs::write(files.join("a.png"), b"not a png").unwrap();
        let plan = plan_densities(density_rows(&conn).unwrap(), &files);
        assert_eq!(plan.report.without_density, 1);
        assert!(plan.fixes.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_daily_copy_is_written_once_and_stays_readable() {
        let dir = std::env::temp_dir().join(format!("sns-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = library();

        let first = backup(&dir, &conn).unwrap().expect("a copy is due");
        assert!(first.exists());
        // The copy is a working database, with the same single sticker.
        let copy = Connection::open(&first).unwrap();
        assert_eq!(totals(&copy).unwrap().0, 1);
        assert_eq!(check_integrity(&copy).unwrap(), "ok");

        // A second run the same day does not pile up files.
        assert!(backup(&dir, &conn).unwrap().is_none());
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
