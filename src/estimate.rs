use crate::arch::Arch;
use crate::civitai::{ModelFile, ModelVersion};
use serde::Serialize;

pub const RUNTIME_OVERHEAD_GIB: f64 = 0.5;
// Text encoders are loaded for the prompt and usually released before the
// sampling loop. 0.55 keeps the estimate honest without pretending they are free.
pub const COMPANION_WEIGHT: f64 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Comfortable,
    Good,
    Tight,
    Marginal,
    WontRun,
    Unsupported,
}

impl Status {
    pub fn icon(self) -> &'static str {
        match self {
            Status::Comfortable | Status::Good => "🟢",
            Status::Tight => "🟡",
            Status::Marginal => "🟠",
            Status::WontRun => "🔴",
            Status::Unsupported => "⛔",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Comfortable => "Perfect",
            Status::Good => "Good",
            Status::Tight => "Tight",
            Status::Marginal => "Marginal",
            Status::WontRun => "Won't run",
            Status::Unsupported => "Unsupported",
        }
    }

    /// Broad buckets used for the default ordering. Everything that can run
    /// (even tightly) stays above won't-run and unsupported entries.
    pub fn fit_group(self) -> u8 {
        match self {
            Status::Comfortable | Status::Good | Status::Tight | Status::Marginal => 0,
            Status::WontRun => 1,
            Status::Unsupported => 2,
        }
    }
}

pub fn activation_gib(width: u32, height: u32) -> f64 {
    let megapixels = (f64::from(width) * f64::from(height)) / (1024.0 * 1024.0);
    if megapixels <= 1.5 {
        1.7
    } else if megapixels <= 3.0 {
        2.6
    } else if megapixels <= 6.0 {
        4.0
    } else {
        6.0
    }
}

pub fn status_for(ratio: f64) -> Status {
    if ratio <= 0.60 {
        Status::Comfortable
    } else if ratio <= 0.75 {
        Status::Good
    } else if ratio <= 0.90 {
        Status::Tight
    } else if ratio <= 1.00 {
        Status::Marginal
    } else {
        Status::WontRun
    }
}

pub fn is_bundled(arch: &Arch, files: &[ModelFile]) -> bool {
    if arch.bundled {
        return true;
    }
    if files.is_empty() || arch.params_b <= 0.0 {
        return false;
    }
    let heaviest = files
        .iter()
        .map(|file| file.size_gib)
        .fold(0.0_f64, f64::max);
    let expected_fp16 = arch.params_b * 2.0;
    heaviest / expected_fp16 >= 1.25
}

#[derive(Debug, Clone)]
pub struct FileEstimate {
    pub file: ModelFile,
    pub peak_gib: f64,
    pub mem_ratio: f64,
    pub status: Status,
    pub speed_s: f64,
}

#[derive(Debug, Clone)]
pub struct VersionEstimate {
    pub version: ModelVersion,
    pub arch: &'static Arch,
    pub bundled: bool,
    pub companions: Vec<crate::arch::Companion>,
    pub files: Vec<FileEstimate>,
    pub chosen: FileEstimate,
    pub notes: Vec<String>,
    pub steps: u32,
    pub loadable_weights: bool,
}

impl VersionEstimate {
    pub fn effective_status(&self) -> Status {
        if !self.loadable_weights || self.arch.comfy == Some(false) {
            Status::Unsupported
        } else {
            self.chosen.status
        }
    }
}

fn file_estimate(
    file: &ModelFile,
    arch: &Arch,
    companions: &[crate::arch::Companion],
    budget: f64,
    width: u32,
    height: u32,
    steps: u32,
) -> FileEstimate {
    let companion_max = companions
        .iter()
        .map(|companion| companion.gb * COMPANION_WEIGHT)
        .fold(0.0_f64, f64::max);
    let resident = file.size_gib.max(companion_max);
    let peak = resident + activation_gib(width, height) + RUNTIME_OVERHEAD_GIB;
    let ratio = if budget > 0.0 { peak / budget } else { 99.0 };

    let area = (f64::from(width) * f64::from(height)) / (1024.0 * 1024.0);
    let k = if arch.key == "unknown" {
        (0.55 * file.size_gib).max(1.2)
    } else {
        arch.k_sec_per_step
    };
    let speed = k * f64::from(steps) * area;

    FileEstimate {
        file: file.clone(),
        peak_gib: peak,
        mem_ratio: ratio,
        status: status_for(ratio),
        speed_s: speed,
    }
}

