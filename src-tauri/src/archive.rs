//! The whole library in one ZIP, so it can move between machines and come back.
//!
//! Layout: `biblioteca.json` holds every column plus the tags of each sticker,
//! and `archivos/<id>.<ext>` the artwork. Previews are not stored: they are
//! rebuilt on import, which keeps the file smaller and avoids shipping stale
//! thumbnails.
//!
//! The ZIP is written and read entry by entry (stored, no compression — the
//! artwork is already compressed) so a library of a gigabyte never sits in
//! memory. Entries carry a data descriptor, since their CRC is only known once
//! the file has been streamed.

use crate::db::{self, RestoredSticker, Sticker};
use crate::importer;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "biblioteca.json";
pub const APP_ID: &str = "sticker-nesting-studio";
pub const VERSION: u32 = 1;
const ART_DIR: &str = "archivos";
const DEFAULT_NAME: &str = "biblioteca-stickers";

/// One sticker as the manifest stores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSticker {
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
    #[serde(default)]
    pub serie: String,
    #[serde(default)]
    pub tipo: String,
    #[serde(default)]
    pub ai_at: i64,
    #[serde(default)]
    pub classified_at: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    app: String,
    exported_at: i64,
    stickers: Vec<ArchiveSticker>,
}

/// A sticker that already exists here, shown next to the one in the archive.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: String,
    pub name: String,
    pub existing_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peek {
    pub total: usize,
    pub conflicts: Vec<Conflict>,
}

/// What to do with one conflict: keep this library's copy, keep the archive's,
/// or keep both (the incoming one gets a fresh id).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub id: String,
    pub action: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// New stickers with the id they had in the archive.
    pub imported: usize,
    /// Conflicts whose archive copy replaced the local one.
    pub replaced: usize,
    /// Conflicts kept twice, the incoming one under a new id.
    pub duplicated: usize,
    /// Conflicts left as they were.
    pub skipped: usize,
    pub failures: Vec<Failure>,
}

/// One archive entry that will be written: its final id and whether it lands on
/// top of an existing row.
pub struct Plan {
    pub sticker: ArchiveSticker,
    pub id: String,
    pub replace: bool,
}

pub struct Planned {
    pub plans: Vec<Plan>,
    pub imported: usize,
    pub replaced: usize,
    pub duplicated: usize,
    pub skipped: usize,
}

pub struct Outcome {
    pub plan: Plan,
    pub error: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Ids become file names, so an archive can never point outside `files/`.
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn safe_ext(ext: &str) -> bool {
    importer::allowed_ext(&ext.to_ascii_lowercase())
}

/// Only plain relative names with no way up are accepted.
fn safe_entry(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains("..")
        && !raw.starts_with('/')
        && !raw.contains('\\')
}

/// Adds `.zip` when the picked name has no extension.
pub fn with_archive_ext(path: &Path) -> PathBuf {
    let matches = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if matches {
        return path.to_path_buf();
    }
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| DEFAULT_NAME.into());
    name.push(".zip");
    path.with_file_name(name)
}

// --- export ---------------------------------------------------------------

