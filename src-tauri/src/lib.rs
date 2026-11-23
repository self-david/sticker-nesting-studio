mod ai;
mod archive;
mod contour;
mod db;
mod importer;
mod nest;
mod pdf;
mod project;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Manager, State};

pub struct AppState {
    pub conn: Mutex<Connection>,
    pub root: PathBuf,
    /// Which saved key the next AI request starts from.
    pub ai_cursor: Mutex<usize>,
    /// Keys Groq has throttled, with the moment they can be used again.
    pub ai_cooldowns: Mutex<HashMap<String, Instant>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportFailure {
    path: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportReport {
    imported_ids: Vec<String>,
    errors: Vec<ImportFailure>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryInfo {
    root: String,
    count: i64,
    bytes: i64,
}

#[tauri::command]
fn import_paths(state: State<'_, AppState>, paths: Vec<String>) -> Result<ImportReport, String> {
    let files = state.root.join("files");
    let thumbs = state.root.join("thumbs");
    let conn = state.conn.lock().map_err(|e| e.to_string())?;

    let mut imported_ids = Vec::new();
    let mut errors = Vec::new();

    for raw in paths {
        let src = PathBuf::from(&raw);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !importer::allowed_ext(&ext) {
            errors.push(ImportFailure {
                path: raw.clone(),
                message: "Formato no compatible (PNG, SVG, JPG o WebP)".into(),
            });
            continue;
        }
        if !src.is_file() {
            errors.push(ImportFailure {
                path: raw.clone(),
                message: "El archivo no existe".into(),
            });
            continue;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let dest = importer::target_file(&files, &id, &ext);

        let result = (|| -> Result<(), String> {
            fs::copy(&src, &dest).map_err(|e| format!("No se pudo copiar: {e}"))?;
            let probe = importer::probe(&dest, &ext, &thumbs, &id)?;
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let size_bytes = fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);
            let file_name = dest
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            db::insert(
                &conn,
                &db::NewSticker {
                    id: &id,
                    name: &importer::sanitize_name(&stem),
                    file_name: &file_name,
                    ext: &ext,
                    kind: probe.kind,
                    width_px: probe.width_px,
                    height_px: probe.height_px,
                    width_mm: probe.width_mm,
                    height_mm: probe.height_mm,
                    dpi: probe.dpi,
                    has_alpha: probe.has_alpha,
                    size_bytes,
                },
            )
            .map_err(|e| format!("No se pudo guardar en la biblioteca: {e}"))?;
            Ok(())
        })();

        match result {
            Ok(()) => imported_ids.push(id),
            Err(message) => {
                let _ = fs::remove_file(&dest);
                let _ = fs::remove_file(thumbs.join(format!("{id}.png")));
                let _ = fs::remove_file(thumbs.join(format!("{id}.jpg")));
                errors.push(ImportFailure { path: raw, message });
            }
        }
    }

    Ok(ImportReport {
        imported_ids,
        errors,
    })
}

#[tauri::command]
fn list_stickers(
    state: State<'_, AppState>,
    query: Option<String>,
    tags: Option<Vec<String>>,
    sort: Option<String>,
) -> Result<Vec<db::Sticker>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::list(
        &conn,
        &state.root,
        query.as_deref().unwrap_or(""),
        tags.as_deref().unwrap_or(&[]),
        sort.as_deref().unwrap_or("recent"),
    )
}

#[tauri::command]
fn list_tags(state: State<'_, AppState>) -> Result<Vec<db::TagCount>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::all_tags(&conn)
}

#[tauri::command]
fn list_series(state: State<'_, AppState>) -> Result<Vec<db::FacetCount>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::series_counts(&conn)
}

#[tauri::command]
fn list_tipos(state: State<'_, AppState>) -> Result<Vec<db::FacetCount>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::tipo_counts(&conn)
}

/// Manual edit of the pair: the panel's two fields, not the AI classifier.
#[tauri::command]
fn set_sticker_classification(
    state: State<'_, AppState>,
    id: String,
    serie: String,
    tipo: String,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_classification(&conn, &id, &serie, &tipo)
}

/// Bulk edit from the selection bar: each field is optional, so setting the
/// type of 30 stickers leaves their series alone.
#[tauri::command]
fn set_stickers_classification(
    state: State<'_, AppState>,
    ids: Vec<String>,
    serie: Option<String>,
    tipo: Option<String>,
) -> Result<usize, String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_classifications(&mut conn, &ids, serie.as_deref(), tipo.as_deref())
}

/// Folds several series into one name. A rename is the same operation with a
/// single source, so there is only one command to keep right.
#[tauri::command]
fn merge_series(
    state: State<'_, AppState>,
    from: Vec<String>,
    into: String,
) -> Result<usize, String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::merge_series(&mut conn, &from, &into)
}

