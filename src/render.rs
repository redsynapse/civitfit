use crate::arch::{tier_label, Arch};
use crate::civitai::Model;
use crate::estimate::{Status, VersionEstimate};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BAR_WIDTH: usize = 10;

pub struct FitRow<'a> {
    pub model: &'a Model,
    pub estimate: &'a VersionEstimate,
}

pub fn bar(ratio: f64) -> String {
    let filled = (ratio * BAR_WIDTH as f64)
        .round()
        .clamp(0.0, BAR_WIDTH as f64) as usize;
    "█".repeat(filled) + &"░".repeat(BAR_WIDTH - filled)
}

pub fn human_speed(seconds: f64) -> String {
    if seconds < 90.0 {
        format!("{seconds:.0}s")
    } else {
        format!("{:.1}m", seconds / 60.0)
    }
}

pub fn human_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.0}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn width_of(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn cell(text: &str, width: usize) -> String {
    let current = width_of(text);
    if current <= width {
        return format!("{text}{}", " ".repeat(width - current));
    }

    let limit = width.saturating_sub(1);
    let mut truncated = String::new();
    let mut used = 0;
    for character in text.chars() {
        let char_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + char_width > limit {
            break;
        }
        truncated.push(character);
        used += char_width;
    }
    truncated.push('…');
    let used = width_of(&truncated);
    format!("{truncated}{}", " ".repeat(width.saturating_sub(used)))
}

fn row(cells: &[String], widths: &[usize]) -> String {
    let body = cells
        .iter()
        .zip(widths)
        .map(|(text, width)| format!(" {} ", cell(text, *width)))
        .collect::<Vec<_>>()
        .join("│");
    format!("│{body}│")
}

pub fn table(headers: &[&str], rows: &[Vec<String>], widths: &[usize]) -> String {
    let top = format!(
        "╭{}╮",
        widths
            .iter()
            .map(|width| "─".repeat(width + 2))
            .collect::<Vec<_>>()
            .join("┬")
    );
    let middle = format!(
        "├{}┤",
        widths
            .iter()
            .map(|width| "─".repeat(width + 2))
            .collect::<Vec<_>>()
            .join("┼")
    );
    let bottom = format!(
        "╰{}╯",
        widths
            .iter()
            .map(|width| "─".repeat(width + 2))
            .collect::<Vec<_>>()
            .join("┴")
    );

    let header_cells: Vec<String> = headers.iter().map(|text| text.to_string()).collect();
    let mut lines = vec![top, row(&header_cells, widths), middle];
    for cells in rows {
        lines.push(row(cells, widths));
    }
    lines.push(bottom);
    lines.join("\n")
}

fn fit_row(row: &FitRow<'_>) -> Vec<String> {
    let status = row.estimate.effective_status();
    vec![
        format!("{} {}", status.icon(), status.label()),
        row.model.name.clone(),
        row.estimate.arch.family.clone(),
        format!("{:.2}GiB", row.estimate.chosen.file.size_gib),
        format!("{:.1}GiB", row.estimate.chosen.peak_gib),
        format!("{:.0}%", row.estimate.chosen.mem_ratio * 100.0),
        human_speed(row.estimate.chosen.speed_s),
        human_count(row.model.downloads),
    ]
}

pub fn fit_table(rows: &[FitRow<'_>], budget: f64, width: u32, height: u32) -> String {
    let widths = [14, 29, 14, 8, 7, 5, 6, 9];
    let headers = [
        "Status",
        "Model",
        "Arch",
        "Size",
        "Peak",
        "Mem",
        "Time",
        "Downloads",
    ];
    let body_rows: Vec<Vec<String>> = rows.iter().map(fit_row).collect();
    let rendered = table(&headers, &body_rows, &widths);
    format!(
        "{rendered}\n  budget {budget:.2}GiB  ·  {width}x{height}  ·  peak = weights + activations + runtime"
    )
}

pub fn detail(
    model: &Model,
    estimates: &[VersionEstimate],
    budget: f64,
    width: u32,
    height: u32,
    note: &str,
) -> String {
    let mut lines = vec![
        String::new(),
        model.name.clone(),
        format!("  source     {}", model.source),
        format!("  page       {}", model.page_url),
        format!("  type       {}", model.kind),
        format!("  creator    {}", model.creator),
        format!("  nsfw       {}", if model.nsfw { "yes" } else { "no" }),
        format!("  downloads  {}   likes {}", model.downloads, model.thumbs),
    ];

    let commercial = if model.commercial.is_empty() {
        "unspecified".to_string()
    } else {
        model.commercial.join(", ")
    };
    let credit = if model.allow_credit {
        "credit required"
    } else {
        "no credit required"
    };
    let derivatives = if model.allow_derivatives {
        "derivatives allowed"
    } else {
        "derivatives not allowed"
    };
    lines.push(format!(
        "  license    {commercial} · {credit} · {derivatives}"
    ));
    if !note.is_empty() {
        lines.push(format!("  versions   {note}"));
    }

    for estimate in estimates {
        let version = &estimate.version;
        let chosen = &estimate.chosen;
        lines.push(String::new());
        lines.push(format!("  ── {}  ({})", version.name, version.base_model));

        let comfy = match estimate.arch.comfy {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        lines.push(format!(
            "     architecture  {} ({})   ComfyUI: {comfy}",
            estimate.arch.family,
            tier_label(estimate.arch.tier)
        ));

        let sizes = estimate
            .files
            .iter()
            .map(|file| format!("{} {:.2}GiB", file.file.precision, file.file.size_gib))
            .collect::<Vec<_>>()
            .join(" / ");
        lines.push(format!("     weights       {sizes}"));

        if estimate.bundled {
            lines.push("     companions    bundled in the checkpoint".into());
        } else if !estimate.companions.is_empty() {
            let companions = estimate
                .companions
                .iter()
                .map(|companion| format!("{} {:.2}GiB", companion.name, companion.gb))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("     companions    {companions}"));
        } else {
            lines.push("     companions    none".into());
        }

        lines.push(format!(
            "     peak          {:.2}GiB / {:.2}GiB  {} {:.0}%",
            chosen.peak_gib,
            budget,
            bar(chosen.mem_ratio),
            chosen.mem_ratio * 100.0
        ));
        lines.push(format!(
            "     time          ~{} at {width}x{height}, {} steps",
            human_speed(chosen.speed_s),
            estimate.steps
        ));
        let status: Status = estimate.effective_status();
        lines.push(format!(
            "     verdict       {} {}",
            status.icon(),
            status.label()
        ));
        for note in &estimate.notes {
            lines.push(format!("     note          {note}"));
        }
        if !estimate.arch.notes.is_empty() {
            lines.push(format!("     info          {}", estimate.arch.notes));
        }

        if estimate.files.len() > 1 {
            lines.push("     files".into());
            for file in &estimate.files {
                lines.push(format!(
                    "       {:5} {:6.2}GiB  peak {:5.2}GiB  ~{:>4}  {}",
                    file.file.precision,
                    file.file.size_gib,
                    file.peak_gib,
                    human_speed(file.speed_s),
                    file.file.name
                ));
            }
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

pub fn arch_table(archs: &[Arch]) -> String {
    let rows: Vec<Vec<String>> = archs
        .iter()
        .map(|arch| {
            vec![
                arch.family.clone(),
                tier_label(arch.tier).to_string(),
                match arch.comfy {
                    Some(true) => "yes".into(),
                    Some(false) => "no".into(),
                    None => "?".into(),
                },
                format!("{}B", arch.params_b),
                format!("{}x{}", arch.native_res[0], arch.native_res[1]),
                arch.steps.to_string(),
                format!("{:.1}", arch.k_sec_per_step),
                if arch.bundled { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let headers = [
        "Family", "Tier", "ComfyUI", "Params", "Native", "Steps", "s/step", "Bundled",
    ];
    table(&headers, &rows, &[24, 10, 7, 7, 11, 5, 6, 7])
}
