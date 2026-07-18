//! `corpusgen gen` — materialize a corpus from a recipe (profile+seed+files).
//! `corpusgen fit` — derive a profile from a real vault: statistics only,
//! never content, so the output is committable even from a private vault.
//! Detection in `fit` is approximate line/substring scanning (documented per
//! construct) — good enough to keep profiles honest, not a parser.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "stats plumbing; see lib.rs"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bench::corpus;
use bench::profile::{ConstructRate, Pathology, Profile, Recipe, SizeDist, Unicode};

const USAGE: &str = "usage:
  corpusgen gen --profile <name|path> [--seed N] [--files N] [--out DIR]
  corpusgen fit <vault-dir> [--name NAME]     # prints profile TOML to stdout";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("gen") => cmd_gen(&args[1..]),
        Some("fit") => cmd_fit(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn cmd_gen(args: &[String]) -> Result<(), String> {
    let profile_arg = flag(args, "--profile").ok_or(USAGE)?;
    let profile = Profile::resolve(&profile_arg)?;
    let seed =
        flag(args, "--seed").map_or(Ok(1), |s| s.parse::<u64>().map_err(|e| e.to_string()))?;
    let files = flag(args, "--files")
        .map(|s| s.parse::<u32>().map_err(|e| e.to_string()))
        .transpose()?;
    let recipe = Recipe::new(profile, seed, files);
    let manifest = if let Some(out) = flag(args, "--out") {
        corpus::generate_into(Path::new(&out), &recipe).map_err(|e| e.to_string())?
    } else {
        let dir = corpus::ensure(&recipe).map_err(|e| e.to_string())?;
        println!("{}", dir.display());
        corpus::read_manifest(&dir).map_err(|e| e.to_string())?
    };
    let dir = flag(args, "--out").map_or_else(|| corpus::cache_dir(&recipe), PathBuf::from);
    let inventory = corpus::load_inventory(&dir).map_err(|e| e.to_string())?;
    eprintln!(
        "corpus {}: {} files, {} bytes, hash {}",
        recipe.id(),
        manifest.files,
        manifest.total_bytes,
        manifest.corpus_hash
    );
    eprintln!(
        "planted: {}",
        serde_json::to_string(&inventory).map_err(|e| e.to_string())?
    );
    Ok(())
}

/// Per-construct accumulators over a vault walk.
#[derive(Default)]
struct FitAcc {
    files_with: u64,
    total: u64,
}

fn cmd_fit(args: &[String]) -> Result<(), String> {
    let vault = args.first().filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
    let name = flag(args, "--name").unwrap_or_else(|| "fitted".to_owned());
    let mut sizes_ln: Vec<f64> = Vec::new();
    let mut sizes: Vec<u64> = Vec::new();
    let mut acc: BTreeMap<&'static str, FitAcc> = BTreeMap::new();
    let mut unterminated_files = 0u64;
    let mut cjk_chars = 0u64;
    let mut total_chars = 0u64;
    let mut n_files = 0u64;

    walk_vault(Path::new(vault), &mut |text| {
        n_files += 1;
        let bytes = text.len() as u64;
        sizes.push(bytes);
        sizes_ln.push((bytes.max(1) as f64).ln());
        let counts = scan_file(text);
        for (id, count) in &counts {
            let entry = acc.entry(id).or_default();
            if *count > 0 {
                entry.files_with += 1;
                entry.total += count;
            }
        }
        if counts.get("__unterminated").copied().unwrap_or(0) > 0 {
            unterminated_files += 1;
        }
        for c in text.chars().take(20_000) {
            total_chars += 1;
            if matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3040}'..='\u{30FF}' | '\u{AC00}'..='\u{D7AF}')
            {
                cjk_chars += 1;
            }
        }
    })?;
    if n_files == 0 {
        return Err(format!("no .md files under {vault}"));
    }

    sizes.sort_unstable();
    let median = sizes[sizes.len() / 2];
    let mean_ln = sizes_ln.iter().sum::<f64>() / sizes_ln.len() as f64;
    let sigma = (sizes_ln.iter().map(|x| (x - mean_ln).powi(2)).sum::<f64>()
        / sizes_ln.len() as f64)
        .sqrt();

    let mut constructs = BTreeMap::new();
    for (id, a) in &acc {
        if id.starts_with("__") || a.files_with == 0 {
            continue;
        }
        constructs.insert(
            (*id).to_owned(),
            ConstructRate {
                file_rate: round3(a.files_with as f64 / n_files as f64),
                per_file: round3(a.total as f64 / a.files_with as f64),
            },
        );
    }
    let profile = Profile {
        name,
        provenance: format!(
            "corpusgen fit over {n_files} files at {vault} (statistics only, no content); approximate line/substring detection"
        ),
        files: u32::try_from(n_files.min(2000)).unwrap_or(2000),
        size: SizeDist {
            median_bytes: median.max(64),
            sigma: round3(sigma),
            max_bytes: *sizes.last().unwrap_or(&262_144),
        },
        constructs,
        pathology: Pathology {
            unterminated_fence_rate: round3(unterminated_files as f64 / n_files as f64),
        },
        unicode: Unicode {
            cjk_rate: round3(if total_chars == 0 {
                0.0
            } else {
                cjk_chars as f64 / total_chars as f64
            }),
        },
    };
    profile.validate()?;
    print!(
        "{}",
        toml::to_string_pretty(&profile).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn walk_vault(dir: &Path, f: &mut impl FnMut(&str)) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue; // hidden dirs/files (.git, .obsidian, …)
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_vault(&path, f)?;
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            f(&text);
        }
    }
    Ok(())
}

