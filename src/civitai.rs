use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

pub const DEFAULT_HOST: &str = "civitai.com";
pub const HOSTS: [&str; 2] = ["civitai.com", "civitai.red"];
const USER_AGENT: &str = concat!("civitfit/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct ModelFile {
    pub name: String,
    pub size_gib: f64,
    pub precision: String,
    pub format: String,
}

impl ModelFile {
    pub fn is_weight(&self) -> bool {
        matches!(
            self.format.as_str(),
            "SafeTensor" | "PickleTensor" | "GGUF" | "Diffusers"
        ) || {
            let name = self.name.to_ascii_lowercase();
            [".safetensors", ".sft", ".ckpt", ".pt", ".pth", ".gguf"]
                .iter()
                .any(|extension| name.ends_with(extension))
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelVersion {
    pub id: u64,
    pub name: String,
    pub base_model: String,
    pub files: Vec<ModelFile>,
}

impl ModelVersion {
    pub fn with_weight_files(&self) -> Self {
        Self {
            files: self
                .files
                .iter()
                .filter(|file| file.is_weight())
                .cloned()
                .collect(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Model {
    pub id: u64,
    pub name: String,
    pub kind: String,
    pub nsfw: bool,
    pub creator: String,
    pub tags: Vec<String>,
    pub downloads: u64,
    pub thumbs: u64,
    pub versions: Vec<ModelVersion>,
    pub source: String,
    pub commercial: Vec<String>,
    pub allow_credit: bool,
    pub allow_derivatives: bool,
    pub page_url: String,
    pub created_at: String,
}

impl Model {
    pub fn primary_version(&self) -> Option<&ModelVersion> {
        self.versions.first()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawModel {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    nsfw: Option<bool>,
    #[serde(default)]
    creator: Option<RawCreator>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    stats: Option<RawStats>,
    #[serde(default)]
    model_versions: Option<Vec<RawVersion>>,
    #[serde(default)]
    allow_commercial_use: Option<Value>,
    #[serde(default)]
    allow_no_credit: Option<bool>,
    #[serde(default)]
    allow_derivatives: Option<bool>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCreator {
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStats {
    #[serde(default)]
    download_count: Option<u64>,
    #[serde(default)]
    thumbs_up_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawVersion {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    files: Option<Vec<RawFile>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    #[serde(default)]
    name: Option<String>,
    // Civitai spells this "sizeKB"; serde's camelCase would produce "sizeKb".
    #[serde(rename = "sizeKB", default)]
    size_kb: Option<f64>,
    #[serde(default)]
    metadata: Option<RawMetadata>,
}

#[derive(Debug, Deserialize)]
struct RawMetadata {
    #[serde(default)]
    fp: Option<Value>,
    #[serde(default)]
    format: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawList {
    #[serde(default)]
    items: Vec<RawModel>,
    #[serde(default)]
    metadata: Option<RawMetadataPage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetadataPage {
    #[serde(default)]
    next_cursor: Option<String>,
}

pub struct ModelPage {
    pub models: Vec<Model>,
    pub next_cursor: Option<String>,
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(text)) if !text.is_empty() => vec![text.clone()],
        _ => Vec::new(),
    }
}

fn model_from_raw(raw: RawModel, source: &str) -> Model {
    let versions = raw
        .model_versions
        .unwrap_or_default()
        .into_iter()
        .map(|version| {
            let files = version
                .files
                .unwrap_or_default()
                .into_iter()
                .map(|file| {
                    let metadata = file.metadata;
                    let precision =
                        value_to_string(metadata.as_ref().and_then(|meta| meta.fp.as_ref()))
                            .or_else(|| {
                                value_to_string(
                                    metadata.as_ref().and_then(|meta| meta.format.as_ref()),
                                )
                            })
                            .unwrap_or_else(|| "?".into());
                    let format =
                        value_to_string(metadata.as_ref().and_then(|meta| meta.format.as_ref()))
                            .unwrap_or_else(|| "?".into());
                    ModelFile {
                        name: file.name.unwrap_or_else(|| "unknown".into()),
                        size_gib: file.size_kb.unwrap_or(0.0) / 1_048_576.0,
                        precision,
                        format,
                    }
                })
                .collect();

            ModelVersion {
                id: version.id,
                name: version.name.unwrap_or_else(|| "unknown".into()),
                base_model: version.base_model.unwrap_or_else(|| "unknown".into()),
                files,
            }
        })
        .collect();

    let stats = raw.stats.unwrap_or(RawStats {
        download_count: None,
        thumbs_up_count: None,
    });

    Model {
        id: raw.id,
        name: raw.name.unwrap_or_else(|| "unknown".into()),
        kind: raw.kind.unwrap_or_else(|| "unknown".into()),
        nsfw: raw.nsfw.unwrap_or(false),
        creator: raw
            .creator
            .and_then(|creator| creator.username)
            .unwrap_or_else(|| "unknown".into()),
        tags: raw.tags.unwrap_or_default(),
        downloads: stats.download_count.unwrap_or(0),
        thumbs: stats.thumbs_up_count.unwrap_or(0),
        versions,
        source: source.into(),
        commercial: string_list(raw.allow_commercial_use.as_ref()),
        allow_credit: !raw.allow_no_credit.unwrap_or(true),
        allow_derivatives: raw.allow_derivatives.unwrap_or(false),
        page_url: format!("https://{source}/models/{}", raw.id),
        created_at: raw.created_at.unwrap_or_default(),
    }
}

fn cache_dir() -> PathBuf {
    if let Some(path) = env::var_os("CIVITFIT_CACHE") {
        return PathBuf::from(path);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cache").join("civitfit")
}

fn cache_ttl() -> u64 {
    env::var("CIVITFIT_CACHE_TTL")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3600)
}

fn cache_file(url: &str) -> PathBuf {
    // FNV-1a is enough here: the cache is local and collisions are harmless.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    cache_dir().join(format!("{hash:016x}.json"))
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub struct Client {
    agent: ureq::Agent,
    token: Option<String>,
    ttl: u64,
}

impl Client {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent,
            token: env::var("CIVITAI_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
            ttl: cache_ttl(),
        }
    }

    fn http_json(&self, url: &str) -> Result<Value> {
        let mut request = self.agent.get(url).set("Accept", "application/json");
        if let Some(token) = &self.token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }

        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                let retry_after = response.header("Retry-After").unwrap_or("60").to_string();
                match code {
                    401 | 403 => {
                        bail!("HTTP {code}: this endpoint needs a token. Set CIVITAI_TOKEN.")
                    }
                    429 => bail!("HTTP 429: rate limited by Civitai. Retry after {retry_after}s."),
                    _ => bail!("HTTP {code} for {url}"),
                }
            }
            Err(error) => bail!("network error: {error}"),
        };

        let body = response
            .into_string()
            .context("reading the Civitai response body")?;
        serde_json::from_str(&body).with_context(|| format!("invalid JSON from {url}"))
    }

    fn fetch_json(&self, url: &str) -> Result<Value> {
        let path = cache_file(url);
        if let Ok(metadata) = fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if SystemTime::now()
                    .duration_since(modified)
                    .map(|age| age.as_secs() < self.ttl)
                    .unwrap_or(false)
                {
                    if let Ok(body) = fs::read_to_string(&path) {
                        if let Ok(value) = serde_json::from_str(&body) {
                            return Ok(value);
                        }
                    }
                }
            }
        }

        let value = self.http_json(url)?;
        if fs::create_dir_all(cache_dir()).is_ok() {
            if let Ok(body) = serde_json::to_string(&value) {
                let _ = fs::write(path, body);
            }
        }
        Ok(value)
    }

    pub fn fetch_model(&self, model_id: u64, host: &str) -> Result<Model> {
        let host = if HOSTS.contains(&host) {
            host
        } else {
            DEFAULT_HOST
        };
        let url = format!("https://{host}/api/v1/models/{model_id}");
        let value = self.fetch_json(&url)?;
        let raw: RawModel = serde_json::from_value(value)
            .with_context(|| format!("unexpected model payload from {host}"))?;
        Ok(model_from_raw(raw, host))
    }

    pub fn list_models(&self, query: &ListQuery<'_>) -> Result<ModelPage> {
        let host = if HOSTS.contains(&query.host) {
            query.host
        } else {
            DEFAULT_HOST
        };
        let mut params = vec![
            ("limit".to_string(), query.limit.clamp(1, 100).to_string()),
            ("sort".to_string(), query.sort.to_string()),
            ("period".to_string(), query.period.to_string()),
            (
                "nsfw".to_string(),
                if query.nsfw { "true" } else { "false" }.to_string(),
            ),
        ];
        if let Some(text) = query.query.filter(|text| !text.is_empty()) {
            params.push(("query".to_string(), text.to_string()));
        }
        if let Some(kinds) = query.kinds.filter(|text| !text.is_empty()) {
            params.push(("types".to_string(), kinds.to_string()));
        }
        if let Some(base) = query.base_models.filter(|text| !text.is_empty()) {
            params.push(("baseModels".to_string(), base.to_string()));
        }
        if let Some(cursor) = query.cursor.filter(|text| !text.is_empty()) {
            params.push(("cursor".to_string(), cursor.to_string()));
        }

        let query_string = params
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("https://{host}/api/v1/models?{query_string}");
        let value = self.fetch_json(&url)?;
        let list: RawList = serde_json::from_value(value)
            .with_context(|| format!("unexpected model list payload from {host}"))?;
        Ok(ModelPage {
            models: list
                .items
                .into_iter()
                .map(|raw| model_from_raw(raw, host))
                .collect(),
            next_cursor: list
                .metadata
                .and_then(|metadata| metadata.next_cursor)
                .filter(|cursor| !cursor.is_empty()),
        })
    }
}

pub struct ListQuery<'a> {
    pub host: &'a str,
    pub limit: u32,
    pub query: Option<&'a str>,
    pub kinds: Option<&'a str>,
    pub base_models: Option<&'a str>,
    pub sort: &'a str,
    pub period: &'a str,
    pub nsfw: bool,
    pub cursor: Option<&'a str>,
}

pub fn parse_ref(reference: &str) -> Result<(String, u64)> {
    let text = reference.trim();
    if let Ok(id) = text.parse::<u64>() {
        return Ok((DEFAULT_HOST.into(), id));
    }

    for host in HOSTS {
        let marker = format!("{host}/models/");
        if let Some(position) = text.find(&marker) {
            let rest = &text[position + marker.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(id) = digits.parse::<u64>() {
                return Ok((host.into(), id));
            }
        }
    }

    if let Some(rest) = text.strip_prefix("models/") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(id) = digits.parse::<u64>() {
            return Ok((DEFAULT_HOST.into(), id));
        }
    }

    bail!("cannot parse Civitai reference: {reference:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs() {
        assert_eq!(parse_ref("277058").unwrap(), ("civitai.com".into(), 277058));
        assert_eq!(
            parse_ref("https://civitai.red/models/277058?foo=1").unwrap(),
            ("civitai.red".into(), 277058)
        );
        assert_eq!(
            parse_ref("https://civitai.com/models/277058/epicrealism-xl").unwrap(),
            ("civitai.com".into(), 277058)
        );
        assert!(parse_ref("not a model").is_err());
    }

    #[test]
    fn detects_weight_files() {
        let safetensors = ModelFile {
            name: "model.safetensors".into(),
            size_gib: 6.4,
            precision: "fp16".into(),
            format: "SafeTensor".into(),
        };
        let onnx = ModelFile {
            name: "face.onnx".into(),
            size_gib: 0.5,
            precision: "?".into(),
            format: "Other".into(),
        };
        assert!(safetensors.is_weight());
        assert!(!onnx.is_weight());
    }

    #[test]
    fn reads_size_kb_as_gib() {
        let payload = r#"{
            "items": [{
                "id": 1,
                "name": "Example",
                "type": "Checkpoint",
                "stats": { "downloadCount": 10, "thumbsUpCount": 2 },
                "modelVersions": [{
                    "id": 2,
                    "name": "v1",
                    "baseModel": "SDXL 1.0",
                    "files": [{
                        "name": "example.safetensors",
                        "sizeKB": 1048576,
                        "metadata": { "fp": "fp16", "format": "SafeTensor" }
                    }]
                }]
            }]
        }"#;
        let list: RawList = serde_json::from_str(payload).unwrap();
        let model = model_from_raw(list.items.into_iter().next().unwrap(), "civitai.com");
        assert_eq!(model.versions[0].files[0].size_gib, 1.0);
        assert!(model.versions[0].files[0].is_weight());
        assert_eq!(model.page_url, "https://civitai.com/models/1");
    }
}
