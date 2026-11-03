use anyhow::{bail, Result};
use serde::Serialize;
use std::process::Command;

pub const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Serialize)]
pub struct Hardware {
    pub os: String,
    pub chip: String,
    pub cpu_cores: u32,
    pub ram_gib: f64,
    pub gpu_name: String,
    pub gpu_budget_gib: f64,
    pub backend: String,
    pub unified_memory: bool,
    pub detected_with: String,
}

fn run(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Parse `32G`, `32000M`, `1.5T`, `32GiB` or a bare number of GiB.
pub fn parse_size(text: &str) -> Result<f64> {
    let trimmed = text.trim().to_ascii_lowercase();
    let without_suffix = trimmed
        .strip_suffix("ib")
        .or_else(|| trimmed.strip_suffix('b'))
        .unwrap_or(&trimmed);

    let (number, unit) = match without_suffix.chars().last() {
        Some(ch @ ('k' | 'm' | 'g' | 't')) => {
            (without_suffix[..without_suffix.len() - 1].trim(), Some(ch))
        }
        _ => (without_suffix.trim(), None),
    };

    let value: f64 = match number.parse() {
        Ok(value) if value > 0.0 => value,
        _ => bail!("cannot parse size: {text:?}"),
    };

    let power = match unit {
        Some('k') => 1,
        Some('m') => 2,
        Some('g') => 3,
        Some('t') => 4,
        _ => return Ok(round2(value)),
    };
    Ok(round2(value * 1024f64.powi(power) / GIB))
}

fn macos() -> Hardware {
    let ram = run("sysctl", &["-n", "hw.memsize"])
        .and_then(|value| value.parse::<f64>().ok())
        .map(|bytes| bytes / GIB)
        .unwrap_or(0.0);

    let chip = run("sysctl", &["-n", "machdep.cpu.brand_string"])
        .unwrap_or_else(|| "Apple Silicon".to_string());
    let cores = run("sysctl", &["-n", "hw.ncpu"])
        .and_then(|value| value.parse::<u32>().ok())
        .or_else(|| {
            std::thread::available_parallelism()
                .ok()
                .map(|n| n.get() as u32)
        })
        .unwrap_or(0);

    let wired_mb = run("sysctl", &["-n", "iogpu.wired_limit_mb"])
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0);

    let (budget, detected_with) = match wired_mb {
        Some(mb) => (mb as f64 / 1024.0, "sysctl iogpu.wired_limit_mb"),
        None => (ram * 0.85, "sysctl hw.memsize (85% fallback)"),
    };

    let version = run("sw_vers", &["-productVersion"]).unwrap_or_else(|| "?".into());

    Hardware {
        os: format!("macOS {version}"),
        chip: chip.clone(),
        cpu_cores: cores,
        ram_gib: round2(ram),
        gpu_name: format!("{chip} (unified)"),
        gpu_budget_gib: round2(budget),
        backend: "Metal".into(),
        unified_memory: true,
        detected_with: detected_with.into(),
    }
}

fn linux() -> Hardware {
    let ram = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let rest = line.strip_prefix("MemTotal:")?;
                let kib: f64 = rest.split_whitespace().next()?.parse().ok()?;
                Some(kib * 1024.0 / GIB)
            })
        })
        .unwrap_or(0.0);

    let chip = run("uname", &["-m"]).unwrap_or_else(|| "unknown".into());
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(0);
    let (mut gpu_name, mut budget, mut backend, mut detected_with) =
        ("CPU".to_string(), ram * 0.5, "CPU", "cpu only");

    if let Some(smi) = run(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    ) {
        if let Some(first) = smi.lines().next() {
            let parts: Vec<&str> = first.split(',').map(str::trim).collect();
            if parts.len() == 2 {
                if let Ok(mib) = parts[1].parse::<f64>() {
                    gpu_name = parts[0].to_string();
                    budget = mib / 1024.0;
                    backend = "CUDA";
                    detected_with = "nvidia-smi";
                }
            }
        }
    }

    let release = run("uname", &["-r"]).unwrap_or_else(|| "?".into());

    Hardware {
        os: format!("Linux {release}"),
        chip,
        cpu_cores: cores,
        ram_gib: round2(ram),
        gpu_name,
        gpu_budget_gib: round2(budget),
        backend: backend.into(),
        unified_memory: false,
        detected_with: detected_with.into(),
    }
}

fn generic() -> Hardware {
    Hardware {
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        chip: std::env::consts::ARCH.into(),
        cpu_cores: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(0),
        ram_gib: 0.0,
        gpu_name: "unknown".into(),
        gpu_budget_gib: 0.0,
        backend: "unknown".into(),
        unified_memory: false,
        detected_with: "generic fallback".into(),
    }
}

pub fn detect(ram_override: Option<&str>, gpu_override: Option<&str>) -> Result<Hardware> {
    let mut hardware = match std::env::consts::OS {
        "macos" => macos(),
        "linux" => linux(),
        _ => generic(),
    };

    if let Some(ram) = ram_override {
        hardware.ram_gib = parse_size(ram)?;
        hardware.detected_with.push_str(" + --ram");
    }
    if let Some(gpu) = gpu_override {
        hardware.gpu_budget_gib = parse_size(gpu)?;
        hardware.detected_with.push_str(" + --gpu");
    }
    Ok(hardware)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_sizes() {
        assert_eq!(parse_size("32G").unwrap(), 32.0);
        assert_eq!(parse_size("32000M").unwrap(), 31.25);
        assert_eq!(parse_size("1.5T").unwrap(), 1536.0);
        assert_eq!(parse_size("24").unwrap(), 24.0);
        assert!(parse_size("nope").is_err());
    }
}