/// Writes every sticker with its artwork. Missing artwork is skipped: a sticker
/// without its file would be useless on the other side.
pub fn export(root: &Path, stickers: &[Sticker], destination: &Path) -> Result<PathBuf, String> {
    if stickers.is_empty() {
        return Err("No hay stickers para exportar".into());
    }
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err("La carpeta de destino no existe".into());
        }
    }
    let target = with_archive_ext(destination);
    let temp = target.with_file_name(format!(
        "{}.{}.tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| DEFAULT_NAME.to_string()),
        uuid::Uuid::new_v4()
    ));

    let result = (|| -> Result<(), String> {
        let file = File::create(&temp)
            .map_err(|e| format!("No se pudo crear el archivo de la biblioteca: {e}"))?;
        let mut zip = ZipWriter::new(file);
        let files = root.join("files");
        let mut entries = Vec::with_capacity(stickers.len());

        for sticker in stickers {
            let source = files.join(&sticker.file_name);
            if !sticker.file_name.is_empty() && safe_entry(&sticker.file_name) && source.is_file() {
                let name = format!("{ART_DIR}/{}", sticker.file_name);
                zip.add_file(&name, &source)
                    .map_err(|e| format!("No se pudo leer «{}»: {e}", sticker.name))?;
            }
            entries.push(ArchiveSticker {
                id: sticker.id.clone(),
                name: sticker.name.clone(),
                file_name: sticker.file_name.clone(),
                ext: sticker.ext.clone(),
                kind: sticker.kind.clone(),
                width_px: sticker.width_px,
                height_px: sticker.height_px,
                width_mm: sticker.width_mm,
                height_mm: sticker.height_mm,
                dpi: sticker.dpi,
                has_alpha: sticker.has_alpha,
                size_bytes: sticker.size_bytes,
                created_at: sticker.created_at,
                serie: sticker.serie.clone(),
                tipo: sticker.tipo.clone(),
                ai_at: sticker.ai_at,
                classified_at: sticker.classified_at,
                tags: sticker.tags.clone(),
            });
        }

        let manifest = Manifest {
            version: VERSION,
            app: APP_ID.to_string(),
            exported_at: now_ms(),
            stickers: entries,
        };
        let json = serde_json::to_vec(&manifest)
            .map_err(|e| format!("No se pudo escribir el índice: {e}"))?;
        zip.add_bytes(MANIFEST, &json)
            .map_err(|e| format!("No se pudo escribir el índice: {e}"))?;
        zip.finish()
            .map_err(|e| format!("No se pudo terminar el archivo: {e}"))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return result.map(|_| target);
    }
    fs::rename(&temp, &target)
        .map_err(|e| format!("No se pudo guardar la biblioteca: {e}"))?;
    Ok(target)
}

// --- inspect --------------------------------------------------------------

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let mut archive = Archive::open(path)?;
    if archive.info(MANIFEST).is_none() {
        return Err(not_a_library());
    }
    let bytes = archive.read(MANIFEST, 16 * 1024 * 1024)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST} no es válido: {e}"))?;
    if manifest.app != APP_ID {
        return Err(not_a_library());
    }
    if manifest.version > VERSION {
        return Err(format!(
            "La biblioteca se exportó con una versión más reciente (v{}). Actualiza la app para importarla",
            manifest.version
        ));
    }
    Ok(manifest)
}

fn not_a_library() -> String {
    "El archivo no es una biblioteca de Sticker Nesting Studio (se esperaba un .zip con su índice)"
        .into()
}

/// Tells how many of the archive's stickers are already here.
pub fn peek(path: &Path, existing: &HashMap<String, String>) -> Result<Peek, String> {
    let manifest = read_manifest(path)?;
    let conflicts = manifest
        .stickers
        .iter()
        .filter_map(|sticker| {
            existing.get(&sticker.id).map(|name| Conflict {
                id: sticker.id.clone(),
                name: sticker.name.clone(),
                existing_name: name.clone(),
            })
        })
        .collect();
    Ok(Peek {
        total: manifest.stickers.len(),
        conflicts,
    })
}

// --- import ---------------------------------------------------------------

/// Works out where every sticker goes before touching a single file.
pub fn plan(
    path: &Path,
    decisions: &[Decision],
    existing: &HashMap<String, String>,
) -> Result<Planned, String> {
    let manifest = read_manifest(path)?;
    let chosen: HashMap<&str, &str> = decisions
        .iter()
        .map(|decision| (decision.id.as_str(), decision.action.as_str()))
        .collect();

    let mut planned = Planned {
        plans: Vec::new(),
        imported: 0,
        replaced: 0,
        duplicated: 0,
        skipped: 0,
    };

    for sticker in manifest.stickers {
        if !safe_id(&sticker.id) || !safe_ext(&sticker.ext) || !safe_entry(&sticker.file_name) {
            planned.skipped += 1;
            continue;
        }
        if !existing.contains_key(&sticker.id) {
            planned.imported += 1;
            planned.plans.push(Plan {
                id: sticker.id.clone(),
                sticker,
                replace: false,
            });
            continue;
        }
        match chosen.get(sticker.id.as_str()).copied() {
            Some("replace") => {
                planned.replaced += 1;
                planned.plans.push(Plan {
                    id: sticker.id.clone(),
                    sticker,
                    replace: true,
                });
            }
            Some("both") => {
                planned.duplicated += 1;
                planned.plans.push(Plan {
                    id: uuid::Uuid::new_v4().to_string(),
                    sticker,
                    replace: false,
                });
            }
            _ => planned.skipped += 1,
        }
    }
    Ok(planned)
}

