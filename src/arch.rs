use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Companion {
    pub name: String,
    pub gb: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Arch {
    pub key: String,
    pub tier: u32,
    pub family: String,
    #[serde(rename = "match")]
    pub match_tokens: Vec<String>,
    pub comfy: Option<bool>,
    #[serde(default)]
    pub bundled: bool,
    pub params_b: f64,
    #[serde(default)]
    pub companions: Vec<Companion>,
    pub native_res: [u32; 2],
    pub steps: u32,
    pub cfg: f64,
    pub sampler: String,
    pub k_sec_per_step: f64,
    #[serde(default)]
    pub measured: bool,
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
struct Registry {
    families: Vec<Arch>,
}

static REGISTRY: OnceLock<Vec<Arch>> = OnceLock::new();
static UNKNOWN: OnceLock<Arch> = OnceLock::new();

pub fn registry() -> &'static [Arch] {
    REGISTRY.get_or_init(|| {
        let registry: Registry = serde_json::from_str(include_str!("../data/architectures.json"))
            .expect("data/architectures.json is embedded and must stay valid");
        registry.families
    })
}

fn unknown() -> &'static Arch {
    UNKNOWN.get_or_init(|| Arch {
        key: "unknown".into(),
        tier: 0,
        family: "Unknown".into(),
        match_tokens: Vec::new(),
        comfy: None,
        bundled: false,
        params_b: 0.0,
        companions: Vec::new(),
        native_res: [1024, 1024],
        steps: 25,
        cfg: 5.0,
        sampler: "euler".into(),
        k_sec_per_step: 3.0,
        measured: false,
        notes: "No registry entry for this base model. Estimates use file size only.".into(),
    })
}

pub fn match_base(base_model: &str) -> &'static Arch {
    let needle = base_model.trim().to_lowercase();
    if needle.is_empty() {
        return unknown();
    }
    registry()
        .iter()
        .find(|arch| {
            arch.match_tokens
                .iter()
                .any(|token| !token.is_empty() && needle.contains(&token.to_lowercase()))
        })
        .unwrap_or_else(|| unknown())
}

pub fn tier_label(tier: u32) -> &'static str {
    match tier {
        5 => "frontier",
        4 => "modern",
        3 => "SDXL-class",
        1 | 2 => "legacy",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_sdxl_before_generic_tokens() {
        assert_eq!(match_base("SDXL 1.0").key, "sdxl");
        assert_eq!(match_base("SDXL Lightning").key, "sdxl_turbo");
        assert_eq!(match_base("Pony").key, "pony");
    }

    #[test]
    fn matches_civitai_spellings() {
        assert_eq!(match_base("ZImageTurbo").key, "z_image");
        assert_eq!(match_base("Flux.1 D").key, "flux1");
        assert_eq!(match_base("NoobAI").key, "illustrious");
        assert_eq!(match_base("SD 1.5 Hyper").key, "sd15_hyper");
        assert_eq!(match_base("Hunyuan Video").key, "unknown");
    }

    #[test]
    fn registry_is_embedded() {
        assert!(registry().len() >= 22);
        assert!(registry().iter().all(|arch| arch.native_res[0] > 0));
    }
}
