//! A saved sheet is one file with the app's own extension (`.sns`): a ZIP
//! container (stored entries — the artwork is already compressed) holding the
//! image files plus `hoja.json` with the layout. Reopening it unpacks the
//! contents into the app cache and rebuilds the sheet exactly, so it can be
//! printed again later instead of keeping a flattened PDF.
//!
//! The previous format (a folder with `archivos/` and `hoja.json`) is still
//! readable: opening one converts it to a `.sns` file next to it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const PROJECT_FILE: &str = "hoja.json";
pub const PROJECT_APP: &str = "sticker-nesting-studio";
pub const PROJECT_VERSION: u32 = 1;
pub const PROJECT_EXT: &str = "sns";
const ASSETS_DIR: &str = "archivos";
const THUMBS_DIR: &str = "miniaturas";
const DEFAULT_NAME: &str = "hoja-stickers";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub paper: String,
    pub orientation: String,
    pub margin_mm: f64,
    pub gap_mm: f64,
    pub cut_lines: bool,
    pub outline_width_mm: f64,
    pub outline_pad_mm: f64,
    /// Page size used when `paper` is "custom"; older files leave it out.
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCopy {
    pub id: String,
    pub sticker_id: String,
    pub name: String,
    pub ext: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub thumb_path: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub aspect: f64,
    #[serde(default)]
    pub rotation: f64,
}

/// Same shape in both formats, so a legacy `hoja.json` converts as-is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredCopy {
    id: String,
    sticker_id: String,
    name: String,
    ext: String,
    width_mm: f64,
    height_mm: f64,
    aspect: f64,
    rotation: f64,
    /// Path inside the archive, e.g. `archivos/<id>.png`.
    file: String,
    /// Optional preview stored next to the artwork; empty means "use file".
    #[serde(default)]
    thumb: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProject {
    version: u32,
    app: String,
    saved_at: i64,
    settings: ProjectSettings,
    copies: Vec<StoredCopy>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedCopy {
    pub id: String,
    pub sticker_id: String,
    pub name: String,
    pub ext: String,
    pub file_path: String,
    pub thumb_path: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub aspect: f64,
    pub rotation: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedProject {
    /// Folder the sheet was unpacked into (inside the app cache).
    pub folder: String,
    /// Set when a legacy folder was converted to a `.sns` file while opening.
    pub converted_to: Option<String>,
    pub settings: ProjectSettings,
    pub copies: Vec<OpenedCopy>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn normalize_rotation(raw: f64) -> f64 {
    if !raw.is_finite() {
        return 0.0;
    }
    let turns = (raw.round() as i64).rem_euclid(360);
    ((turns / 90) * 90).rem_euclid(360) as f64
}

fn safe_id(id: &str) -> String {
    let clean: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "copia".to_string()
    } else {
        clean
    }
}

fn asset_ext(declared: &str, source: &Path) -> String {
    let ext = declared.trim().trim_start_matches('.').to_ascii_lowercase();
    if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return ext;
    }
    let from_source = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if from_source.is_empty() {
        "png".to_string()
    } else {
        from_source
    }
}

/// Only plain relative paths are accepted, so a crafted archive can never
/// write outside the folder it is unpacked into.
fn safe_relative(raw: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in Path::new(raw).components() {
        if let Component::Normal(segment) = part {
            out.push(segment);
        }
    }
    out
}

/// Adds the app extension when the picked name has none (or another one).
fn with_project_ext(path: &Path) -> PathBuf {
    let matches = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(PROJECT_EXT))
        .unwrap_or(false);
    if matches {
        return path.to_path_buf();
    }
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| DEFAULT_NAME.into());
    name.push(format!(".{PROJECT_EXT}"));
    path.with_file_name(name)
}