/// Approximate per-file construct counts. Detection rules (documented so the
/// approximation is auditable):
/// - frontmatter: file starts with `---` line
/// - heading: line starts with 1–6 `#` + space (outside fences)
/// - fence: pairs of backtick-fence delimiter lines; odd count means unterminated
/// - callout: line starts `> [!` (outside fences)
/// - task: trimmed line starts `- [ ]` / `- [x]` (outside fences)
/// - table: `|…---…|` separator lines (≈ one per table)
/// - anchor: line's last token starts `^` (outside fences)
/// - wikilink/embed: `[[` occurrences minus `![[` / `![[` occurrences (whole text)
/// - comment: `%%` pairs (whole text)
/// - `inline_code`: backtick pairs outside fence-delimiter lines
fn scan_file(text: &str) -> BTreeMap<&'static str, u64> {
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut bump = |id: &'static str, n: u64| *counts.entry(id).or_default() += n;
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        bump("frontmatter", 1);
    }
    let mut in_fence = false;
    let mut fence_delims = 0u64;
    let mut backticks = 0u64;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence_delims += 1;
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        backticks += line.matches('`').count() as u64;
        let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
            bump("heading", 1);
        }
        if trimmed.starts_with("> [!") {
            bump("callout", 1);
        }
        if trimmed.starts_with("- [ ]")
            || trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [X]")
        {
            bump("task", 1);
        }
        if trimmed.starts_with('|') && trimmed.contains("---") {
            bump("table", 1);
        }
        if let Some(last) = trimmed.split_whitespace().next_back()
            && last.len() > 1
            && last.starts_with('^')
        {
            bump("anchor", 1);
        }
    }
    let embeds = text.matches("![[").count() as u64;
    let links = (text.matches("[[").count() as u64).saturating_sub(embeds);
    bump("wikilink", links);
    bump("embed", embeds);
    bump("comment", text.matches("%%").count() as u64 / 2);
    bump("inline_code", backticks / 2);
    bump("fence", fence_delims / 2);
    if fence_delims % 2 == 1 {
        bump("fence", 1);
        bump("__unterminated", 1);
    }
    counts
}
