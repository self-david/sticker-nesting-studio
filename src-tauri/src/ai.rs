use crate::db::AiSource;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Groq's vision model: the cheapest one in the catalogue that reads images.
pub const DEFAULT_MODEL: &str = "qwen/qwen3.8-27b";
/// Reading names, series and types needs no eyes. A small text model costs about
/// a tenth of the vision one per token, so classification has its own model.
pub const TEXT_MODEL: &str = "openai/gpt-oss-20b";
const ENDPOINT: &str = "https://api.groq.com/openai/v1/chat/completions";
const MAX_NAME_CHARS: usize = 60;
const MAX_SERIE_CHARS: usize = 60;
const MAX_TEXTO_CHARS: usize = 200;
const MAX_KEY_NAME_CHARS: usize = 40;

/// The closed list of families a sticker can belong to. A model answer outside
/// this list never creates a new family: it falls back to `otros`.
pub const TIPOS: [&str; 11] = [
    "anime",
    "comic",
    "cartoon",
    "videojuego",
    "cine",
    "terror",
    "fantasia",
    "kawaii",
    "cotidiano",
    "frase",
    "otros",
];

pub const SYSTEM: &str = "Eres un catalogador de pegatinas para una biblioteca personal. \
Muchas son personajes de películas, series, anime, videojuegos y cómics. \
Miras la imagen y respondes solo con JSON válido.";

const PROMPT: &str = "Mira este sticker.\n\
Si reconoces a un personaje conocido (película, serie, anime, videojuego, cómic), nómbralo con su nombre propio \
(«Sombrerero loco», «Hello Kitty», «Pikachu») y pon la obra entre las etiquetas.\n\
Si no lo reconoces, describe lo que se ve.\n\
Devuelve SOLO este JSON: {\"name\": \"...\", \"serie\": \"...\", \"tipo\": \"...\", \"tags\": [\"...\"], \"texto\": \"...\"}.\n\
- name: de 2 a 5 palabras en español. Personaje reconocido: su nombre. Si no, qué se ve. Sin comillas ni emojis.\n\
- texto: si la imagen lleva texto impreso, cópialo tal cual, en una sola línea y sin comillas; cadena vacía si no hay. \
Si el sticker es sobre todo tipografía, usa ese texto como name.\n\
- serie: la obra o franquicia a la que pertenece (película, serie, anime, videojuego, cómic), con su forma más conocida. \
Cadena vacía si no la reconoces con seguridad; no la inventes.\n\
- tipo: exactamente uno de: anime, comic, cartoon, videojuego, cine, terror, fantasia, kawaii, cotidiano, frase, otros.\n\
- tags: de 3 a 6 etiquetas en español, en minúsculas: tema, objeto, animal, estilo o color dominante. No repitas la serie entre las etiquetas.";

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
    /// The vision model: names and tags from the artwork.
    pub model: String,
    /// The text model: series and types from names and tags only.
    pub text_model: String,
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
    #[serde(default)]
    text_model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub name: String,
    /// The work or franchise, already cleaned: empty when the model is not sure.
    #[serde(default)]
    pub serie: String,
    /// One of `TIPOS`, or empty when the model left it out.
    #[serde(default)]
    pub tipo: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Words printed on the artwork, as the model read them ("" = none).
    #[serde(default)]
    pub texto: String,
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
    /// Stickers answered from the cache instead of the API.
    pub cached: usize,
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
            text_model: TEXT_MODEL.to_string(),
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Store {
            keys: Vec::new(),
            model: DEFAULT_MODEL.to_string(),
            text_model: TEXT_MODEL.to_string(),
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
            text_model: value
                .get("textModel")
                .and_then(|m| m.as_str())
                .unwrap_or(TEXT_MODEL)
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
    // Libraries saved before the split keep the vision model and gain the text one.
    if store.text_model.trim().is_empty() {
        store.text_model = TEXT_MODEL.to_string();
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
        text_model: store.text_model,
    }
}

