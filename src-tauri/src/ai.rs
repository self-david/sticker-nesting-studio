use crate::db::AiSource;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Groq's vision model: the cheapest one in the catalogue that reads images.
pub const DEFAULT_MODEL: &str = "qwen/qwen3.8-27b";
const ENDPOINT: &str = "https://api.groq.com/openai/v1/chat/completions";
const MAX_NAME_CHARS: usize = 60;
const MAX_KEY_NAME_CHARS: usize = 40;

const SYSTEM: &str = "Eres un catalogador de pegatinas para una biblioteca personal. \
Muchas son personajes de películas, series, anime, videojuegos y cómics. \
Miras la imagen y respondes solo con JSON válido.";

const PROMPT: &str = "Mira este sticker.\n\
Si reconoces a un personaje conocido (película, serie, anime, videojuego, cómic), nómbralo con su nombre propio \
(«Sombrerero loco», «Hello Kitty», «Pikachu») y pon la obra entre las etiquetas.\n\
Si no lo reconoces, describe lo que se ve.\n\
Devuelve SOLO este JSON: {\"name\": \"...\", \"tags\": [\"...\"]}.\n\
- name: de 2 a 5 palabras en español. Personaje reconocido: su nombre. Si no, qué se ve. Sin comillas ni emojis.\n\
- tags: de 3 a 6 etiquetas en español, en minúsculas: personaje, obra, tema, objeto, animal, estilo o color dominante.";

/// One saved key. The secret stays here, in Rust: the webview only sees a tail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    pub id: String,
    pub name: String,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub id: String,
    pub name: String,
    /// Last characters, so a saved key can be recognised without exposing it.
    pub hint: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// In execution order: the first one is used first.
    pub keys: Vec<KeyInfo>,
    pub model: String,
}

/// Result of checking one key on its own.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheck {
    /// "ok" | "limited" | "invalid" | "error"
    pub status: String,
    pub message: String,
    /// Seconds Groq asked to wait, when it is limited.
    pub wait: f64,
    pub ms: u64,
}

/// One sticker's image, already turned into the request payload: the same one
/// is reused by every key tried for it.
pub struct Prepared {
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default)]
    keys: Vec<StoredKey>,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub updated: Vec<String>,
    pub failures: Vec<Failure>,
}

/// Why one call did not produce an answer: the caller decides whether to move
/// on to the next key or give up on the sticker.
pub enum CallError {
    /// This key is out of tokens for now; wait that many seconds.
    Throttled(f64),
    /// The key itself is rejected (wrong, revoked, no access).
    Key(String),
    /// Network, server or a useless answer.
    Other(String),
}

fn config_path(root: &Path) -> PathBuf {
    root.join("ai.json")
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Reads the store, converting the single-key format written before.
fn read_store(root: &Path) -> Store {
    let Ok(text) = fs::read_to_string(config_path(root)) else {
        return Store {
            keys: Vec::new(),
            model: DEFAULT_MODEL.to_string(),
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Store {
            keys: Vec::new(),
            model: DEFAULT_MODEL.to_string(),
        };
    };

    let mut store: Store = if value.get("keys").is_some() {
        serde_json::from_value(value.clone()).unwrap_or_default()
    } else if let Some(old) = value.get("apiKey").and_then(|v| v.as_str()) {
        Store {
            keys: vec![StoredKey {
                id: new_id(),
                name: "Clave 1".to_string(),
                api_key: old.trim().to_string(),
            }],
            model: value
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or(DEFAULT_MODEL)
                .to_string(),
        }
    } else {
        Store::default()
    };

    store.keys.retain(|k| !k.api_key.trim().is_empty());
    for (index, key) in store.keys.iter_mut().enumerate() {
        key.name = clean_key_name(&key.name, index);
    }
    if store.model.trim().is_empty() {
        store.model = DEFAULT_MODEL.to_string();
    }
    store
}

fn write_store(root: &Path, store: &Store) -> Result<(), String> {
    let text = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(config_path(root), text).map_err(|e| format!("No se pudo guardar las claves: {e}"))
}

pub fn status(root: &Path) -> AiStatus {
    let store = read_store(root);
    AiStatus {
        keys: store
            .keys
            .iter()
            .map(|key| KeyInfo {
                id: key.id.clone(),
                name: key.name.clone(),
                hint: hint_of(&key.api_key),
            })
            .collect(),
        model: store.model,
    }
}

/// The keys in execution order, for the requests themselves.
pub fn load(root: &Path) -> Option<(Vec<StoredKey>, String)> {
    let store = read_store(root);
    if store.keys.is_empty() {
        None
    } else {
        Some((store.keys, store.model))
    }
}

fn hint_of(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

fn clean_key_name(raw: &str, index: usize) -> String {
    let name = raw
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        format!("Clave {}", index + 1)
    } else {
        name.chars().take(MAX_KEY_NAME_CHARS).collect()
    }
}

fn check_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if !key.starts_with("gsk_") || key.chars().count() < 20 {
        return Err("La clave de Groq empieza por «gsk_». Revisa que esté completa.".into());
    }
    if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("La clave no puede tener espacios ni saltos de línea.".into());
    }
    Ok(key.to_string())
}