/// Copies each planned artwork into `files/` under its final id and rebuilds
/// the preview. One failing sticker does not stop the rest.
pub fn extract(root: &Path, path: &Path, plans: Vec<Plan>) -> Result<Vec<Outcome>, String> {
    let mut archive = Archive::open(path)?;
    let files = root.join("files");
    let thumbs = root.join("thumbs");
    let mut outcomes = Vec::with_capacity(plans.len());

    for plan in plans {
        let entry_name = format!("{ART_DIR}/{}", plan.sticker.file_name);
        let outcome = match archive.extract(&entry_name, &files, &plan.id, &plan.sticker.ext) {
            Err(message) => Outcome {
                plan,
                error: Some(message),
            },
            Ok(target) => {
                let thumb = importer::probe(&target, &plan.sticker.ext, &thumbs, &plan.id);
                // A vector file without width is still usable: its size travels
                // in the manifest and there is no preview to build.
                match thumb {
                    Ok(_) => Outcome { plan, error: None },
                    Err(message) if plan.sticker.ext.eq_ignore_ascii_case("svg") => {
                        let _ = message;
                        Outcome { plan, error: None }
                    }
                    Err(message) => {
                        let _ = fs::remove_file(&target);
                        Outcome {
                            plan,
                            error: Some(message),
                        }
                    }
                }
            }
        };
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

/// Writes one extracted sticker. `replace` overwrites the row it collides with.
pub fn store(
    conn: &mut Connection,
    root: &Path,
    outcome: &Outcome,
) -> Result<(), String> {
    let plan = &outcome.plan;
    let file_name = format!("{}.{}", plan.id, plan.sticker.ext);
    let size_bytes = fs::metadata(root.join("files").join(&file_name))
        .map(|meta| meta.len() as i64)
        .unwrap_or(plan.sticker.size_bytes);
    let sticker = RestoredSticker {
        id: plan.id.clone(),
        name: plan.sticker.name.trim().to_string(),
        file_name,
        ext: plan.sticker.ext.clone(),
        kind: plan.sticker.kind.clone(),
        width_px: plan.sticker.width_px,
        height_px: plan.sticker.height_px,
        width_mm: plan.sticker.width_mm.clamp(1.0, 2000.0),
        height_mm: plan.sticker.height_mm.clamp(1.0, 2000.0),
        dpi: plan.sticker.dpi,
        has_alpha: plan.sticker.has_alpha,
        size_bytes,
        created_at: if plan.sticker.created_at > 0 {
            plan.sticker.created_at
        } else {
            now_ms()
        },
        ai_at: plan.sticker.ai_at,
        serie: plan.sticker.serie.trim().to_string(),
        tipo: plan.sticker.tipo.trim().to_string(),
        classified_at: plan.sticker.classified_at,
        tags: plan.sticker.tags.clone(),
    };
    if sticker.name.is_empty() {
        return Err("El sticker no tiene nombre".into());
    }

    if plan.replace {
        // The old row may point at another file name (a different extension).
        if let Some(old) = db::file_name_of(conn, &sticker.id)? {
            if old != sticker.file_name {
                let _ = fs::remove_file(root.join("files").join(old));
            }
        }
        db::replace_restored(conn, &sticker)
    } else {
        db::insert_restored(conn, &sticker)
    }
}

// --- ZIP container --------------------------------------------------------

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;
/// Bit 11: UTF-8 names. Bit 3: sizes and CRC travel after the data.
const FLAGS: u16 = 0x0800 | 0x0008;
const DOS_DATE_1980: u16 = (1 << 5) | 1;
const COPY_CHUNK: usize = 128 * 1024;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }
    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let index = ((self.0 ^ byte as u32) & 0xFF) as usize;
            self.0 = CRC_TABLE[index] ^ (self.0 >> 8);
        }
    }
    fn value(&self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Central {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

struct ZipWriter {
    out: BufWriter<File>,
    offset: u64,
    entries: Vec<Central>,
}

impl ZipWriter {
    fn new(file: File) -> Self {
        ZipWriter {
            out: BufWriter::with_capacity(COPY_CHUNK, file),
            offset: 0,
            entries: Vec::new(),
        }
    }

    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.out.write_all(data)?;
        self.offset += data.len() as u64;
        Ok(())
    }

    fn header(&mut self, name: &str) -> std::io::Result<u32> {
        if self.offset > u32::MAX as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "la biblioteca es demasiado grande para un solo archivo",
            ));
        }
        let start = self.offset as u32;
        let mut head = Vec::with_capacity(30 + name.len());
        push_u32(&mut head, LOCAL_HEADER);
        push_u16(&mut head, 20);
        push_u16(&mut head, FLAGS);
        push_u16(&mut head, 0); // stored
        push_u16(&mut head, 0); // time
        push_u16(&mut head, DOS_DATE_1980);
        push_u32(&mut head, 0); // crc, in the descriptor
        push_u32(&mut head, 0); // compressed size
        push_u32(&mut head, 0); // uncompressed size
        push_u16(&mut head, name.len() as u16);
        push_u16(&mut head, 0); // extra
        head.extend_from_slice(name.as_bytes());
        self.write(&head)?;
        Ok(start)
    }

    fn descriptor(&mut self, crc: u32, size: u32) -> std::io::Result<()> {
        let mut tail = Vec::with_capacity(16);
        push_u32(&mut tail, DATA_DESCRIPTOR);
        push_u32(&mut tail, crc);
        push_u32(&mut tail, size);
        push_u32(&mut tail, size);
        self.write(&tail)
    }

    fn add_bytes(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if data.len() > u32::MAX as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "entrada demasiado grande",
            ));
        }
        let offset = self.header(name)?;
        self.write(data)?;
        let mut crc = Crc32::new();
        crc.update(data);
        let crc = crc.value();
        self.descriptor(crc, data.len() as u32)?;
        self.entries.push(Central {
            name: name.to_string(),
            crc,
            size: data.len() as u32,
            offset,
        });
        Ok(())
    }

    fn add_file(&mut self, name: &str, source: &Path) -> std::io::Result<()> {
        let meta = fs::metadata(source)?;
        if meta.len() > u32::MAX as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "el archivo es demasiado grande",
            ));
        }
        let offset = self.header(name)?;
        let mut reader = BufReader::with_capacity(COPY_CHUNK, File::open(source)?);
        let mut crc = Crc32::new();
        let mut size = 0u64;
        let mut buffer = vec![0u8; COPY_CHUNK];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            self.write(&buffer[..read])?;
            size += read as u64;
        }
        let crc = crc.value();
        self.descriptor(crc, size as u32)?;
        self.entries.push(Central {
            name: name.to_string(),
            crc,
            size: size as u32,
            offset,
        });
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        if self.offset > u32::MAX as u64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "la biblioteca es demasiado grande para un solo archivo",
            ));
        }
        let start = self.offset as u32;
        let mut central = Vec::new();
        for entry in &self.entries {
            push_u32(&mut central, CENTRAL_HEADER);
            push_u16(&mut central, 20); // version made by
            push_u16(&mut central, 20); // version needed
            push_u16(&mut central, FLAGS);
            push_u16(&mut central, 0); // stored
            push_u16(&mut central, 0); // time
            push_u16(&mut central, DOS_DATE_1980);
            push_u32(&mut central, entry.crc);
            push_u32(&mut central, entry.size);
            push_u32(&mut central, entry.size);
            push_u16(&mut central, entry.name.len() as u16);
            push_u16(&mut central, 0); // extra
            push_u16(&mut central, 0); // comment
            push_u16(&mut central, 0); // disk
            push_u16(&mut central, 0); // internal attributes
            push_u32(&mut central, 0); // external attributes
            push_u32(&mut central, entry.offset);
            central.extend_from_slice(entry.name.as_bytes());
        }
        let size = central.len() as u32;
        self.write(&central)?;
        let mut end = Vec::with_capacity(22);
        push_u32(&mut end, END_OF_CENTRAL);
        push_u16(&mut end, 0);
        push_u16(&mut end, 0);
        push_u16(&mut end, self.entries.len() as u16);
        push_u16(&mut end, self.entries.len() as u16);
        push_u32(&mut end, size);
        push_u32(&mut end, start);
        push_u16(&mut end, 0);
        self.write(&end)?;
        self.out.flush()
    }
}