/// Writes the whole library into one `.zip`: the artwork plus a manifest with
/// every column and the tags, so another machine can import it as it was.
#[tauri::command]
async fn export_library(
    state: State<'_, AppState>,
    destination: String,
) -> Result<String, String> {
    let root = state.root.clone();
    let stickers = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::list(&conn, &root, "", &[], "recent")?
    };
    let target = PathBuf::from(destination);
    tauri::async_runtime::spawn_blocking(move || {
        archive::export(&root, &stickers, &target).map(|path| path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Reads an archive's manifest and says which stickers are already here, so the
/// window can ask what to do with each one before anything is written.
#[tauri::command]
async fn inspect_library_archive(
    state: State<'_, AppState>,
    path: String,
) -> Result<archive::Peek, String> {
    let existing = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::id_names(&conn)?
    };
    let file = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || archive::peek(&file, &existing))
        .await
        .map_err(|e| e.to_string())?
}

/// Imports an archive applying the chosen conflict decisions. The artwork is
/// copied and thumbnailed off the UI thread; only the rows are written here.
#[tauri::command]
async fn import_library_archive(
    state: State<'_, AppState>,
    path: String,
    decisions: Vec<archive::Decision>,
) -> Result<archive::ImportReport, String> {
    let root = state.root.clone();
    let existing = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        db::id_names(&conn)?
    };
    let file = PathBuf::from(&path);
    let planned = tauri::async_runtime::spawn_blocking({
        let file = file.clone();
        move || archive::plan(&file, &decisions, &existing)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut report = archive::ImportReport {
        imported: 0,
        replaced: 0,
        duplicated: 0,
        skipped: planned.skipped,
        failures: Vec::new(),
    };
    if planned.plans.is_empty() {
        return Ok(report);
    }

    let handoff = root.clone();
    let outcomes = tauri::async_runtime::spawn_blocking(move || {
        archive::extract(&handoff, &file, planned.plans)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    for outcome in outcomes {
        let name = outcome.plan.sticker.name.clone();
        if let Some(message) = outcome.error.clone() {
            report.failures.push(archive::Failure { name, message });
            continue;
        }
        let duplicated = outcome.plan.sticker.id != outcome.plan.id;
        let replaced = outcome.plan.replace;
        match archive::store(&mut conn, &root, &outcome) {
            Ok(()) if replaced => report.replaced += 1,
            Ok(()) if duplicated => report.duplicated += 1,
            Ok(()) => report.imported += 1,
            Err(message) => report.failures.push(archive::Failure { name, message }),
        }
    }
    Ok(report)
}

#[tauri::command]
fn update_sticker(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    width_mm: Option<f64>,
    height_mm: Option<f64>,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::update_meta(
        &conn,
        &id,
        name.as_deref(),
        width_mm,
        height_mm,
    )
}

#[tauri::command]
fn set_sticker_tags(
    state: State<'_, AppState>,
    id: String,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::set_tags(&conn, &id, &tags)
}

#[tauri::command]
fn ai_status(state: State<'_, AppState>) -> Result<ai::AiStatus, String> {
    Ok(ai::status(&state.root))
}

// Keys only travel one way: the webview never gets to read them back.
#[tauri::command]
fn ai_add_key(
    state: State<'_, AppState>,
    name: String,
    key: String,
) -> Result<ai::AiStatus, String> {
    ai::add_key(&state.root, &name, &key)
}

#[tauri::command]
fn ai_update_key(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    key: Option<String>,
) -> Result<ai::AiStatus, String> {
    ai::update_key(&state.root, &id, name.as_deref(), key.as_deref())
}

#[tauri::command]
fn ai_remove_key(state: State<'_, AppState>, id: String) -> Result<ai::AiStatus, String> {
    ai::remove_key(&state.root, &id)
}

#[tauri::command]
fn ai_reorder_keys(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<ai::AiStatus, String> {
    ai::reorder_keys(&state.root, &ids)
}

/// Names and tags the given stickers from their thumbnails. The caller decides
/// the ids, so the same command serves both the bulk run and one sticker.
#[tauri::command]
async fn ai_describe(state: State<'_, AppState>, ids: Vec<String>) -> Result<ai::Report, String> {
    let Some((keys, model)) = ai::load(&state.root) else {
        return Err("Añade al menos una clave de Groq en «Renombrar con IA».".to_string());
    };

    let mut sources = Vec::new();
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        for id in &ids {
            if let Some(source) = db::ai_source(&conn, &state.root, id)? {
                sources.push(source);
            }
        }
    }
    if sources.is_empty() {
        return Err("No hay stickers que actualizar".into());
    }

    let client = ai::client()?;
    let mut updated = Vec::new();
    let mut failures = Vec::new();

    for source in sources {
        let image = match ai::prepare(&source) {
            Ok(image) => image,
            Err(message) => {
                failures.push(ai::Failure {
                    id: source.id,
                    message,
                });
                continue;
            }
        };
        let chat = ai::Chat {
            model: &model,
            system: ai::SYSTEM,
            content: ai::vision_content(&image),
            max_tokens: 300,
        };
        let outcome = ask_with_keys(&state, &client, &keys, &chat).await;
        match outcome {
            Ok(raw) => {
                let answer = match ai::answer_from(&raw) {
                    Ok(answer) => answer,
                    Err(message) => {
                        failures.push(ai::Failure {
                            id: source.id,
                            message,
                        });
                        continue;
                    }
                };
                let saved = {
                    let conn = state.conn.lock().map_err(|e| e.to_string())?;
                    db::apply_ai(
                        &conn,
                        &source.id,
                        &answer.name,
                        &answer.serie,
                        &answer.tipo,
                        &answer.tags,
                    )
                };
                match saved {
                    Ok(_) => updated.push(source.id),
                    Err(message) => failures.push(ai::Failure {
                        id: source.id,
                        message,
                    }),
                }
            }
            Err(message) => failures.push(ai::Failure {
                id: source.id,
                message,
            }),
        }
    }

    Ok(ai::Report { updated, failures })
}

/// Fills the series and the type of every sticker that has not been classified
/// yet, reading only its name and tags: the artwork is never sent again, and
/// neither the name nor the tags are written.
#[tauri::command]
async fn ai_classify(state: State<'_, AppState>) -> Result<ai::Report, String> {
    let Some((keys, model)) = ai::load(&state.root) else {
        return Err("Añade al menos una clave de Groq en «Renombrar con IA».".to_string());
    };

    let (queue, mut series) = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        (db::classify_queue(&conn)?, db::known_series(&conn)?)
    };
    let mut updated = Vec::new();
    let mut failures = Vec::new();
    if queue.is_empty() {
        return Ok(ai::Report { updated, failures });
    }

    let client = ai::client()?;
    for chunk in queue.chunks(ai::CLASSIFY_BATCH) {
        let inputs: Vec<ai::ClassifyInput> = chunk
            .iter()
            .map(|item| ai::ClassifyInput {
                id: item.id.clone(),
                name: item.name.clone(),
                tags: item.tags.clone(),
            })
            .collect();
        let wanted: Vec<String> = inputs.iter().map(|item| item.id.clone()).collect();
        let chat = ai::Chat {
            model: &model,
            system: ai::CLASSIFY_SYSTEM,
            content: ai::classify_content(&series, &inputs),
            max_tokens: ai::CLASSIFY_MAX_TOKENS,
        };

        let raw = match ask_with_keys(&state, &client, &keys, &chat).await {
            Ok(raw) => raw,
            Err(message) => {
                failures.push(ai::Failure {
                    id: String::new(),
                    message,
                });
                continue;
            }
        };
        let items = match ai::parse_classifications(&raw, &wanted) {
            Ok(items) => items,
            Err(message) => {
                failures.push(ai::Failure {
                    id: String::new(),
                    message,
                });
                continue;
            }
        };
        if items.is_empty() {
            failures.push(ai::Failure {
                id: String::new(),
                message: "La IA no clasificó ninguno de estos stickers".into(),
            });
            continue;
        }

        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        for item in items {
            match db::set_classification(&conn, &item.id, &item.serie, &item.tipo) {
                Ok(_) => {
                    // The next batches see the series this one just used.
                    if !item.serie.is_empty()
                        && !series.iter().any(|known| known.eq_ignore_ascii_case(&item.serie))
                    {
                        series.push(item.serie.clone());
                    }
                    updated.push(item.id);
                }
                Err(message) => failures.push(ai::Failure {
                    id: item.id,
                    message,
                }),
            }
        }
    }

    Ok(ai::Report { updated, failures })
}

/// One request per key, starting where the previous sticker left off, and
/// skipping the keys Groq has just throttled so no request is wasted on a
/// key that is known to be busy.
async fn ask_with_keys(
    state: &State<'_, AppState>,
    client: &reqwest::Client,
    keys: &[ai::StoredKey],
    chat: &ai::Chat<'_>,
) -> Result<String, String> {
    let start = {
        let mut cursor = state.ai_cursor.lock().map_err(|e| e.to_string())?;
        let start = *cursor % keys.len();
        *cursor = (start + 1) % keys.len();
        start
    };

    let now = Instant::now();
    let cooling = state
        .ai_cooldowns
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();

    let mut ready = Vec::new();
    let mut soonest: Option<(Instant, usize)> = None;
    for offset in 0..keys.len() {
        let index = (start + offset) % keys.len();
        match cooling.get(&keys[index].id) {
            Some(until) if *until > now => {
                if soonest.map_or(true, |(best, _)| *until < best) {
                    soonest = Some((*until, index));
                }
            }
            _ => ready.push(index),
        }
    }

    // Every key is cooling down: wait for the first one to free up.
    if ready.is_empty() {
        if let Some((until, index)) = soonest {
            tokio::time::sleep(until.saturating_duration_since(now) + Duration::from_millis(250))
                .await;
            ready.push(index);
        }
    }

    let mut waits: Vec<(f64, usize)> = Vec::new();
    let mut problems = Vec::new();

    for index in ready {
        let key = &keys[index];
        match ai::complete(client, key, chat).await {
            Ok(raw) => {
                clear_cooldown(state, &key.id);
                return Ok(raw);
            }
            Err(ai::CallError::Throttled(wait)) => {
                set_cooldown(state, &key.id, wait);
                waits.push((wait, index));
            }
            Err(ai::CallError::Key(message)) => {
                problems.push(format!("{}: {message}", key.name))
            }
            Err(ai::CallError::Other(message)) => return Err(message),
        }
    }

    // Every key was busy: wait for the shortest limit and try that one again.
    if problems.is_empty() {
        if let Some((wait, index)) = waits.iter().copied().min_by(|a, b| a.0.total_cmp(&b.0)) {
            tokio::time::sleep(Duration::from_secs_f64(wait.min(60.0) + 0.3)).await;
            match ai::complete(client, &keys[index], chat).await {
                Ok(raw) => {
                    clear_cooldown(state, &keys[index].id);
                    return Ok(raw);
                }
                Err(ai::CallError::Key(message)) => {
                    problems.push(format!("{}: {message}", keys[index].name))
                }
                Err(ai::CallError::Throttled(_)) => {
                    return Err(
                        "Groq sigue limitando las peticiones; espera un minuto y continúa".into()
                    )
                }
                Err(ai::CallError::Other(message)) => return Err(message),
            }
        }
    }

    if !problems.is_empty() {
        return Err(problems.join(" · "));
    }
    Err("No se pudo consultar la IA".into())
}

fn set_cooldown(state: &State<'_, AppState>, id: &str, seconds: f64) {
    if let Ok(mut map) = state.ai_cooldowns.lock() {
        map.insert(id.to_string(), Instant::now() + Duration::from_secs_f64(seconds.max(1.0)));
    }
}

fn clear_cooldown(state: &State<'_, AppState>, id: &str) {
    if let Ok(mut map) = state.ai_cooldowns.lock() {
        map.remove(id);
    }
}

/// Checks one saved key with a single-token request, so the panel can show
/// whether it answers, is throttled or is rejected.
#[tauri::command]
async fn ai_test_key(state: State<'_, AppState>, id: String) -> Result<ai::KeyCheck, String> {
    let Some((keys, model)) = ai::load(&state.root) else {
        return Err("Añade al menos una clave de Groq.".into());
    };
    let key = keys
        .into_iter()
        .find(|key| key.id == id)
        .ok_or_else(|| "Esa clave ya no existe".to_string())?;

    let check = ai::probe_key(&ai::client()?, &key, &model).await;
    match check.status.as_str() {
        "limited" => set_cooldown(&state, &id, check.wait),
        "ok" => clear_cooldown(&state, &id),
        _ => {}
    }
    Ok(check)
}

#[tauri::command]
fn delete_stickers(state: State<'_, AppState>, ids: Vec<String>) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let mut removed = 0usize;
    for id in ids {
        if let Some(file_name) = db::remove(&conn, &id)? {
            let _ = fs::remove_file(state.root.join("files").join(file_name));
            let _ = fs::remove_file(state.root.join("thumbs").join(format!("{id}.png")));
            let _ = fs::remove_file(state.root.join("thumbs").join(format!("{id}.jpg")));
            removed += 1;
        }
    }
    Ok(removed)
}

#[tauri::command]
fn library_info(state: State<'_, AppState>) -> Result<LibraryInfo, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let (count, bytes) = db::totals(&conn)?;
    Ok(LibraryInfo {
        root: state.root.to_string_lossy().to_string(),
        count,
        bytes,
    })
}

#[tauri::command]
async fn sticker_contour(
    path: String,
    width_mm: f64,
    height_mm: f64,
    padding_mm: f64,
) -> Result<Vec<Vec<(f64, f64)>>, String> {
    // Tracing costs ~12 ms per size: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let shape = contour::shape_from_file(&PathBuf::from(path), width_mm, height_mm, padding_mm)?;
        Ok(shape.contours)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Fits the whole sheet with each sticker's real silhouette. Tracing and
/// placing are heavy, so both run off the UI thread.
#[tauri::command]
async fn nest_sheet(
    copies: Vec<nest::NestCopy>,
    settings: nest::NestSettings,
) -> Result<nest::NestResult, String> {
    tauri::async_runtime::spawn_blocking(move || nest::nest(&copies, &settings))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfExportItem {
    path: String,
    x_mm: f64,
    y_mm: f64,
    rotation: f64,
    art_x_mm: f64,
    art_y_mm: f64,
    art_w_mm: f64,
    art_h_mm: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfExportPage {
    width_mm: f64,
    height_mm: f64,
    items: Vec<PdfExportItem>,
}

#[tauri::command]
fn export_sheet_pdf(
    path: String,
    pages: Vec<PdfExportPage>,
    cut_lines: bool,
    outline_width_mm: f64,
    outline_padding_mm: f64,
) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("Falta la ruta de destino del PDF".into());
    }
    if pages.is_empty() {
        return Err("No hay copias colocadas para exportar".into());
    }

    let specs: Vec<pdf::PageSpec> = pages
        .into_iter()
        .map(|page| pdf::PageSpec {
            width_mm: page.width_mm,
            height_mm: page.height_mm,
            items: page
                .items
                .into_iter()
                .map(|item| pdf::Item {
                    path: PathBuf::from(item.path),
                    x_mm: item.x_mm,
                    y_mm: item.y_mm,
                    rotation: item.rotation,
                    art_x_mm: item.art_x_mm,
                    art_y_mm: item.art_y_mm,
                    art_w_mm: item.art_w_mm,
                    art_h_mm: item.art_h_mm,
                })
                .collect(),
        })
        .collect();

    let outline = cut_lines.then(|| pdf::Outline {
        width_mm: outline_width_mm.clamp(0.05, 5.0),
        padding_mm: outline_padding_mm.clamp(0.0, 30.0),
    });
    let dest = PathBuf::from(&path);
    pdf::export(&dest, &specs, outline.as_ref())?;
    Ok(dest.to_string_lossy().to_string())
}

/// Writes the sheet as one `.sns` file (a ZIP holding the artwork and
/// `hoja.json`), so it can be reopened and printed later instead of a PDF.
#[tauri::command]
fn save_sheet_project(
    destination: String,
    settings: project::ProjectSettings,
    copies: Vec<project::SaveCopy>,
) -> Result<String, String> {
    if destination.trim().is_empty() {
        return Err("Falta el nombre del archivo de la hoja".into());
    }
    project::save(PathBuf::from(&destination).as_path(), settings, copies)
        .map(|sheet| sheet.to_string_lossy().to_string())
}

/// Reopens a saved sheet from its `.sns` file. The archive is unpacked inside
/// the app cache, and that folder is added to the asset protocol scope so the
/// previews can be loaded from the webview.
#[tauri::command]
fn open_sheet_project(
    app: tauri::AppHandle,
    path: String,
) -> Result<project::OpenedProject, String> {
    let cache = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No se pudo preparar la caché de la app: {e}"))?
        .join("projects");
    fs::create_dir_all(&cache)
        .map_err(|e| format!("No se pudo preparar la caché de la app: {e}"))?;
    let opened = project::open(PathBuf::from(&path).as_path(), &cache)?;
    app.asset_protocol_scope()
        .allow_directory(PathBuf::from(&opened.folder), true)
        .map_err(|e| format!("No se pudo leer la hoja guardada: {e}"))?;
    Ok(opened)
}

/// Sheets live in the library database, so they reopen without a file to keep
/// track of. The manifest is the same JSON a `.sns` carries.
#[tauri::command]
fn list_saved_sheets(state: State<'_, AppState>) -> Result<Vec<db::SavedSheet>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::list_sheets(&conn)
}

#[tauri::command]
fn save_saved_sheet(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    payload: String,
) -> Result<db::SavedSheet, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::save_sheet(&conn, id.as_deref(), &name, &payload)
}

#[tauri::command]
fn open_saved_sheet(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::open_sheet(&conn, &id)
}

#[tauri::command]
fn rename_saved_sheet(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<db::SavedSheet, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::rename_sheet(&conn, &id, &name)
}

#[tauri::command]
fn delete_saved_sheet(state: State<'_, AppState>, id: String) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    db::delete_sheet(&conn, &id)
}

#[tauri::command]
fn reveal_library(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&state.root)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = &state;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            fs::create_dir_all(root.join("files"))?;
            fs::create_dir_all(root.join("thumbs"))?;
            let conn = Connection::open(root.join("library.db"))?;
            db::init(&conn)?;
            app.manage(AppState {
                conn: Mutex::new(conn),
                root,
                ai_cursor: Mutex::new(0),
                ai_cooldowns: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            import_paths,
            list_stickers,
            list_tags,
            list_series,
            list_tipos,
            set_sticker_classification,
            set_stickers_classification,
            merge_series,
            export_library,
            inspect_library_archive,
            import_library_archive,
            ai_status,
            ai_add_key,
            ai_update_key,
            ai_remove_key,
            ai_reorder_keys,
            ai_describe,
            ai_classify,
            ai_test_key,
            update_sticker,
            set_sticker_tags,
            delete_stickers,
            library_info,
            sticker_contour,
            nest_sheet,
            export_sheet_pdf,
            save_sheet_project,
            open_sheet_project,
            list_saved_sheets,
            save_saved_sheet,
            open_saved_sheet,
            rename_saved_sheet,
            delete_saved_sheet,
            reveal_library
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar Sticker Nesting Studio");
}