pub fn add_key(root: &Path, name: &str, raw_key: &str) -> Result<AiStatus, String> {
    let api_key = check_key(raw_key)?;
    let mut store = read_store(root);
    if store.keys.iter().any(|k| k.api_key == api_key) {
        return Err("Esa clave ya está en la lista".into());
    }
    let index = store.keys.len();
    store.keys.push(StoredKey {
        id: new_id(),
        name: clean_key_name(name, index),
        api_key,
    });
    write_store(root, &store)?;
    Ok(status(root))
}

/// Renames a key and, when `raw_key` is given, replaces its secret.
pub fn update_key(
    root: &Path,
    id: &str,
    name: Option<&str>,
    raw_key: Option<&str>,
) -> Result<AiStatus, String> {
    let mut store = read_store(root);
    let index = store
        .keys
        .iter()
        .position(|k| k.id == id)
        .ok_or_else(|| "Esa clave ya no existe".to_string())?;
    if let Some(name) = name {
        store.keys[index].name = clean_key_name(name, index);
    }
    if let Some(raw_key) = raw_key {
        if !raw_key.trim().is_empty() {
            store.keys[index].api_key = check_key(raw_key)?;
        }
    }
    write_store(root, &store)?;
    Ok(status(root))
}

pub fn remove_key(root: &Path, id: &str) -> Result<AiStatus, String> {
    let mut store = read_store(root);
    store.keys.retain(|k| k.id != id);
    write_store(root, &store)?;
    Ok(status(root))
}

/// Saves the execution order: first id is used first.
pub fn reorder_keys(root: &Path, ids: &[String]) -> Result<AiStatus, String> {
    let mut store = read_store(root);
    let mut ordered: Vec<StoredKey> = Vec::with_capacity(store.keys.len());
    for id in ids {
        if let Some(pos) = store.keys.iter().position(|k| &k.id == id) {
            ordered.push(store.keys.remove(pos));
        }
    }
    // Anything the caller forgot keeps its place at the end.
    ordered.append(&mut store.keys);
    store.keys = ordered;
    write_store(root, &store)?;
    Ok(status(root))
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("No se pudo preparar la conexión: {e}"))
}

/// The artwork flattened onto white and re-encoded as JPEG: the models read it
/// more reliably than a transparent PNG, and it keeps the request small.
fn data_url(path: &Path) -> Result<String, String> {
    let (rgba, width, height) = crate::contour::load_rgba(path)?;
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "No se pudo preparar la vista previa".to_string())?;
    let mut flat = image::RgbImage::new(width, height);
    for (x, y, px) in img.enumerate_pixels() {
        let alpha = px[3] as u32;
        let over = |c: u8| ((c as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        flat.put_pixel(x, y, image::Rgb([over(px[0]), over(px[1]), over(px[2])]));
    }
    let mut buffer = Vec::new();
    image::DynamicImage::ImageRgb8(flat)
        .write_to(&mut Cursor::new(&mut buffer), image::ImageFormat::Jpeg)
        .map_err(|e| format!("No se pudo preparar la vista previa: {e}"))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buffer);
    Ok(format!("data:image/jpeg;base64,{encoded}"))
}