struct Entry {
    name: String,
    method: u16,
    size: u32,
    crc: u32,
    local: u32,
}

struct Archive {
    file: File,
    entries: Vec<Entry>,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

impl Archive {
    fn open(path: &Path) -> Result<Archive, String> {
        if !path.is_file() {
            return Err("El archivo elegido no existe".into());
        }
        let mut file = File::open(path).map_err(|e| format!("No se pudo abrir el archivo: {e}"))?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if length < 22 {
            return Err(not_a_library());
        }
        // The end-of-directory record is in the last 22 bytes plus a comment.
        let tail_len = std::cmp::min(length, 22 + 65_535) as usize;
        let mut tail = vec![0u8; tail_len];
        file.seek(SeekFrom::Start(length - tail_len as u64))
            .map_err(|e| e.to_string())?;
        file.read_exact(&mut tail).map_err(|e| e.to_string())?;
        let eocd = (0..=tail.len() - 22)
            .rev()
            .find(|&at| read_u32(&tail, at) == Some(END_OF_CENTRAL))
            .ok_or_else(not_a_library)?;

        let count = read_u16(&tail, eocd + 10).ok_or_else(not_a_library)? as usize;
        let mut at = read_u32(&tail, eocd + 16).ok_or_else(not_a_library)? as u64;
        let mut central = vec![0u8; (length - at) as usize];
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        file.read_exact(&mut central).map_err(|e| e.to_string())?;

        let mut entries = Vec::with_capacity(count);
        let mut cursor = 0usize;
        for _ in 0..count {
            if read_u32(&central, cursor) != Some(CENTRAL_HEADER) {
                return Err(not_a_library());
            }
            let method = read_u16(&central, cursor + 10).ok_or_else(not_a_library)?;
            let crc = read_u32(&central, cursor + 16).ok_or_else(not_a_library)?;
            let size = read_u32(&central, cursor + 24).ok_or_else(not_a_library)?;
            let name_len = read_u16(&central, cursor + 28).ok_or_else(not_a_library)? as usize;
            let extra_len = read_u16(&central, cursor + 30).ok_or_else(not_a_library)? as usize;
            let comment_len = read_u16(&central, cursor + 32).ok_or_else(not_a_library)? as usize;
            let local = read_u32(&central, cursor + 42).ok_or_else(not_a_library)?;
            let name = central
                .get(cursor + 46..cursor + 46 + name_len)
                .and_then(|raw| std::str::from_utf8(raw).ok())
                .ok_or_else(not_a_library)?
                .to_string();
            cursor += 46 + name_len + extra_len + comment_len;
            entries.push(Entry {
                name,
                method,
                size,
                crc,
                local,
            });
        }
        let _ = &mut at;
        Ok(Archive { file, entries })
    }