pub fn save(
    destination: &Path,
    settings: ProjectSettings,
    copies: Vec<SaveCopy>,
) -> Result<PathBuf, String> {
    if copies.is_empty() {
        return Err("La hoja está vacía: añade copias antes de guardarla".into());
    }
    let target = with_project_ext(destination);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err("La carpeta de destino no existe".into());
        }
    }
    write_sheet(&target, &collect_entries(settings, copies)?)
}

/// Reads `hoja.json` and the artwork of the current sheet into archive entries.
fn collect_entries(
    settings: ProjectSettings,
    copies: Vec<SaveCopy>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // Each distinct file is stored once, so 200 copies of one design stay small.
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut stored: Vec<StoredCopy> = Vec::with_capacity(copies.len());

    for copy in copies {
        let source = if copy.file_path.trim().is_empty() {
            PathBuf::from(&copy.thumb_path)
        } else {
            PathBuf::from(&copy.file_path)
        };
        if !source.is_file() {
            return Err(format!("No se encuentra el archivo de «{}»", copy.name));
        }
        let ext = asset_ext(&copy.ext, &source);
        let file = store(&mut entries, &mut seen, ASSETS_DIR, &copy.id, &ext, &source)
            .map_err(|e| format!("No se pudo leer «{}»: {e}", copy.name))?;

        let thumb_source = PathBuf::from(copy.thumb_path.trim());
        let thumb = if copy.thumb_path.trim().is_empty()
            || thumb_source == source
            || !thumb_source.is_file()
        {
            String::new()
        } else {
            let ext = asset_ext("", &thumb_source);
            // A missing preview is not a reason to lose the sheet.
            store(&mut entries, &mut seen, THUMBS_DIR, &copy.id, &ext, &thumb_source)
                .unwrap_or_default()
        };

        stored.push(StoredCopy {
            id: copy.id,
            sticker_id: copy.sticker_id,
            name: copy.name,
            ext,
            width_mm: copy.width_mm,
            height_mm: copy.height_mm,
            aspect: copy.aspect,
            rotation: normalize_rotation(copy.rotation),
            file,
            thumb,
        });
    }

    let project = StoredProject {
        version: PROJECT_VERSION,
        app: PROJECT_APP.to_string(),
        saved_at: now_ms(),
        settings,
        copies: stored,
    };
    let json = serde_json::to_vec_pretty(&project)
        .map_err(|e| format!("No se pudo escribir la hoja: {e}"))?;
    entries.push((PROJECT_FILE.to_string(), json));
    Ok(entries)
}

/// Copies `source` into the archive as `dir/<id>.<ext>` unless that exact file
/// is already there, and returns its path inside the archive.
fn store(
    entries: &mut Vec<(String, Vec<u8>)>,
    seen: &mut HashMap<String, String>,
    dir: &str,
    id: &str,
    ext: &str,
    source: &Path,
) -> std::io::Result<String> {
    let key = source.to_string_lossy().to_lowercase();
    if let Some(rel) = seen.get(&key) {
        return Ok(rel.clone());
    }
    let rel = format!("{dir}/{}.{}", safe_id(id), ext);
    let bytes = fs::read(source)?;
    entries.push((rel.clone(), bytes));
    seen.insert(key, rel.clone());
    Ok(rel)
}