/// Reads and encodes the artwork once per sticker.
pub fn prepare(source: &AiSource) -> Result<Prepared, String> {
    Ok(Prepared {
        url: data_url(&source.path)?,
    })
}

/// Asks the model for a name and tags for one sticker, using one key.
pub async fn describe(
    client: &reqwest::Client,
    key: &StoredKey,
    model: &str,
    image: &Prepared,
) -> Result<Answer, CallError> {
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 300,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": PROMPT },
                    { "type": "image_url", "image_url": { "url": image.url.clone() } }
                ]
            }
        ]
    });

    let response = client
        .post(ENDPOINT)
        .bearer_auth(&key.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| CallError::Other(format!("No se pudo conectar con Groq: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| CallError::Other(format!("Respuesta ilegible de Groq: {e}")))?;

    if status.as_u16() == 429 {
        return Err(CallError::Throttled(
            retry_seconds(&text).unwrap_or(15.0).min(60.0),
        ));
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(CallError::Key(api_error(status.as_u16(), &text)));
    }
    if !status.is_success() {
        return Err(CallError::Other(api_error(status.as_u16(), &text)));
    }

    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| CallError::Other(format!("Respuesta ilegible de Groq: {e}")))?;
    let content = payload["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| CallError::Other("Groq no devolvió ninguna descripción".to_string()))?;

    let answer = parse_answer(content)
        .ok_or_else(|| CallError::Other("La IA no devolvió un nombre válido".to_string()))?;
    let name = clean_name(&answer.name)
        .ok_or_else(|| CallError::Other("La IA no devolvió un nombre válido".to_string()))?;
    Ok(Answer {
        name,
        tags: answer.tags,
    })
}

/// One cheap request (a single token, no image) to tell whether a key works.
pub async fn probe_key(client: &reqwest::Client, key: &StoredKey, model: &str) -> KeyCheck {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "ping" }]
    });
    let started = std::time::Instant::now();
    let response = client
        .post(ENDPOINT)
        .bearer_auth(&key.api_key)
        .json(&body)
        .send()
        .await;
    let ms = started.elapsed().as_millis() as u64;

    let response = match response {
        Ok(response) => response,
        Err(e) => {
            return KeyCheck {
                status: "error".into(),
                message: format!("No se pudo conectar con Groq: {e}"),
                wait: 0.0,
                ms,
            }
        }
    };

    let code = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let status = check_status(code);
    KeyCheck {
        status: status.into(),
        message: if status == "ok" {
            String::new()
        } else {
            api_error(code, &text)
        },
        wait: if status == "limited" {
            retry_seconds(&text).unwrap_or(15.0)
        } else {
            0.0
        },
        ms,
    }
}

fn check_status(code: u16) -> &'static str {
    match code {
        200..=299 => "ok",
        401 | 403 => "invalid",
        429 => "limited",
        _ => "error",
    }
}

/// Groq answers 429 with «Please try again in 14.32s».
pub fn retry_seconds(body: &str) -> Option<f64> {
    let start = body.find("try again in ")? + "try again in ".len();
    let rest = &body[start..];
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    rest[..end].parse::<f64>().ok()
}

fn api_error(code: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| v["message"].as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| body.chars().take(160).collect());
    match code {
        401 | 403 => format!("Groq rechazó esta clave. ({detail})"),
        429 => format!("Groq está limitando las peticiones. ({detail})"),
        _ => format!("Groq respondió {code}: {detail}"),
    }
}

/// The model sometimes wraps the JSON in fences or adds a sentence around it.
pub fn parse_answer(raw: &str) -> Option<Answer> {
    let trimmed = raw.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Answer>(&trimmed[start..=end]).ok()
}