/// The keys in execution order and both models: the vision one for the artwork,
/// the text one for classification.
pub fn load(root: &Path) -> Option<(Vec<StoredKey>, String, String)> {
    let store = read_store(root);
    if store.keys.is_empty() {
        None
    } else {
        Some((store.keys, store.model, store.text_model))
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
    let (rgba, width, height) = crate::contour::load_rgba(path, crate::contour::MAX_SIDE)?;
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

/// One request: who the model is, what it gets and how much it may answer.
pub struct Chat<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub content: serde_json::Value,
    pub max_tokens: u32,
}

/// The user content for the vision call: the instructions plus the artwork.
pub fn vision_content(image: &Prepared) -> serde_json::Value {
    serde_json::json!([
        { "type": "text", "text": PROMPT },
        { "type": "image_url", "image_url": { "url": image.url.clone() } }
    ])
}

/// Sends one chat to Groq with one key and returns the raw content it wrote.
pub async fn complete(
    client: &reqwest::Client,
    key: &StoredKey,
    chat: &Chat<'_>,
) -> Result<String, CallError> {
    let body = serde_json::json!({
        "model": chat.model,
        "temperature": 0.2,
        "max_tokens": chat.max_tokens,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": chat.system },
            { "role": "user", "content": chat.content }
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
        .ok_or_else(|| CallError::Other("Groq no devolvió ninguna respuesta".to_string()))?;
    Ok(content.to_string())
}

/// The cleaned name, series and type for one sticker, from the raw answer.
pub fn answer_from(raw: &str) -> Result<Answer, String> {
    let answer =
        parse_answer(raw).ok_or_else(|| "La IA no devolvió un nombre válido".to_string())?;
    let name = clean_name(&answer.name)
        .ok_or_else(|| "La IA no devolvió un nombre válido".to_string())?;
    Ok(Answer {
        name,
        serie: clean_serie(&answer.serie).unwrap_or_default(),
        tipo: clean_tipo(&answer.tipo),
        tags: answer.tags,
        texto: clean_texto(&answer.texto).unwrap_or_default(),
    })
}

// ---------- series and types, without reading the image again ----------

/// How many stickers one classification request carries.
pub const CLASSIFY_BATCH: usize = 25;
pub const CLASSIFY_MAX_TOKENS: u32 = 2048;
pub const CLASSIFY_SYSTEM: &str = "Eres un catalogador de pegatinas para una biblioteca \
personal. Clasificas cada sticker por la obra a la que pertenece (serie) y por su familia (tipo). \
Respondes solo con JSON válido.";

/// One sticker as the classifier sees it: text only, never the artwork.
pub struct ClassifyInput {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub id: String,
    pub serie: String,
    pub tipo: String,
}

/// The user content for one batch: the rules, the series that already exist
/// (so the model reuses their names) and the list of stickers.
pub fn classify_content(series: &[String], batch: &[ClassifyInput]) -> serde_json::Value {
    let mut text = String::from("Clasifica estos stickers.\n\n");
    if !series.is_empty() {
        text.push_str("Series que ya existen en la biblioteca; si el sticker encaja con una, \
usa ese nombre EXACTAMENTE:\n");
        for serie in series {
            text.push_str("- ");
            text.push_str(serie);
            text.push('\n');
        }
        text.push('\n');
    }
    text.push_str(&format!(
        "serie: la obra o franquicia (película, serie, anime, videojuego, cómic), con su forma \
más conocida y en español cuando exista. Cadena vacía si no la reconoces con seguridad; no la \
inventes.\ntipo: exactamente uno de: {}.\n\nStickers:\n",
        TIPOS.join(", ")
    ));
    for (index, item) in batch.iter().enumerate() {
        text.push_str(&format!(
            "{}. id={} | nombre: {} | etiquetas: {}\n",
            index + 1,
            item.id,
            item.name,
            item.tags.join(", ")
        ));
    }
    text.push_str(
        "\nDevuelve SOLO este JSON: {\"items\":[{\"id\":\"...\",\"serie\":\"...\",\"tipo\":\"...\"}]}",
    );
    serde_json::json!([{ "type": "text", "text": text }])
}

#[derive(Debug, Default, Deserialize)]
struct WireItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    serie: String,
    #[serde(default)]
    tipo: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireAnswer {
    #[serde(default)]
    items: Vec<WireItem>,
}

/// Keeps only the stickers that were in the batch, first answer wins, and runs
/// every field through the same cleaning as the vision answers.
pub fn parse_classifications(
    raw: &str,
    wanted: &[String],
) -> Result<Vec<Classification>, String> {
    let trimmed = raw.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| "La IA no devolvió JSON".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "La IA no devolvió JSON".to_string())?;
    if end <= start {
        return Err("La IA no devolvió JSON".to_string());
    }
    let wire: WireAnswer = serde_json::from_str(&trimmed[start..=end])
        .map_err(|e| format!("Respuesta ilegible de la IA: {e}"))?;

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in wire.items {
        if !wanted.iter().any(|id| id == &item.id) || !seen.insert(item.id.clone()) {
            continue;
        }
        out.push(Classification {
            serie: clean_serie(&item.serie).unwrap_or_default(),
            tipo: clean_tipo(&item.tipo),
            id: item.id,
        });
    }
    Ok(out)
}

// ---------- one vocabulary, without reading a single image ----------

/// Both passes answer with a short list of groups.
pub const SUGGEST_MAX_TOKENS: u32 = 1024;
pub const TAGS_SYSTEM: &str = "Eres un catalogador de pegatinas para una biblioteca \
personal. Unificas el vocabulario de las etiquetas: agrupas las que significan lo mismo y eliges \
la forma más clara en español. Respondes solo con JSON válido.";

/// One canonical name (a tag, a series) and the spellings it absorbs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeGroup {
    pub canonico: String,
    pub variantes: Vec<String>,
}

const TAG_RULES: &str = "Estas son todas las etiquetas de la biblioteca de stickers.\n\
Agrupa SOLO las que signifiquen lo mismo en este catálogo: sinónimos («perro» / «can»), \
plurales («perritos» / «perro»), diminutivos, erratas y variantes de escritura.\n\
No agrupes por parecido vago ni por tema: «gato» y «perro» son animales, no sinónimos.\n\
El campo canonico debe ser EXACTAMENTE una de las etiquetas de la lista: la forma más general \
y clara. Las variantes son las demás etiquetas de la lista que deben pasar a esa.\n\
Si una etiqueta no tiene sinónimos, no la devuelvas.\n\nEtiquetas:";

/// The user content for either pass: the rules, the whole list and the shape of
/// the answer. Only the rules change between tags and series.
fn vocabulary_content(rules: &str, names: &[String]) -> serde_json::Value {
    let mut text = String::from(rules);
    for name in names {
        text.push_str("\n- ");
        text.push_str(name);
    }
    text.push_str(
        "\n\nDevuelve SOLO este JSON: {\"grupos\":[{\"canonico\":\"...\",\"variantes\":[\"...\"]}]}",
    );
    serde_json::json!([{ "type": "text", "text": text }])
}

pub fn tags_content(tags: &[String]) -> serde_json::Value {
    vocabulary_content(TAG_RULES, tags)
}

#[derive(Debug, Default, Deserialize)]
struct WireTagGroup {
    #[serde(default)]
    canonico: String,
    #[serde(default)]
    variantes: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct WireTags {
    #[serde(default)]
    grupos: Vec<WireTagGroup>,
}

/// The stored spelling of a name the model gave, matched without case and, as a
/// second chance, without diacritics. `None` when it is not in the library.
fn real_name<'a>(known: &'a [String], raw: &str) -> Option<&'a String> {
    let wanted = raw.trim().to_lowercase();
    if let Some(found) = known.iter().find(|name| **name == wanted) {
        return Some(found);
    }
    // «Pokémon» and «pokemon» are the same word but not the same spelling: the
    // case-insensitive pass keeps the accented one when both exist.
    if let Some(found) = known.iter().find(|name| name.to_lowercase() == wanted) {
        return Some(found);
    }
    let folded = fold(&wanted);
    known.iter().find(|name| fold(name) == folded)
}

