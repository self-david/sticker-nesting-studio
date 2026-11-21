use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

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
    /// When the AI last named and tagged this sticker (0 = never).
    pub ai_at: i64,
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
            created_at  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sticker_tags (
            sticker_id TEXT NOT NULL,
            tag        TEXT NOT NULL,
            PRIMARY KEY (sticker_id, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_sticker_tags_tag ON sticker_tags(tag);
        CREATE INDEX IF NOT EXISTS idx_stickers_created ON stickers(created_at DESC);
        "#,
    )?;
    // Libraries created before the AI section have no ai_at column yet.
    let has_ai_at: bool = conn
        .prepare("PRAGMA table_info(stickers)")?
        .query_map([], |r| r.get::<_, String>(1))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map(|cols| cols.iter().any(|c| c == "ai_at"))
        .unwrap_or(false);
    if !has_ai_at {
        conn.execute_batch("ALTER TABLE stickers ADD COLUMN ai_at INTEGER NOT NULL DEFAULT 0;")?;
    }
    Ok(())
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
}

pub fn insert(conn: &Connection, s: &NewSticker<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO stickers (id, name, file_name, ext, kind, width_px, height_px, width_mm, height_mm, dpi, has_alpha, size_bytes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
        ],
    )?;
    Ok(())
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
        "SELECT id, name, file_name, ext, kind, width_px, height_px, width_mm, height_mm, dpi, has_alpha, size_bytes, created_at, ai_at FROM stickers",
    );
    let mut clauses: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        clauses.push("lower(name) LIKE ?".into());
        vals.push(Box::new(format!("%{}%", q)));
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
    tags: &[String],
) -> Result<Vec<String>, String> {
    conn.execute(
        "UPDATE stickers SET name = ?1, ai_at = ?2 WHERE id = ?3",
        params![name, now_ms(), id],
    )
    .map_err(|e| e.to_string())?;
    set_tags(conn, id, tags)
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

pub fn totals(conn: &Connection) -> Result<(i64, i64), String> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM stickers",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .map_err(|e| e.to_string())
}
