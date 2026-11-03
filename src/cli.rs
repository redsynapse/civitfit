use crate::arch::{match_base, registry, tier_label};
use crate::civitai::{parse_ref, Client, ListQuery, Model, ModelVersion};
use crate::estimate::{evaluate_version, Status, VersionEstimate};
use crate::hardware::{detect, Hardware};
use crate::render::{arch_table, detail, fit_table, FitRow};
use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::HashSet;

const SUBCOMMANDS: [&str; 6] = ["system", "fit", "info", "search", "arch", "plan"];

const VIDEO_MARKERS: [&str; 13] = [
    "hunyuan video",
    "hunyuanvideo",
    "wan video",
    "wan2",
    "wan 2",
    "ltx video",
    "ltx-video",
    "ltxv",
    "cogvideox",
    "mochi",
    "skyreels",
    "framepack",
    "seedance",
];

#[derive(Parser, Debug)]
#[command(
    name = "civitfit",
    version,
    about = "Right-size Civitai image models to your hardware",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Machine readable output
    #[arg(long, global = true)]
    pub json: bool,
    /// Override the GPU memory budget, e.g. 24G
    #[arg(long, global = true)]
    pub gpu: Option<String>,
    /// Override system RAM, e.g. 64G
    #[arg(long, global = true)]
    pub ram: Option<String>,
    #[arg(long, global = true, default_value_t = 1024)]
    pub width: u32,
    #[arg(long, global = true, default_value_t = 1024)]
    pub height: u32,
    /// Override sampler steps for the speed estimate
    #[arg(long, global = true)]
    pub steps: Option<u32>,
    /// Civitai instance to query
    #[arg(
        long,
        global = true,
        default_value = "civitai.com",
        value_parser = ["civitai.com", "civitai.red", "both"]
    )]
    pub source: String,
    /// Include NSFW results
    #[arg(long, global = true)]
    pub nsfw: bool,
    /// Show every model version in `info`
    #[arg(long, global = true)]
    pub all: bool,
    /// Include models whose base model is Other/Unknown
    #[arg(long, global = true)]
    pub include_other: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show detected hardware
    System,
    /// Browse Civitai models that fit this machine
    Fit(FitArgs),
    /// Inspect one model URL or id
    Info { reference: String },
    /// Search Civitai by name
    Search(SearchArgs),
    /// List the architecture registry
    Arch,
    /// Detailed memory plan for one model
    Plan { reference: String },
}

#[derive(Args, Debug)]
pub struct FitArgs {
    pub query: Option<String>,
    #[arg(long = "type", default_value = "Checkpoint")]
    pub kind: String,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(short = 'n', long = "top", default_value_t = 20)]
    pub top: usize,
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    #[arg(long, default_value_t = 1)]
    pub pages: u32,
    #[arg(long, value_enum, default_value = "best")]
    pub sort: SortMode,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    pub query: String,
    #[arg(long = "type", default_value = "Checkpoint")]
    pub kind: String,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(short = 'n', long = "top", default_value_t = 10)]
    pub top: usize,
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
    #[arg(long, default_value_t = 1)]
    pub pages: u32,
    #[arg(long, value_enum, default_value = "best")]
    pub sort: SortMode,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
    Best,
    Downloads,
    Newest,
    Speed,
    Memory,
}

impl std::fmt::Display for SortMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            SortMode::Best => "best",
            SortMode::Downloads => "downloads",
            SortMode::Newest => "newest",
            SortMode::Speed => "speed",
            SortMode::Memory => "memory",
        };
        formatter.write_str(text)
    }
}

struct BrowseOptions<'a> {
    query: Option<&'a str>,
    kind: &'a str,
    base: Option<&'a str>,
    top: usize,
    limit: u32,
    pages: u32,
    sort: SortMode,
}