/// Keeps only groups whose canonical form and variants already exist and each
/// variant in one group only: the model may propose, never invent a name.
pub fn parse_merge_groups(raw: &str, known: &[String]) -> Result<Vec<MergeGroup>, String> {
    let trimmed = raw.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| "La IA no devolvió JSON".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "La IA no devolvió JSON".to_string())?;
    if end <= start {
        return Err("La IA no devolvió JSON".to_string());
    }
    let wire: WireTags = serde_json::from_str(&trimmed[start..=end])
        .map_err(|e| format!("Respuesta ilegible de la IA: {e}"))?;

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut groups = Vec::new();
    for group in wire.grupos {
        let Some(canonico) = real_name(known, &group.canonico) else {
            continue;
        };
        if used.contains(canonico) {
            continue;
        }
        let mut variantes: Vec<String> = Vec::new();
        for raw in &group.variantes {
            let Some(tag) = real_name(known, raw) else {
                continue;
            };
            if tag == canonico || used.contains(tag) || variantes.contains(tag) {
                continue;
            }
            variantes.push(tag.clone());
        }
        if variantes.is_empty() {
            continue;
        }
        used.insert(canonico.clone());
        for variante in &variantes {
            used.insert(variante.clone());
        }
        groups.push(MergeGroup {
            canonico: canonico.clone(),
            variantes,
        });
    }
    Ok(groups)
}