pub fn evaluate_version(
    version: ModelVersion,
    arch: &'static Arch,
    budget: f64,
    width: u32,
    height: u32,
    steps_override: Option<u32>,
) -> VersionEstimate {
    let steps = steps_override.unwrap_or(arch.steps);
    let loadable_weights = version.files.iter().any(|file| file.is_weight());
    let bundled = is_bundled(arch, &version.files);
    let companions = if bundled {
        Vec::new()
    } else {
        arch.companions.clone()
    };

    let mut files: Vec<FileEstimate> = version
        .files
        .iter()
        .filter(|file| file.size_gib > 0.0)
        .map(|file| file_estimate(file, arch, &companions, budget, width, height, steps))
        .collect();

    if files.is_empty() {
        let missing = ModelFile {
            name: "unknown".into(),
            size_gib: 0.0,
            precision: "?".into(),
            format: "?".into(),
        };
        files.push(file_estimate(
            &missing,
            arch,
            &companions,
            budget,
            width,
            height,
            steps,
        ));
    }

    let fitting: Vec<&FileEstimate> = files.iter().filter(|file| file.mem_ratio <= 1.0).collect();
    let candidates: Vec<&FileEstimate> = if fitting.is_empty() {
        files.iter().collect()
    } else {
        fitting
    };
    let chosen = candidates
        .iter()
        .max_by(|left, right| {
            left.file
                .size_gib
                .partial_cmp(&right.file.size_gib)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|file| (**file).clone())
        .unwrap_or_else(|| files[0].clone());

    let mut notes = Vec::new();
    if !loadable_weights {
        notes.push("no loadable weights (ONNX/auxiliary files only)".to_string());
    }
    if arch.key == "unknown" {
        notes.push("no architecture entry; size-only estimate".to_string());
    }
    if bundled && !arch.bundled {
        notes.push("large file: bundled encoders assumed".to_string());
    }
    if arch.comfy.is_none() {
        notes.push("ComfyUI support unknown".to_string());
    }

    VersionEstimate {
        version,
        arch,
        bundled,
        companions,
        files,
        chosen,
        notes,
        steps,
        loadable_weights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::match_base;

    fn version(base: &str, files: Vec<ModelFile>) -> ModelVersion {
        ModelVersion {
            id: 1,
            name: "test".into(),
            base_model: base.into(),
            files,
        }
    }

    fn file(name: &str, size_gib: f64) -> ModelFile {
        ModelFile {
            name: name.into(),
            size_gib,
            precision: "int8".into(),
            format: "SafeTensor".into(),
        }
    }

    #[test]
    fn status_thresholds() {
        assert_eq!(status_for(0.5), Status::Comfortable);
        assert_eq!(status_for(0.7), Status::Good);
        assert_eq!(status_for(0.8), Status::Tight);
        assert_eq!(status_for(0.95), Status::Marginal);
        assert_eq!(status_for(1.2), Status::WontRun);
    }

    #[test]
    fn zimage_matches_measurement() {
        let estimate = evaluate_version(
            version("Z-Image", vec![file("z_image_turbo_int8.safetensors", 5.8)]),
            match_base("Z-Image"),
            29.30,
            1024,
            1024,
            None,
        );
        assert_eq!(estimate.effective_status(), Status::Comfortable);
        // measured: 7.75GiB peak and 25.5s on Apple Silicon
        assert!((estimate.chosen.peak_gib - 8.0).abs() < 0.6);
        assert!((estimate.chosen.speed_s - 25.6).abs() < 3.0);
    }

    #[test]
    fn sdxl_is_bundled() {
        let sdxl = match_base("SDXL 1.0");
        assert!(is_bundled(sdxl, &[file("epicrealism.safetensors", 6.46)]));
    }

    #[test]
    fn auxiliary_only_files_are_unsupported() {
        let zip = ModelFile {
            name: "trainingData.zip".into(),
            size_gib: 0.9,
            precision: "?".into(),
            format: "Other".into(),
        };
        let estimate = evaluate_version(
            version("Flux.1 D", vec![zip]),
            match_base("Flux.1 D"),
            29.3,
            1024,
            1024,
            None,
        );
        assert_eq!(estimate.effective_status(), Status::Unsupported);
        assert!(!estimate.loadable_weights);
    }

    #[test]
    fn moss_is_not_bundled() {
        let moss = match_base("MOSS Image");
        assert!(!is_bundled(moss, &[file("moss.safetensors", 9.93)]));
    }
}