struct Scored<'a> {
    model: &'a Model,
    estimate: VersionEstimate,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse_from(std::iter::once("civitfit".to_string()).chain(cli_args()));
    let hardware = detect(cli.ram.as_deref(), cli.gpu.as_deref())?;

    match &cli.command {
        Some(Command::System) => {
            if cli.json {
                print_json(json!({ "hardware": hardware }))?;
            } else {
                print_system(&hardware);
            }
        }
        Some(Command::Arch) => {
            if cli.json {
                let families: Vec<Value> = registry()
                    .iter()
                    .map(|arch| {
                        json!({
                            "key": arch.key,
                            "family": arch.family,
                            "tier": arch.tier,
                            "tier_label": tier_label(arch.tier),
                            "comfy": arch.comfy,
                            "bundled": arch.bundled,
                            "params_b": arch.params_b,
                            "companions": arch.companions,
                            "native_res": arch.native_res,
                            "steps": arch.steps,
                            "cfg": arch.cfg,
                            "sampler": arch.sampler,
                            "k_sec_per_step": arch.k_sec_per_step,
                            "measured": arch.measured,
                            "notes": arch.notes,
                            "match": arch.match_tokens,
                        })
                    })
                    .collect();
                print_json(Value::Array(families))?;
            } else {
                println!();
                println!("=== Architecture registry ===");
                println!();
                println!("{}", arch_table(registry()));
                println!();
                println!("  tiers: frontier > modern > SDXL-class > legacy");
                println!("  s/step is calibrated at 1024x1024 on Apple Silicon.");
                println!();
            }
        }
        Some(Command::Info { reference }) | Some(Command::Plan { reference }) => {
            cmd_info(&cli, &hardware, reference)?;
        }
        Some(Command::Fit(args)) => {
            let options = BrowseOptions {
                query: args.query.as_deref(),
                kind: &args.kind,
                base: args.base.as_deref(),
                top: args.top,
                limit: args.limit,
                pages: args.pages,
                sort: args.sort,
            };
            cmd_fit(&cli, &hardware, &options)?;
        }
        Some(Command::Search(args)) => {
            let options = BrowseOptions {
                query: Some(args.query.as_str()),
                kind: &args.kind,
                base: args.base.as_deref(),
                top: args.top,
                limit: args.limit,
                pages: args.pages,
                sort: args.sort,
            };
            cmd_fit(&cli, &hardware, &options)?;
        }
        None => unreachable!("the argument preprocessor always inserts a subcommand"),
    }

    Ok(())
}

fn cli_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|argument| SUBCOMMANDS.contains(&argument.as_str()))
    {
        return args;
    }
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--version" | "-V"))
    {
        return args;
    }

    let mut with_command = Vec::with_capacity(args.len() + 1);
    if args.iter().any(|argument| looks_like_ref(argument)) {
        with_command.push("info".to_string());
    } else {
        with_command.push("fit".to_string());
    }
    with_command.extend(args);
    with_command
}

fn looks_like_ref(text: &str) -> bool {
    text.parse::<u64>().is_ok()
        || text.contains("civitai.com/models/")
        || text.contains("civitai.red/models/")
        || text
            .strip_prefix("models/")
            .map(|rest| rest.parse::<u64>().is_ok())
            .unwrap_or(false)
}

fn print_json(value: Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn print_system(hardware: &Hardware) {
    println!();
    println!("=== System Specifications ===");
    println!("  OS:            {}", hardware.os);
    println!("  Chip:          {}", hardware.chip);
    println!("  CPU cores:     {}", hardware.cpu_cores);
    println!("  RAM:           {:.2} GiB", hardware.ram_gib);
    println!("  GPU:           {}", hardware.gpu_name);
    println!("  GPU budget:    {:.2} GiB", hardware.gpu_budget_gib);
    println!("  Backend:       {}", hardware.backend);
    println!("  Detected via:  {}", hardware.detected_with);
    println!();
    println!("  Note: the GPU budget is the macOS wired-memory limit when available,");
    println!("  otherwise 85% of RAM. Estimates below 60% of this budget are safe.");
    println!();
}

fn is_other_base(base_model: &str) -> bool {
    matches!(
        base_model.trim().to_ascii_lowercase().as_str(),
        "" | "other" | "unknown"
    )
}

fn is_video_base(base_model: &str) -> bool {
    let base = base_model.to_ascii_lowercase();
    VIDEO_MARKERS.iter().any(|marker| base.contains(marker))
}

fn base_matches(base_model: &str, filter: &str) -> bool {
    filter
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| base_model.eq_ignore_ascii_case(part))
}