// ---------- series, one work under one spelling ----------

pub const SERIES_SYSTEM: &str = "Eres un catalogador de pegatinas para una biblioteca \
personal. Unificas los nombres de las obras y franquicias de las que salen los stickers. \
Respondes solo con JSON válido.";

const SERIES_RULES: &str = "Estas son todas las series de la biblioteca, de la más usada a la \
menos usada.\n\
Agrupa SOLO las que sean la misma obra escrita de distinta forma: mayúsculas, tildes, \
erratas y el nombre corto frente al completo («Pokemon» / «Pokémon» / «Pókemon»).\n\
No unas obras distintas, ni una saga con su secuela o su precuela («Toy Story» y \
«Toy Story 2» son dos series diferentes).\n\
El campo canonico debe ser EXACTAMENTE una de las series de la lista: la forma más usada \
o la más conocida. Las variantes son las demás series de la lista que deben pasar a esa.\n\
Si una serie no tiene variantes, no la devuelvas.\n\nSeries:";

pub fn series_content(series: &[String]) -> serde_json::Value {
    vocabulary_content(SERIES_RULES, series)
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

/// Lowercase without diacritics, so «Animación» and «animacion» are the same.
fn fold(value: &str) -> String {
    let plain: String = value
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The printed text is kept on one line, without quotes, and short enough to be
/// a column rather than a paragraph; `None` when the model read nothing.
pub fn clean_texto(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c| c == '"' || c == '\'' || c == '«' || c == '»')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    Some(
        cleaned
            .chars()
            .take(MAX_TEXTO_CHARS)
            .collect::<String>()
            .trim()
            .to_string(),
    )
}

/// The series is a plain canonical name: no quotes, no stray spaces, no control
/// characters, and `None` when the model had nothing to say.
pub fn clean_serie(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c| c == '"' || c == '\'' || c == '«' || c == '»')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.chars().take(MAX_SERIE_CHARS).collect::<String>().trim().to_string())
}