/// Writes the archive next to its target and renames it, so a failure never
/// leaves a half-written sheet behind.
fn write_sheet(target: &Path, entries: &[(String, Vec<u8>)]) -> Result<PathBuf, String> {
    let temp = target.with_file_name(format!(
        "{}.{}.tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| DEFAULT_NAME.to_string()),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<(), String> {
        let mut file = fs::File::create(&temp)
            .map_err(|e| format!("No se pudo crear el archivo de la hoja: {e}"))?;
        file.write_all(&write_zip(entries))
            .map_err(|e| format!("No se pudo escribir la hoja: {e}"))?;
        drop(file);
        fs::rename(&temp, target).map_err(|e| format!("No se pudo guardar la hoja: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map(|_| target.to_path_buf())
}

fn sanitize_settings(mut settings: ProjectSettings) -> ProjectSettings {
    if settings.paper != "letter" && settings.paper != "custom" {
        settings.paper = "a4".to_string();
    }
    settings.custom_width_mm = settings.custom_width_mm.clamp(20.0, 2000.0);
    settings.custom_height_mm = settings.custom_height_mm.clamp(20.0, 2000.0);
    if settings.orientation != "landscape" {
        settings.orientation = "portrait".to_string();
    }
    settings.margin_mm = settings.margin_mm.clamp(0.0, 100.0);
    settings.gap_mm = settings.gap_mm.clamp(0.0, 30.0);
    settings.outline_width_mm = settings.outline_width_mm.clamp(0.05, 5.0);
    settings.outline_pad_mm = settings.outline_pad_mm.clamp(0.0, 30.0);
    settings
}

fn check_manifest(project: &StoredProject) -> Result<(), String> {
    if project.app != PROJECT_APP {
        return Err("El archivo no pertenece a Sticker Nesting Studio".into());
    }
    if project.version > PROJECT_VERSION {
        return Err(format!(
            "La hoja se guardó con una versión más reciente (v{}). Actualiza la app para abrirla",
            project.version
        ));
    }
    if project.copies.is_empty() {
        return Err("La hoja guardada no contiene copias".into());
    }
    Ok(())
}

/// Stable per-file folder name, so saving over the same sheet reuses it.
fn cache_key(file: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Opens a `.sns` file, or a folder saved with the previous format (which is
/// converted to a `.sns` file next to it). Either way the artwork is unpacked
/// into `cache`, removing the folders left by earlier openings.
pub fn open(path: &Path, cache: &Path) -> Result<OpenedProject, String> {
    if !path.exists() {
        return Err("La hoja seleccionada no existe".into());
    }
    let converted_to = if path.is_dir() {
        Some(convert_legacy_folder(path)?)
    } else {
        None
    };
    let sheet = converted_to.as_deref().map(Path::new).unwrap_or(path);
    let bytes = fs::read(sheet).map_err(|e| format!("No se pudo leer la hoja: {e}"))?;
    let entries = read_zip(&bytes)?;

    let json = entries
        .iter()
        .find(|(name, _)| name == PROJECT_FILE)
        .map(|(_, data)| data.clone())
        .ok_or_else(|| {
            format!("El archivo no es una hoja de Sticker Nesting Studio (falta {PROJECT_FILE})")
        })?;
    let project: StoredProject =
        serde_json::from_slice(&json).map_err(|e| format!("{PROJECT_FILE} no es válido: {e}"))?;
    check_manifest(&project)?;

    let root = cache.join(cache_key(sheet));
    if let Ok(others) = fs::read_dir(cache) {
        for other in others.flatten() {
            if other.path() != root {
                let _ = fs::remove_dir_all(other.path());
            }
        }
    }
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)
        .map_err(|e| format!("No se pudo preparar la hoja guardada: {e}"))?;

    for (name, data) in entries {
        if name == PROJECT_FILE {
            continue;
        }
        let rel = safe_relative(&name);
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = root.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("No se pudo preparar la hoja guardada: {e}"))?;
        }
        fs::write(&dest, data)
            .map_err(|e| format!("No se pudo extraer el archivo de la hoja: {e}"))?;
    }

    let mut copies = Vec::with_capacity(project.copies.len());
    let mut missing = Vec::new();
    for copy in project.copies {
        let file = root.join(safe_relative(&copy.file));
        if copy.file.is_empty() || !file.is_file() {
            missing.push(copy.name.clone());
            continue;
        }
        let candidate = root.join(safe_relative(&copy.thumb));
        let thumb = if copy.thumb.is_empty() || !candidate.is_file() {
            file.clone()
        } else {
            candidate
        };
        copies.push(OpenedCopy {
            id: copy.id,
            sticker_id: copy.sticker_id,
            name: copy.name,
            ext: copy.ext,
            file_path: file.to_string_lossy().to_string(),
            thumb_path: thumb.to_string_lossy().to_string(),
            width_mm: copy.width_mm.clamp(0.1, 2000.0),
            height_mm: copy.height_mm.clamp(0.1, 2000.0),
            aspect: if copy.aspect.is_finite() && copy.aspect > 0.0 {
                copy.aspect
            } else {
                1.0
            },
            rotation: normalize_rotation(copy.rotation),
        });
    }
    if !missing.is_empty() {
        return Err(format!(
            "Faltan {} archivos de la hoja guardada: {}",
            missing.len(),
            missing
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(OpenedProject {
        folder: root.to_string_lossy().to_string(),
        converted_to: converted_to.map(|p| p.to_string_lossy().to_string()),
        settings: sanitize_settings(project.settings),
        copies,
    })
}

/// Finds the sheet folder (the chosen one, or the only saved one inside it).
fn legacy_root(folder: &Path) -> Result<PathBuf, String> {
    if folder.join(PROJECT_FILE).is_file() {
        return Ok(folder.to_path_buf());
    }
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(|e| format!("No se pudo leer la carpeta: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir() && dir.join(PROJECT_FILE).is_file())
        .collect();
    match found.len() {
        0 => Err(format!(
            "La carpeta elegida no contiene una hoja guardada (ni {PROJECT_EXT} ni {PROJECT_FILE})"
        )),
        1 => Ok(found.remove(0)),
        _ => Err(
            "Hay varias hojas guardadas ahí: elige la carpeta de la hoja que quieres abrir".into(),
        ),
    }
}

/// Packs a folder saved with the previous format into `<name>.sns` next to it.
fn convert_legacy_folder(folder: &Path) -> Result<PathBuf, String> {
    let root = legacy_root(folder)?;
    let raw = fs::read_to_string(root.join(PROJECT_FILE))
        .map_err(|e| format!("No se pudo leer {PROJECT_FILE}: {e}"))?;
    let project: StoredProject =
        serde_json::from_str(&raw).map_err(|e| format!("{PROJECT_FILE} no es válido: {e}"))?;
    check_manifest(&project)?;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for copy in &project.copies {
        for rel in [&copy.file, &copy.thumb] {
            if rel.is_empty() || seen.contains_key(rel) {
                continue;
            }
            let source = root.join(safe_relative(rel));
            let bytes = fs::read(&source).map_err(|_| {
                format!("No se encuentra el archivo de «{}» en la carpeta antigua", copy.name)
            })?;
            entries.push((rel.clone(), bytes));
            seen.insert(rel.clone(), rel.clone());
        }
    }
    entries.push((PROJECT_FILE.to_string(), raw.into_bytes()));

    let parent = root.parent().filter(|p| p.is_dir()).unwrap_or(&root);
    let stem = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| DEFAULT_NAME.to_string());
    write_sheet(&unique_sheet_path(parent, &stem), &entries)
}

fn unique_sheet_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{PROJECT_EXT}"));
    if !first.exists() {
        return first;
    }
    for n in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({n}).{PROJECT_EXT}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.{PROJECT_EXT}", now_ms()))
}

// --- Minimal ZIP container -------------------------------------------------

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;
/// Bit 11: file names are UTF-8.
const FLAG_UTF8: u16 = 0x0800;
const DOS_DATE_1980: u16 = (1 << 5) | 1;

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc = table[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for (name, data) in entries {
        let offset = out.len() as u32;
        let name_bytes = name.as_bytes();
        let size = data.len() as u32;
        let crc = crc32(data);

        push_u32(&mut out, LOCAL_HEADER);
        push_u16(&mut out, 20); // version needed
        push_u16(&mut out, FLAG_UTF8);
        push_u16(&mut out, 0); // stored
        push_u16(&mut out, 0); // time
        push_u16(&mut out, DOS_DATE_1980);
        push_u32(&mut out, crc);
        push_u32(&mut out, size);
        push_u32(&mut out, size);
        push_u16(&mut out, name_bytes.len() as u16);
        push_u16(&mut out, 0); // extra
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        push_u32(&mut central, CENTRAL_HEADER);
        push_u16(&mut central, 20); // version made by
        push_u16(&mut central, 20); // version needed
        push_u16(&mut central, FLAG_UTF8);
        push_u16(&mut central, 0); // stored
        push_u16(&mut central, 0); // time
        push_u16(&mut central, DOS_DATE_1980);
        push_u32(&mut central, crc);
        push_u32(&mut central, size);
        push_u32(&mut central, size);
        push_u16(&mut central, name_bytes.len() as u16);
        push_u16(&mut central, 0); // extra
        push_u16(&mut central, 0); // comment
        push_u16(&mut central, 0); // disk
        push_u16(&mut central, 0); // internal attributes
        push_u32(&mut central, 0); // external attributes
        push_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    push_u32(&mut out, END_OF_CENTRAL);
    push_u16(&mut out, 0); // this disk
    push_u16(&mut out, 0); // disk with the directory
    push_u16(&mut out, entries.len() as u16);
    push_u16(&mut out, entries.len() as u16);
    push_u32(&mut out, central_size);
    push_u32(&mut out, central_offset);
    push_u16(&mut out, 0); // comment length
    out
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn not_a_sheet() -> String {
    format!("El archivo no es una hoja de Sticker Nesting Studio (se esperaba un .{PROJECT_EXT})")
}

fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if bytes.len() < 22 {
        return Err(not_a_sheet());
    }
    // The end-of-directory record sits in the last 22 bytes plus a comment.
    let tail = bytes.len().saturating_sub(22 + 65_535);
    let eocd = (tail..=bytes.len() - 22)
        .rev()
        .find(|&i| read_u32(bytes, i) == Some(END_OF_CENTRAL))
        .ok_or_else(not_a_sheet)?;

    let count = read_u16(bytes, eocd + 10).ok_or_else(not_a_sheet)? as usize;
    let mut at = read_u32(bytes, eocd + 16).ok_or_else(not_a_sheet)? as usize;

    let mut out: Vec<(String, Vec<u8>)> = Vec::with_capacity(count);
    for _ in 0..count {
        if read_u32(bytes, at) != Some(CENTRAL_HEADER) {
            return Err(not_a_sheet());
        }
        let method = read_u16(bytes, at + 10).ok_or_else(not_a_sheet)?;
        let size = read_u32(bytes, at + 20).ok_or_else(not_a_sheet)? as usize;
        let name_len = read_u16(bytes, at + 28).ok_or_else(not_a_sheet)? as usize;
        let extra_len = read_u16(bytes, at + 30).ok_or_else(not_a_sheet)? as usize;
        let comment_len = read_u16(bytes, at + 32).ok_or_else(not_a_sheet)? as usize;
        let local = read_u32(bytes, at + 42).ok_or_else(not_a_sheet)? as usize;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .and_then(|raw| std::str::from_utf8(raw).ok())
            .ok_or_else(not_a_sheet)?
            .to_string();
        at += 46 + name_len + extra_len + comment_len;

        // Entries written by this app are stored as-is.
        if method != 0 {
            return Err("La hoja guardada está comprimida de una forma no soportada".into());
        }
        if read_u32(bytes, local) != Some(LOCAL_HEADER) {
            return Err(not_a_sheet());
        }
        let local_name = read_u16(bytes, local + 26).ok_or_else(not_a_sheet)? as usize;
        let local_extra = read_u16(bytes, local + 28).ok_or_else(not_a_sheet)? as usize;
        let start = local + 30 + local_name + local_extra;
        let data = bytes
            .get(start..start + size)
            .ok_or_else(not_a_sheet)?;
        out.push((name, data.to_vec()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sns-project-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settings() -> ProjectSettings {
        ProjectSettings {
            paper: "a4".into(),
            orientation: "portrait".into(),
            margin_mm: 8.0,
            gap_mm: 3.0,
            cut_lines: true,
            outline_width_mm: 0.3,
            outline_pad_mm: 1.5,
            custom_width_mm: 210.0,
            custom_height_mm: 297.0,
        }
    }

    fn copy_named(id: &str, source: &Path, name: &str) -> SaveCopy {
        SaveCopy {
            id: id.into(),
            sticker_id: format!("s-{id}"),
            name: name.into(),
            ext: "png".into(),
            file_path: source.to_string_lossy().to_string(),
            thumb_path: source.to_string_lossy().to_string(),
            width_mm: 40.0,
            height_mm: 20.0,
            aspect: 2.0,
            rotation: 0.0,
        }
    }

    /// Writes the folder layout used before the `.sns` format.
    fn legacy_bundle(root: &Path, art: &[u8]) -> PathBuf {
        let bundle = root.join("hoja-stickers");
        fs::create_dir_all(bundle.join(ASSETS_DIR)).unwrap();
        fs::write(bundle.join(ASSETS_DIR).join("c1.png"), art).unwrap();
        let manifest = format!(
            r#"{{
  "version": 1,
  "app": "sticker-nesting-studio",
  "savedAt": 1,
  "settings": {{"paper": "a4", "orientation": "landscape", "marginMm": 10.0,
    "gapMm": 2.0, "cutLines": true, "outlineWidthMm": 0.4, "outlinePadMm": 1.0}},
  "copies": [{{"id": "c1", "stickerId": "s1", "name": "Star", "ext": "png",
    "widthMm": 30.0, "heightMm": 15.0, "aspect": 2.0, "rotation": 270,
    "file": "archivos/c1.png", "thumb": ""}}]
}}"#
        );
        fs::write(bundle.join(PROJECT_FILE), manifest).unwrap();
        bundle
    }

    #[test]
    fn saves_one_file_with_the_app_extension_and_reopens_the_same_sheet() {
        let root = temp_dir("roundtrip");
        let art = root.join("star.png");
        fs::write(&art, b"png-bytes").unwrap();

        let mut turned = copy_named("copy-b", &art, "Star");
        turned.rotation = 90.0;
        let copies = vec![copy_named("copy-a", &art, "Star"), turned];
        let sheet = save(&root.join("mi-hoja"), settings(), copies).unwrap();

        assert_eq!(sheet.extension().unwrap(), PROJECT_EXT);
        assert!(sheet.is_file());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2); // art + sheet

        let opened = open(&sheet, &root.join("cache")).unwrap();
        assert!(opened.converted_to.is_none());
        assert_eq!(opened.copies.len(), 2);
        assert_eq!(opened.copies[1].rotation, 90.0);
        assert_eq!(fs::read(&opened.copies[0].file_path).unwrap(), b"png-bytes");
        assert_eq!(opened.settings.margin_mm, 8.0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_the_extension_when_it_is_already_there() {
        assert_eq!(
            with_project_ext(Path::new("/tmp/hoja.SNS")).file_name().unwrap(),
            "hoja.SNS"
        );
        assert_eq!(
            with_project_ext(Path::new("/tmp/hoja.pdf")).file_name().unwrap(),
            "hoja.pdf.sns"
        );
        assert_eq!(
            with_project_ext(Path::new("/tmp/hoja")).file_name().unwrap(),
            "hoja.sns"
        );
    }

    #[test]
    fn reopening_reuses_the_cache_folder_and_drops_older_ones() {
        let root = temp_dir("cache");
        let art = root.join("heart.png");
        fs::write(&art, b"art").unwrap();
        let cache = root.join("cache");
        let a = save(&root.join("a"), settings(), vec![copy_named("h", &art, "Heart")]).unwrap();
        let b = save(&root.join("b"), settings(), vec![copy_named("h", &art, "Heart")]).unwrap();

        let first = open(&a, &cache).unwrap();
        let again = open(&a, &cache).unwrap();
        assert_eq!(first.folder, again.folder);

        let other = open(&b, &cache).unwrap();
        assert_ne!(other.folder, first.folder);
        assert!(!Path::new(&first.folder).exists());
        assert!(Path::new(&other.folder).is_dir());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reports_missing_artwork_instead_of_opening_a_broken_sheet() {
        let root = temp_dir("missing");
        let art = root.join("gone.png");
        fs::write(&art, b"x").unwrap();
        let sheet = save(&root.join("hoja"), settings(), vec![copy_named("g", &art, "Gone")]).unwrap();

        let kept: Vec<(String, Vec<u8>)> = read_zip(&fs::read(&sheet).unwrap())
            .unwrap()
            .into_iter()
            .filter(|(name, _)| name == PROJECT_FILE)
            .collect();
        fs::write(&sheet, write_zip(&kept)).unwrap();

        let error = open(&sheet, &root.join("cache")).unwrap_err();
        assert!(error.contains("Faltan 1"), "{error}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_save_an_empty_sheet() {
        let root = temp_dir("empty");
        assert!(save(&root.join("hoja"), settings(), Vec::new()).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_an_error() {
        let root = temp_dir("corrupt");
        let bad = root.join("hoja.sns");
        fs::write(&bad, b"not a zip").unwrap();
        let error = open(&bad, &root.join("cache")).unwrap_err();
        assert!(error.contains(PROJECT_EXT), "{error}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_an_archive_without_a_manifest() {
        let root = temp_dir("nomanifest");
        let bad = root.join("hoja.sns");
        fs::write(&bad, write_zip(&[("archivos/a.png".into(), b"x".to_vec())])).unwrap();
        assert!(open(&bad, &root.join("cache")).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn opens_an_old_folder_and_converts_it_to_a_sheet_file() {
        let root = temp_dir("legacy");
        let bundle = legacy_bundle(&root, b"old-png");
        let cache = root.join("cache");

        let opened = open(&bundle, &cache).unwrap();
        assert_eq!(opened.copies.len(), 1);
        assert_eq!(opened.copies[0].name, "Star");
        assert_eq!(opened.copies[0].rotation, 270.0);
        assert_eq!(fs::read(&opened.copies[0].file_path).unwrap(), b"old-png");
        assert_eq!(opened.settings.orientation, "landscape");

        let converted = opened.converted_to.expect("converted file");
        assert!(converted.ends_with(".sns"));
        assert!(Path::new(&converted).is_file());
        // The old folder is kept, untouched.
        assert!(bundle.join(PROJECT_FILE).is_file());

        // The new file opens on its own and matches.
        let again = open(Path::new(&converted), &cache).unwrap();
        assert!(again.converted_to.is_none());
        assert_eq!(again.copies.len(), 1);
        assert_eq!(again.copies[0].rotation, 270.0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn opens_an_old_folder_by_picking_its_parent() {
        let root = temp_dir("legacy-parent");
        legacy_bundle(&root, b"old");
        let opened = open(&root, &root.join("cache")).unwrap();
        assert_eq!(opened.copies.len(), 1);
        assert!(Path::new(opened.converted_to.as_deref().unwrap()).is_file());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_an_old_folder_with_missing_artwork() {
        let root = temp_dir("legacy-missing");
        let bundle = legacy_bundle(&root, b"old");
        fs::remove_file(bundle.join(ASSETS_DIR).join("c1.png")).unwrap();
        let error = open(&bundle, &root.join("cache")).unwrap_err();
        assert!(error.contains("carpeta antigua"), "{error}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rounds_rotation_to_quarter_turns() {
        assert_eq!(normalize_rotation(90.0), 90.0);
        assert_eq!(normalize_rotation(270.0), 270.0);
        assert_eq!(normalize_rotation(360.0), 0.0);
        assert_eq!(normalize_rotation(-90.0), 270.0);
        assert_eq!(normalize_rotation(45.0), 0.0);
        assert_eq!(normalize_rotation(f64::NAN), 0.0);
    }
}