fn hosts(source: &str) -> Vec<&'static str> {
    match source {
        "civitai.red" => vec!["civitai.red"],
        "both" => vec!["civitai.com", "civitai.red"],
        _ => vec!["civitai.com"],
    }
}

fn api_sort(sort: SortMode) -> &'static str {
    match sort {
        SortMode::Newest => "Newest",
        _ => "Most Downloaded",
    }
}

fn fetch_host(
    client: &Client,
    host: &str,
    options: &BrowseOptions<'_>,
    nsfw: bool,
) -> Result<Vec<Model>> {
    let limit = options.limit.clamp(1, 100);
    let pages = options.pages.clamp(1, 10);
    let base_values: Vec<&str> = options
        .base
        .map(|base| {
            base.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default();
    // Civitai does not accept comma-separated baseModels, so query each one.
    let base_queries: Vec<Option<&str>> = if base_values.is_empty() {
        vec![None]
    } else {
        base_values.into_iter().map(Some).collect()
    };
    let mut models = Vec::new();

    for base in base_queries {
        let mut cursor: Option<String> = None;
        for _ in 0..pages {
            let page = client.list_models(&ListQuery {
                host,
                limit,
                query: options.query,
                kinds: Some(options.kind),
                base_models: base,
                sort: api_sort(options.sort),
                period: "AllTime",
                nsfw,
                cursor: cursor.as_deref(),
            })?;
            let received = page.models.len();
            models.extend(page.models);
            cursor = page.next_cursor;
            if received < limit as usize || cursor.is_none() {
                break;
            }
        }
    }

    Ok(models)
}

fn quality(model: &Model) -> f64 {
    ((model.downloads as f64) + 1.0).log10() + 0.4 * ((model.thumbs as f64) + 1.0).log10()
}

fn sort_scored(scored: &mut [Scored<'_>], mode: SortMode) {
    scored.sort_by(|left, right| {
        let left_status = left.estimate.effective_status();
        let right_status = right.estimate.effective_status();
        let group = left_status.fit_group().cmp(&right_status.fit_group());
        if group != Ordering::Equal {
            return group;
        }

        let primary = match mode {
            SortMode::Best => right
                .estimate
                .arch
                .tier
                .cmp(&left.estimate.arch.tier)
                .then_with(|| {
                    quality(right.model)
                        .partial_cmp(&quality(left.model))
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| {
                    left.estimate
                        .chosen
                        .mem_ratio
                        .partial_cmp(&right.estimate.chosen.mem_ratio)
                        .unwrap_or(Ordering::Equal)
                }),
            SortMode::Downloads => right.model.downloads.cmp(&left.model.downloads),
            SortMode::Newest => right
                .model
                .created_at
                .cmp(&left.model.created_at)
                .then_with(|| right.model.downloads.cmp(&left.model.downloads)),
            SortMode::Speed => left
                .estimate
                .chosen
                .speed_s
                .partial_cmp(&right.estimate.chosen.speed_s)
                .unwrap_or(Ordering::Equal),
            SortMode::Memory => left
                .estimate
                .chosen
                .mem_ratio
                .partial_cmp(&right.estimate.chosen.mem_ratio)
                .unwrap_or(Ordering::Equal),
        };

        primary.then_with(|| {
            left.model
                .name
                .to_lowercase()
                .cmp(&right.model.name.to_lowercase())
        })
    });
}

fn cmd_fit(cli: &Cli, hardware: &Hardware, options: &BrowseOptions<'_>) -> Result<()> {
    if !options.kind.eq_ignore_ascii_case("Checkpoint") {
        bail!(
            "civitfit scores full checkpoints only; --type {} is not supported",
            options.kind
        );
    }

    let client = Client::new();
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();

    for host in hosts(&cli.source) {
        match fetch_host(&client, host, options, cli.nsfw) {
            Ok(batch) => {
                for model in batch {
                    if seen.insert(model.id) {
                        models.push(model);
                    }
                }
            }
            Err(error) => errors.push(format!("{host}: {error:#}")),
        }
    }

    if models.is_empty() {
        if errors.is_empty() {
            println!("no models returned by Civitai");
            return Ok(());
        }
        bail!("{}", errors.join("; "));
    }
    if !errors.is_empty() {
        for error in errors {
            eprintln!("warning: {error}");
        }
    }

    let mut skipped = 0usize;
    let mut scored: Vec<Scored<'_>> = Vec::new();

    for model in &models {
        if !cli.nsfw && model.nsfw {
            skipped += 1;
            continue;
        }
        if !model.kind.eq_ignore_ascii_case(options.kind) {
            skipped += 1;
            continue;
        }
        let Some(version) = model.primary_version() else {
            skipped += 1;
            continue;
        };
        if !cli.include_other && is_other_base(&version.base_model) {
            skipped += 1;
            continue;
        }
        if is_video_base(&version.base_model) {
            skipped += 1;
            continue;
        }
        if let Some(base_filter) = options.base {
            if !base_matches(&version.base_model, base_filter) {
                skipped += 1;
                continue;
            }
        }

        let weights = version.with_weight_files();
        if weights.files.is_empty() {
            skipped += 1;
            continue;
        }

        let arch = match_base(&weights.base_model);
        let estimate = evaluate_version(
            weights,
            arch,
            hardware.gpu_budget_gib,
            cli.width,
            cli.height,
            cli.steps,
        );
        scored.push(Scored { model, estimate });
    }

    sort_scored(&mut scored, options.sort);
    let shown_count = options.top.min(scored.len());
    let shown = &scored[..shown_count];

    if cli.json {
        let payload = json!({
            "hardware": hardware,
            "source": cli.source,
            "sort": options.sort.to_string(),
            "count": scored.len(),
            "shown": shown.len(),
            "skipped": skipped,
            "models": shown.iter().map(|item| estimate_json(item.model, &item.estimate)).collect::<Vec<_>>(),
        });
        return print_json(payload);
    }

    print_system(hardware);
    println!(
        "=== Civitai models that fit ===  ({} scored, showing {}, {} filtered)",
        scored.len(),
        shown.len(),
        skipped
    );
    println!(
        "  source {}  ·  sort {}  ·  {}x{}",
        cli.source, options.sort, cli.width, cli.height
    );
    println!();

    let rows: Vec<FitRow<'_>> = shown
        .iter()
        .map(|item| FitRow {
            model: item.model,
            estimate: &item.estimate,
        })
        .collect();
    println!(
        "{}",
        fit_table(&rows, hardware.gpu_budget_gib, cli.width, cli.height)
    );
    println!();
    println!("  🟢 Perfect   🟡 Tight   🟠 Marginal   🔴 Won't run   ⛔ Unsupported");
    println!("  next: civitfit info <url>  for a full breakdown");
    println!();
    Ok(())
}

fn cmd_info(cli: &Cli, hardware: &Hardware, reference: &str) -> Result<()> {
    let (host, model_id) = parse_ref(reference)?;
    let client = Client::new();
    let model = client.fetch_model(model_id, &host)?;

    let total_versions = model.versions.len();
    let selected: Vec<ModelVersion> = if cli.all {
        model.versions.clone()
    } else {
        model.versions.iter().take(3).cloned().collect()
    };

    let mut estimates = Vec::new();
    for version in selected {
        let weights = version.with_weight_files();
        let has_weights = !weights.files.is_empty();
        let evaluation_version = if has_weights { weights } else { version };
        let arch = match_base(&evaluation_version.base_model);
        let estimate = evaluate_version(
            evaluation_version,
            arch,
            hardware.gpu_budget_gib,
            cli.width,
            cli.height,
            cli.steps,
        );
        estimates.push(estimate);
    }

    let version_note = if estimates.len() < total_versions {
        format!(
            "showing {} of {} (use --all)",
            estimates.len(),
            total_versions
        )
    } else {
        String::new()
    };

    if cli.json {
        let payload = json!({
            "hardware": hardware,
            "model": {
                "id": model.id,
                "name": model.name,
                "url": model.page_url,
                "type": model.kind,
                "nsfw": model.nsfw,
                "creator": model.creator,
                "downloads": model.downloads,
                "thumbs": model.thumbs,
                "commercial": model.commercial,
                "tags": model.tags,
                "created_at": model.created_at,
            },
            "versions": estimates.iter().map(|estimate| estimate_json(&model, estimate)).collect::<Vec<_>>(),
        });
        return print_json(payload);
    }

    print_system(hardware);
    println!(
        "{}",
        detail(
            &model,
            &estimates,
            hardware.gpu_budget_gib,
            cli.width,
            cli.height,
            &version_note
        )
    );
    Ok(())
}

fn estimate_json(model: &Model, estimate: &VersionEstimate) -> Value {
    let status: Status = estimate.effective_status();
    json!({
        "model": {
            "id": model.id,
            "name": model.name,
            "source": model.source,
            "url": model.page_url,
        },
        "version": {
            "id": estimate.version.id,
            "name": estimate.version.name,
            "base_model": estimate.version.base_model,
        },
        "arch": estimate.arch.key,
        "tier": estimate.arch.tier,
        "comfy": estimate.arch.comfy,
        "bundled": estimate.bundled,
        "companions": estimate.companions.iter().map(|companion| json!({
            "name": companion.name,
            "gib": companion.gb,
        })).collect::<Vec<_>>(),
        "file": {
            "name": estimate.chosen.file.name,
            "gib": estimate.chosen.file.size_gib,
            "precision": estimate.chosen.file.precision,
        },
        "peak_gib": estimate.chosen.peak_gib,
        "mem_ratio": estimate.chosen.mem_ratio,
        "status": status,
        "loadable_weights": estimate.loadable_weights,
        "speed_s": estimate.chosen.speed_s,
        "steps": estimate.steps,
        "notes": estimate.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civitai::ModelFile;

    fn model(name: &str, base: &str, downloads: u64) -> Model {
        Model {
            id: 1,
            name: name.into(),
            kind: "Checkpoint".into(),
            nsfw: false,
            creator: "test".into(),
            tags: Vec::new(),
            downloads,
            thumbs: downloads / 10,
            versions: vec![ModelVersion {
                id: 1,
                name: "v1".into(),
                base_model: base.into(),
                files: vec![ModelFile {
                    name: "model.safetensors".into(),
                    size_gib: 6.0,
                    precision: "fp16".into(),
                    format: "SafeTensor".into(),
                }],
            }],
            source: "civitai.com".into(),
            commercial: Vec::new(),
            allow_credit: false,
            allow_derivatives: false,
            page_url: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn video_and_other_bases_are_filtered() {
        assert!(is_video_base("Hunyuan Video"));
        assert!(is_video_base("Wan Video 2.2"));
        assert!(is_video_base("LTXV"));
        assert!(is_other_base("Other"));
        assert!(is_other_base(""));
        assert!(!is_other_base("SDXL 1.0"));
        assert!(!is_video_base("Illustrious"));
    }

    #[test]
    fn base_filter_accepts_comma_separated_values() {
        assert!(base_matches("SDXL 1.0", "SDXL 1.0, Illustrious"));
        assert!(base_matches("Illustrious", "SDXL 1.0, Illustrious"));
        assert!(!base_matches("Pony", "SDXL 1.0, Illustrious"));
    }

    #[test]
    fn best_sort_prefers_modern_models() {
        let old = model("DreamShaper", "SD 1.5", 2_000_000);
        let modern = model("Z Image Turbo", "ZImageTurbo", 20_000);
        let mut scored = vec![
            Scored {
                model: &old,
                estimate: evaluate_version(
                    old.versions[0].clone(),
                    match_base("SD 1.5"),
                    29.3,
                    1024,
                    1024,
                    None,
                ),
            },
            Scored {
                model: &modern,
                estimate: evaluate_version(
                    modern.versions[0].clone(),
                    match_base("ZImageTurbo"),
                    29.3,
                    1024,
                    1024,
                    None,
                ),
            },
        ];
        sort_scored(&mut scored, SortMode::Best);
        assert_eq!(scored[0].model.name, "Z Image Turbo");
    }
}