    /// Size, CRC, local offset and method of one entry.
    fn info(&self, name: &str) -> Option<(u32, u32, u32, u16)> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| (entry.size, entry.crc, entry.local, entry.method))
    }

    fn data_start(&mut self, local: u32, method: u16) -> Result<u64, String> {
        if method != 0 {
            return Err("La biblioteca está comprimida de una forma no soportada".into());
        }
        let mut head = [0u8; 30];
        self.file
            .seek(SeekFrom::Start(local as u64))
            .map_err(|e| e.to_string())?;
        self.file.read_exact(&mut head).map_err(|e| e.to_string())?;
        if read_u32(&head, 0) != Some(LOCAL_HEADER) {
            return Err(not_a_library());
        }
        let name_len = read_u16(&head, 26).ok_or_else(not_a_library)? as u64;
        let extra_len = read_u16(&head, 28).ok_or_else(not_a_library)? as u64;
        Ok(local as u64 + 30 + name_len + extra_len)
    }

    fn read(&mut self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let (size, _, local, method) = self
            .info(name)
            .ok_or_else(|| format!("Falta «{name}» dentro del archivo"))?;
        if size as u64 > limit {
            return Err("El índice de la biblioteca es demasiado grande".into());
        }
        let start = self.data_start(local, method)?;
        self.file
            .seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut data = vec![0u8; size as usize];
        self.file.read_exact(&mut data).map_err(|e| e.to_string())?;
        Ok(data)
    }

    /// Copies one entry to `files/<id>.<ext>` and checks its checksum.
    fn extract(
        &mut self,
        name: &str,
        files: &Path,
        id: &str,
        ext: &str,
    ) -> Result<PathBuf, String> {
        let (size, crc_local, local, method) = self
            .info(name)
            .ok_or_else(|| format!("Falta «{name}» dentro del archivo"))?;
        let start = self.data_start(local, method)?;
        let target = files.join(format!("{id}.{}", ext.to_ascii_lowercase()));
        fs::create_dir_all(files).map_err(|e| e.to_string())?;

        let result = (|| -> Result<(), String> {
            self.file
                .seek(SeekFrom::Start(start))
                .map_err(|e| e.to_string())?;
            let mut out = BufWriter::with_capacity(COPY_CHUNK, File::create(&target).map_err(|e| {
                format!("No se pudo escribir el archivo del sticker: {e}")
            })?);
            let mut remaining = size as u64;
            let mut crc = Crc32::new();
            let mut buffer = vec![0u8; COPY_CHUNK];
            while remaining > 0 {
                let want = std::cmp::min(remaining as usize, buffer.len());
                self.file
                    .read_exact(&mut buffer[..want])
                    .map_err(|e| format!("El archivo está truncado: {e}"))?;
                crc.update(&buffer[..want]);
                out.write_all(&buffer[..want])
                    .map_err(|e| format!("No se pudo escribir el archivo del sticker: {e}"))?;
                remaining -= want as u64;
            }
            out.flush().map_err(|e| e.to_string())?;
            if crc.value() != crc_local {
                return Err("El archivo llegó dañado (no coincide su suma de control)".into());
            }
            Ok(())
        })();

        match result {
            Ok(()) => Ok(target),
            Err(message) => {
                let _ = fs::remove_file(&target);
                Err(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, NewSticker};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sns-archive-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("files")).unwrap();
        fs::create_dir_all(dir.join("thumbs")).unwrap();
        dir
    }

    fn art(dir: &Path, id: &str) -> PathBuf {
        let path = dir.join("files").join(format!("{id}.png"));
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 120, 40, 255]));
        image.save(&path).unwrap();
        path
    }

    fn library(dir: &Path) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let id = "s1";
        art(dir, id);
        db::insert(
            &conn,
            &NewSticker {
                id,
                name: "Pikachu",
                file_name: "s1.png",
                ext: "png",
                kind: "raster",
                width_px: 8.0,
                height_px: 8.0,
                width_mm: 25.4,
                height_mm: 25.4,
                dpi: 300.0,
                has_alpha: true,
                size_bytes: 10,
            },
        )
        .unwrap();
        db::set_tags(&conn, id, &["kawaii".into(), "pokemon".into()]).unwrap();
        db::set_classification(&conn, id, "Pokémon", "kawaii").unwrap();
        conn
    }

    fn stickers(conn: &Connection, root: &Path) -> Vec<Sticker> {
        db::list(conn, root, "", &[], "recent").unwrap()
    }

    #[test]
    fn exports_and_brings_back_a_sticker_with_every_field() {
        let root = temp_dir("roundtrip");
        let conn = library(&root);
        let archive = export(&root, &stickers(&conn, &root), &root.join("copia")).unwrap();
        assert_eq!(archive.extension().unwrap(), "zip");

        // A fresh library imports it without asking anything.
        let other = temp_dir("roundtrip-other");
        let mut fresh = Connection::open_in_memory().unwrap();
        db::init(&fresh).unwrap();
        let existing = db::id_names(&fresh).unwrap();
        let peeked = peek(&archive, &existing).unwrap();
        assert_eq!(peeked.total, 1);
        assert!(peeked.conflicts.is_empty());

        let planned = plan(&archive, &[], &existing).unwrap();
        assert_eq!(planned.imported, 1);
        let outcomes = extract(&other, &archive, planned.plans).unwrap();
        for outcome in &outcomes {
            assert!(outcome.error.is_none(), "{:?}", outcome.error);
            store(&mut fresh, &other, outcome).unwrap();
        }

        let list = db::list(&fresh, &other, "", &[], "recent").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Pikachu");
        assert_eq!(list[0].serie, "Pokémon");
        assert_eq!(list[0].tipo, "kawaii");
        assert_eq!(list[0].tags, vec!["kawaii", "pokemon"]);
        assert!(list[0].ai_at > 0 || list[0].classified_at > 0);
        assert!(Path::new(&list[0].thumb_path).is_file());

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&other).unwrap();
    }

    #[test]
    fn a_conflict_can_keep_either_copy_or_both() {
        let root = temp_dir("conflict");
        let conn = library(&root);
        let archive = export(&root, &stickers(&conn, &root), &root.join("copia")).unwrap();
        let mut target = library(&root); // same id already here
        let existing = db::id_names(&target).unwrap();

        let kept = plan(
            &archive,
            &[Decision {
                id: "s1".into(),
                action: "keep".into(),
            }],
            &existing,
        )
        .unwrap();
        assert_eq!((kept.skipped, kept.plans.len()), (1, 0));

        let replaced = plan(
            &archive,
            &[Decision {
                id: "s1".into(),
                action: "replace".into(),
            }],
            &existing,
        )
        .unwrap();
        assert_eq!((replaced.replaced, replaced.plans.len()), (1, 1));
        assert!(replaced.plans[0].replace);
        assert_eq!(replaced.plans[0].id, "s1");

        let both = plan(
            &archive,
            &[Decision {
                id: "s1".into(),
                action: "both".into(),
            }],
            &existing,
        )
        .unwrap();
        assert_eq!((both.duplicated, both.plans.len()), (1, 1));
        assert_ne!(both.plans[0].id, "s1");

        // The "both" copy lands with a new row, the old one stays put.
        let outcomes = extract(&root, &archive, both.plans).unwrap();
        for outcome in &outcomes {
            store(&mut target, &root, outcome).unwrap();
        }
        assert_eq!(db::list(&target, &root, "", &[], "recent").unwrap().len(), 2);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replacing_overwrites_the_name_and_tags_of_the_local_copy() {
        let root = temp_dir("replace");
        let conn = library(&root);
        let archive = export(&root, &stickers(&conn, &root), &root.join("copia")).unwrap();
        let mut target = library(&root);
        db::update_meta(&target, "s1", Some("Nombre local"), None, None).unwrap();
        db::set_tags(&target, "s1", &["otra".into()]).unwrap();
        let existing = db::id_names(&target).unwrap();

        let planned = plan(
            &archive,
            &[Decision {
                id: "s1".into(),
                action: "replace".into(),
            }],
            &existing,
        )
        .unwrap();
        let outcomes = extract(&root, &archive, planned.plans).unwrap();
        for outcome in &outcomes {
            store(&mut target, &root, outcome).unwrap();
        }
        let list = db::list(&target, &root, "", &[], "recent").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Pikachu");
        assert_eq!(list[0].tags, vec!["kawaii", "pokemon"]);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_a_file_that_is_not_a_library() {
        let root = temp_dir("nope");
        let bad = root.join("cualquiera.zip");
        fs::write(&bad, b"not a zip").unwrap();
        let existing = HashMap::new();
        assert!(peek(&bad, &existing).is_err());

        // A real archive without the manifest is rejected too.
        let mut zip = ZipWriter::new(File::create(root.join("otro.zip")).unwrap());
        zip.add_bytes("archivos/x.png", b"x").unwrap();
        zip.finish().unwrap();
        let error = peek(&root.join("otro.zip"), &existing).unwrap_err();
        assert!(error.contains("no es una biblioteca"), "{error}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_damaged_file_is_reported_instead_of_stored() {
        let root = temp_dir("damaged");
        let conn = library(&root);
        let archive = export(&root, &stickers(&conn, &root), &root.join("copia")).unwrap();

        // Flip a byte of the artwork inside the archive.
        let mut bytes = fs::read(&archive).unwrap();
        let needle = b"archivos/s1.png";
        let at = bytes
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap();
        let data = at + needle.len();
        bytes[data] ^= 0xFF;
        fs::write(&archive, &bytes).unwrap();

        let planned = plan(&archive, &[], &HashMap::new()).unwrap();
        let outcomes = extract(&root, &archive, planned.plans).unwrap();
        assert!(outcomes[0].error.is_some());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adds_the_zip_extension_only_when_it_is_missing() {
        assert_eq!(
            with_archive_ext(Path::new("/tmp/biblioteca.ZIP"))
                .file_name()
                .unwrap(),
            "biblioteca.ZIP"
        );
        assert_eq!(
            with_archive_ext(Path::new("/tmp/biblioteca"))
                .file_name()
                .unwrap(),
            "biblioteca.zip"
        );
    }
}