/// Keeps the name usable as a filename and as a label in the library.
pub fn clean_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '«' || c == '»')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return None;
    }
    let short: String = cleaned.chars().take(MAX_NAME_CHARS).collect();
    Some(short.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sns-ai-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn reads_plain_and_wrapped_answers() {
        let plain = parse_answer(r#"{"name":"Gato bomba","tags":["gato","animal"]}"#).unwrap();
        assert_eq!(plain.name, "Gato bomba");
        assert_eq!(plain.tags.len(), 2);

        let fenced = parse_answer(
            "Claro:\n```json\n{\"name\": \"Hello Kitty\", \"tags\": [\"kitty\"]}\n```",
        )
        .unwrap();
        assert_eq!(fenced.name, "Hello Kitty");

        assert!(parse_answer("no tengo ni idea").is_none());
    }

    #[test]
    fn names_stay_valid_as_filenames() {
        assert_eq!(clean_name(r#"  "Gato / bomba"  "#).unwrap(), "Gato bomba");
        assert_eq!(clean_name("a\u{0}b").unwrap(), "a b");
        assert!(clean_name("   ").is_none());
        assert_eq!(
            clean_name(&"x".repeat(80)).unwrap().chars().count(),
            MAX_NAME_CHARS
        );
    }

    #[test]
    fn reads_the_wait_from_a_throttled_answer() {
        let body = r#"{"error":{"message":"Rate limit reached ... Please try again in 14.322857142s. Need more tokens?"}}"#;
        assert!((retry_seconds(body).unwrap() - 14.322857142).abs() < 1e-6);
        assert!(retry_seconds("nope").is_none());
    }

    #[test]
    fn a_check_tells_apart_the_three_states() {
        assert_eq!(check_status(200), "ok");
        assert_eq!(check_status(401), "invalid");
        assert_eq!(check_status(429), "limited");
        assert_eq!(check_status(500), "error");
    }

    #[test]
    fn hints_only_expose_the_tail() {
        assert_eq!(hint_of("gsk_abcdefghijkl"), "…ijkl");
    }

    #[test]
    fn keeps_a_list_of_named_keys_in_order() {
        let dir = temp_dir("keys");
        assert!(add_key(&dir, "corta", "nope").is_err());
        assert!(status(&dir).keys.is_empty());

        let after_first = add_key(&dir, "Trabajo", "gsk_0123456789abcdefghij").unwrap();
        assert_eq!(after_first.keys.len(), 1);
        assert_eq!(after_first.keys[0].name, "Trabajo");
        assert_eq!(after_first.keys[0].hint, "…ghij");

        let after_second = add_key(&dir, "", "gsk_zzzzzzzzzzzzzzzzzzzz").unwrap();
        assert_eq!(after_second.keys.len(), 2);
        assert_eq!(after_second.keys[1].name, "Clave 2");

        // The same key cannot be stored twice.
        assert!(add_key(&dir, "Otra", "gsk_0123456789abcdefghij").is_err());

        let ids: Vec<String> = after_second.keys.iter().map(|k| k.id.clone()).collect();
        let flipped = reorder_keys(&dir, &[ids[1].clone(), ids[0].clone()]).unwrap();
        assert_eq!(flipped.keys[0].id, ids[1]);
        let (keys, _) = load(&dir).unwrap();
        assert_eq!(keys[0].id, ids[1]);

        let renamed = update_key(&dir, &ids[1], Some("Personal"), None).unwrap();
        assert_eq!(renamed.keys[0].name, "Personal");

        let removed = remove_key(&dir, &ids[1]).unwrap();
        assert_eq!(removed.keys.len(), 1);
        assert_eq!(removed.keys[0].id, ids[0]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_the_old_single_key_file() {
        let dir = temp_dir("legacy");
        fs::write(
            config_path(&dir),
            r#"{"apiKey":"gsk_0123456789abcdefghij","model":"qwen/qwen3.8-27b"}"#,
        )
        .unwrap();
        let loaded = status(&dir);
        assert_eq!(loaded.keys.len(), 1);
        assert_eq!(loaded.keys[0].name, "Clave 1");
        assert_eq!(loaded.keys[0].hint, "…ghij");
        let _ = fs::remove_dir_all(&dir);
    }
}