/// Reads the model's word for the family and fits it into `TIPOS`. Empty stays
/// empty (the sticker can be classified later); anything unknown becomes `otros`.
pub fn clean_tipo(raw: &str) -> String {
    let folded = fold(raw);
    if folded.is_empty() {
        return String::new();
    }
    let known = match folded.as_str() {
        "anime" | "animacion japonesa" | "manga" => "anime",
        "comic" | "comics" | "tebeo" | "superheroes" => "comic",
        "cartoon" | "dibujos animados" | "animacion" => "cartoon",
        "videojuego" | "videojuegos" | "juego" => "videojuego",
        "cine" | "pelicula" | "live action" | "accion real" => "cine",
        "terror" | "horror" | "gotico" => "terror",
        "fantasia" | "mitologia" => "fantasia",
        "kawaii" | "chibi" | "monoso" => "kawaii",
        "cotidiano" | "objeto" | "comida" | "animales" => "cotidiano",
        "frase" | "tipografia" | "lettering" | "texto" => "frase",
        "otros" => "otros",
        _ => return "otros".to_string(),
    };
    // `TIPOS` is the single source of truth; the aliases above must land on it.
    if TIPOS.contains(&known) {
        known.to_string()
    } else {
        "otros".to_string()
    }
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
        assert!(fenced.serie.is_empty() && fenced.tipo.is_empty());
        assert_eq!(fenced.name, "Hello Kitty");

        assert!(parse_answer("no tengo ni idea").is_none());
    }

    fn batch() -> Vec<ClassifyInput> {
        vec![
            ClassifyInput {
                id: "a".into(),
                name: "Pikachu".into(),
                tags: vec!["amarillo".into()],
            },
            ClassifyInput {
                id: "b".into(),
                name: "Mandala".into(),
                tags: vec!["flores".into()],
            },
        ]
    }

    #[test]
    fn the_batch_prompt_carries_the_rules_and_the_stickers() {
        let content = classify_content(&["Pokémon".into()], &batch());
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("Pokémon"), "falta la serie conocida");
        for tipo in TIPOS {
            assert!(text.contains(tipo), "el lote no menciona «{tipo}»");
        }
        assert!(text.contains("id=a") && text.contains("Pikachu"));
        assert!(text.contains("id=b") && text.contains("Mandala"));
    }

    #[test]
    fn classifications_keep_only_known_ids_and_clean_fields() {
        let raw = "```json\n{\"items\":[\n  {\"id\":\"a\",\"serie\":\" «Pokémon» \",\"tipo\":\"Animación\"},\n  {\"id\":\"b\",\"serie\":\"\",\"tipo\":\"steampunk\"},\n  {\"id\":\"zz\",\"serie\":\"Otro\",\"tipo\":\"anime\"},\n  {\"id\":\"a\",\"serie\":\"Duplicado\",\"tipo\":\"cine\"}\n]}\n```";
        let wanted = vec!["a".to_string(), "b".to_string()];
        let items = parse_classifications(raw, &wanted).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].serie, "Pokémon");
        assert_eq!(items[0].tipo, "cartoon");
        assert_eq!(items[1].serie, "");
        assert_eq!(items[1].tipo, "otros");
    }

    #[test]
    fn a_broken_batch_answer_is_an_error_not_a_panic() {
        assert!(parse_classifications("lo siento", &[]).is_err());
        assert!(parse_classifications("{", &[]).is_err());
        assert_eq!(parse_classifications("{\"items\":[]}", &[]).unwrap().len(), 0);
    }

    #[test]
    fn the_prompt_offers_every_known_tipo() {
        for tipo in TIPOS {
            assert!(PROMPT.contains(tipo), "el prompt no menciona «{tipo}»");
        }
    }

    #[test]
    fn tipos_are_folded_into_the_closed_list() {
        assert_eq!(clean_tipo("Animación"), "cartoon");
        assert_eq!(clean_tipo("  ANIME "), "anime");
        assert_eq!(clean_tipo("horror"), "terror");
        assert_eq!(clean_tipo("Kawaii"), "kawaii");
        assert_eq!(clean_tipo("steampunk"), "otros");
        assert_eq!(clean_tipo(""), "");
        assert_eq!(clean_tipo("   "), "");
    }

    #[test]
    fn series_names_are_trimmed_and_never_empty_strings() {
        assert_eq!(clean_serie("  «Pokémon»  ").unwrap(), "Pokémon");
        assert_eq!(clean_serie("Looney   Tunes").unwrap(), "Looney Tunes");
        assert_eq!(clean_serie("   "), None);
        assert_eq!(clean_serie("\"\""), None);
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
        let (keys, _, _) = load(&dir).unwrap();
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

    #[test]
    fn the_vision_prompt_asks_for_the_printed_text() {
        assert!(PROMPT.contains("\"texto\""), "el JSON no pide texto");
        assert!(PROMPT.contains("texto impreso"), "falta la regla del OCR");
        assert!(
            PROMPT.contains("usa ese texto como name"),
            "el OCR no mejora el nombre de un sticker tipográfico"
        );
    }

    #[test]
    fn the_printed_text_comes_back_clean_and_on_one_line() {
        let raw = r#"{"name":"Tarta","tags":["postre"],"texto":"  «FELIZ\nCUMPLEAÑOS»  "}"#;
        let answer = answer_from(raw).unwrap();
        assert_eq!(answer.texto, "FELIZ CUMPLEAÑOS");
        assert!(answer_from(r#"{"name":"Tarta"}"#).unwrap().texto.is_empty());
        assert_eq!(clean_texto("   "), None);
        assert_eq!(
            clean_texto(&"x".repeat(300)).unwrap().chars().count(),
            MAX_TEXTO_CHARS
        );
    }

    #[test]
    fn vision_and_text_models_have_their_own_defaults() {
        let dir = temp_dir("models");
        let fresh = status(&dir);
        assert_eq!(fresh.model, DEFAULT_MODEL);
        assert_eq!(fresh.text_model, TEXT_MODEL);

        fs::write(
            config_path(&dir),
            r#"{"keys":[],"model":"vision/x","textModel":"text/y"}"#,
        )
        .unwrap();
        let saved = status(&dir);
        assert_eq!(saved.model, "vision/x");
        assert_eq!(saved.text_model, "text/y");

        // A store from before the split keeps its vision model and gains the text one.
        fs::write(config_path(&dir), r#"{"keys":[],"model":"vision/x"}"#).unwrap();
        assert_eq!(status(&dir).text_model, TEXT_MODEL);
        let _ = fs::remove_dir_all(&dir);
    }

    fn vocabulary() -> Vec<String> {
        vec![
            "perro".into(),
            "perritos".into(),
            "can".into(),
            "gato".into(),
            "pokemon".into(),
            "pokémon".into(),
        ]
    }

    #[test]
    fn the_tag_prompt_lists_the_vocabulary_and_the_rules() {
        let content = tags_content(&vocabulary());
        let text = content[0]["text"].as_str().unwrap();
        for tag in vocabulary() {
            assert!(text.contains(&tag), "el prompt no menciona «{tag}»");
        }
        assert!(text.contains("canonico"), "el JSON no pide la forma canónica");
        assert!(text.contains("\"grupos\""), "el JSON no pide grupos");
        assert!(text.contains("No agrupes"), "falta el límite de los sinónimos");
    }

    #[test]
    fn tag_groups_only_use_tags_that_exist_and_only_once() {
        let raw = r#"{"grupos":[
            {"canonico":"perro","variantes":["perritos","can","Perro","inventada"]},
            {"canonico":"gato","variantes":["perritos"]},
            {"canonico":"dragon","variantes":["gato"]},
            {"canonico":"pokemon","variantes":["pokémon"]}
        ]}"#;
        let groups = parse_merge_groups(raw, &vocabulary()).unwrap();
        assert_eq!(groups.len(), 2, "un grupo repetido o inventado se cuela");
        assert_eq!(groups[0].canonico, "perro");
        assert_eq!(groups[0].variantes, vec!["perritos", "can"]);
        assert_eq!(groups[1].canonico, "pokemon");
        assert_eq!(groups[1].variantes, vec!["pokémon"]);

        // Nothing to merge is not an error.
        assert!(parse_merge_groups(r#"{"grupos":[]}"#, &vocabulary())
            .unwrap()
            .is_empty());
        assert!(parse_merge_groups("lo siento", &vocabulary()).is_err());
    }

    #[test]
    fn the_series_prompt_carries_the_list_and_the_limits() {
        let series = vec![
            "Pokemon".to_string(),
            "Pokémon".to_string(),
            "Toy Story".to_string(),
        ];
        let content = series_content(&series);
        let text = content[0]["text"].as_str().unwrap();
        for serie in &series {
            assert!(text.contains(serie), "el prompt no menciona «{serie}»");
        }
        assert!(text.contains("canonico"), "el JSON no pide la forma canónica");
        assert!(text.contains("secuela"), "falta el límite de las secuelas");

        // The same parser as the tags, with series names.
        let raw = r#"{"grupos":[{"canonico":"Pokémon","variantes":["Pokemon","Pókemon"]}]}"#;
        let groups = parse_merge_groups(raw, &series).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].canonico, "Pokémon");
        assert_eq!(groups[0].variantes, vec!["Pokemon"]);
    }
}
