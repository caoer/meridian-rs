//! The deterministic markdown generator: `(profile, seed, file index)` → byte-identical
//! file + construct inventory.
//!
//! **Recognizable-context invariant (load-bearing):** counted constructs are planted only
//! where a correct parser must recognize them — never inside fence bodies, callout
//! bodies, table cells, or after an unterminated fence (which is always the final block).
//! Filler text never begins a line with `#`, `>`, `|`, `-`, `` ` ``, `%%`, `[[` or `---`.
//! Indentation is used by one construct only — task lines — and never jumps more than one
//! level past the item above it, so an indented line is always a nested item and never an
//! indented code block. Together these make the inventory a sound lower bound:
//! `parsed(kind) ≥ planted(kind)` for every correct parser, which is what the ≥-planted
//! assertion in `tests/` checks.
//!

use std::fmt::Write as _;

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

use crate::inventory::Inventory;
use crate::profile::Recipe;

/// One generated file: shard-relative path, content, what was planted.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub rel_path: String,
    pub text: String,
    pub inventory: Inventory,
}

const WORDS: &[&str] = &[
    "session", "agent", "vault", "note", "span", "byte", "model", "index", "graph", "policy",
    "budget", "corpus", "sweep", "frozen", "contract", "seam", "crate", "law", "wire", "node",
    "tree", "hash", "merkle", "resolve", "splice", "atomic", "watch", "walk", "board", "task",
    "review", "gate", "lint", "build", "test", "bench", "profile", "seed", "recipe", "shape",
    "design", "schema", "ladder", "rung", "stream", "lane", "fixture", "monster", "survey",
    "margin", "detail", "signal", "window", "handoff", "memo", "ledger", "anchor", "target",
    "system", "runtime", "worker", "leader", "advisor", "kernel", "buffer", "cursor",
];

const CJK_WORDS: &[&str] = &[
    "会议", "笔记", "任务", "项目", "架构", "设计", "评审", "基准", "语法", "模型", "线索", "边界",
    "约束", "证据", "快照", "索引",
];

const FENCE_INFOS: &[&str] = &["rust", "python", "bash", "", "js"];
const CALLOUT_TYPES: &[&str] = &["note", "warning", "tip", "quote"];
const CALLOUT_FOLDS: &[&str] = &["", "+", "-"];
const FM_KEYS: &[&str] = &["title", "tags", "created", "status", "domain", "aliases"];

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(rng: &mut ChaCha8Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn draw(rng: &mut ChaCha8Rng, p: f64) -> bool {
    unit(rng) < p
}

/// Uniform in `[lo, hi)`. Modulo bias is irrelevant at generator scale.
fn range(rng: &mut ChaCha8Rng, lo: usize, hi: usize) -> usize {
    lo + (rng.next_u64() as usize) % (hi.saturating_sub(lo)).max(1)
}

fn normal(rng: &mut ChaCha8Rng) -> f64 {
    // Box-Muller; u1 floored away from 0.
    let u1 = unit(rng).max(1e-12);
    let u2 = unit(rng);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

fn poisson(rng: &mut ChaCha8Rng, lambda: f64) -> u64 {
    if lambda <= 0.0 {
        return 0;
    }
    if lambda > 30.0 {
        // Normal approximation for large λ.
        return (lambda + lambda.sqrt() * normal(rng)).round().max(0.0) as u64;
    }
    // Knuth.
    let l = (-lambda).exp();
    let mut k: u64 = 0;
    let mut p = 1.0;
    loop {
        p *= unit(rng);
        if p < l {
            return k;
        }
        k += 1;
    }
}

fn word<'a>(rng: &mut ChaCha8Rng, cjk_rate: f64) -> &'a str {
    if cjk_rate > 0.0 && draw(rng, cjk_rate) {
        CJK_WORDS[range(rng, 0, CJK_WORDS.len())]
    } else {
        WORDS[range(rng, 0, WORDS.len())]
    }
}

fn words(rng: &mut ChaCha8Rng, cjk_rate: f64, n: usize) -> String {
    let mut out = String::new();
    for i in 0..n {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(word(rng, cjk_rate));
    }
    out
}

/// `words` with the count drawn from `[lo, hi)` first (avoids nested `&mut rng`).
fn words_r(rng: &mut ChaCha8Rng, cjk_rate: f64, lo: usize, hi: usize) -> String {
    let n = range(rng, lo, hi);
    words(rng, cjk_rate, n)
}

fn hex4(rng: &mut ChaCha8Rng) -> String {
    format!("{:04x}", rng.next_u64() & 0xffff)
}

/// Block kinds the planner interleaves. Inline constructs (wikilink, embed,
/// inline code, comment, anchor) are planted into Paragraph blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Heading,
    Callout,
    Fence,
    Table,
    TaskList,
    Paragraph,
}

#[allow(
    clippy::too_many_lines,
    reason = "the block planner is one deliberate dispatch loop"
)]
#[must_use]
pub fn generate_file(recipe: &Recipe, index: u32) -> GeneratedFile {
    let p = &recipe.profile;
    let mut rng = ChaCha8Rng::seed_from_u64(splitmix64(
        recipe.seed ^ splitmix64(u64::from(index).wrapping_add(0x517c_c1b7_2722_0a95)),
    ));
    let cjk = p.unicode.cjk_rate;
    let target = sample_size(&mut rng, p);
    let mut inv = Inventory::new();
    let mut out = String::with_capacity(target + 1024);

    // Frontmatter must be the very first bytes when present.
    if p.rate("frontmatter")
        .is_some_and(|r| draw(&mut rng, r.file_rate))
    {
        write_frontmatter(&mut rng, cjk, &mut out);
        inv.bump("frontmatter");
    }

    // Per-construct plant counts for this file (active file ⇒ at least 1).
    let planned = |rng: &mut ChaCha8Rng, id: &str| -> u64 {
        p.rate(id).map_or(0, |r| {
            if draw(rng, r.file_rate) {
                poisson(rng, r.per_file).max(1)
            } else {
                0
            }
        })
    };
    let mut headings = planned(&mut rng, "heading");
    let mut callouts = planned(&mut rng, "callout");
    let mut fences = planned(&mut rng, "fence");
    let mut tables = planned(&mut rng, "table");
    let tasks = planned(&mut rng, "task");
    let mut task_lists = u64::from(tasks > 0);
    // Inline pool, drained into paragraphs (order interleaved via round-robin).
    let mut inline: Vec<(&str, u64)> = ["wikilink", "embed", "inline_code", "comment", "anchor"]
        .into_iter()
        .map(|id| (id, planned(&mut rng, id)))
        .collect();

    let unterminated = fences > 0 && draw(&mut rng, p.pathology.unterminated_fence_rate);
    if unterminated {
        fences -= 1; // reserved for the final block
    }

    let mut guard = 0u32;
    loop {
        guard += 1;
        let planted_left = headings
            + callouts
            + fences
            + tables
            + task_lists
            + inline.iter().map(|(_, n)| *n).sum::<u64>();
        if (out.len() >= target && planted_left == 0) || guard > 100_000 {
            break;
        }
        // Pick among block kinds that still owe plants; else filler paragraph.
        let mut choices: Vec<Block> = Vec::with_capacity(6);
        if headings > 0 {
            choices.push(Block::Heading);
        }
        if callouts > 0 {
            choices.push(Block::Callout);
        }
        if fences > 0 {
            choices.push(Block::Fence);
        }
        if tables > 0 {
            choices.push(Block::Table);
        }
        if task_lists > 0 {
            choices.push(Block::TaskList);
        }
        // Paragraphs both host the inline pool and pad toward target size.
        choices.push(Block::Paragraph);
        choices.push(Block::Paragraph);

        match choices[range(&mut rng, 0, choices.len())] {
            Block::Heading => {
                let level = [1usize, 2, 2, 2, 3, 3, 4][range(&mut rng, 0, 7)];
                let text = words_r(&mut rng, cjk, 2, 6);
                out.push_str(&"#".repeat(level));
                out.push(' ');
                out.push_str(&text);
                out.push_str("\n\n");
                inv.bump("heading");
                headings -= 1;
            }
            Block::Callout => {
                let ty = CALLOUT_TYPES[range(&mut rng, 0, CALLOUT_TYPES.len())];
                let fold = CALLOUT_FOLDS[range(&mut rng, 0, CALLOUT_FOLDS.len())];
                let title = words_r(&mut rng, cjk, 2, 5);
                let _ = writeln!(out, "> [!{ty}]{fold} {title}");
                for _ in 0..range(&mut rng, 1, 4) {
                    // Callout body: plain words only (recognizable-context invariant).
                    out.push_str("> ");
                    out.push_str(&words_r(&mut rng, cjk, 6, 14));
                    out.push('\n');
                }
                out.push('\n');
                inv.bump("callout");
                callouts -= 1;
            }
            Block::Fence => {
                write_fence(&mut rng, cjk, &mut out, false);
                inv.bump("fence");
                fences -= 1;
            }
            Block::Table => {
                let cols = range(&mut rng, 2, 5);
                let rows = range(&mut rng, 2, 6);
                let header: Vec<String> = (0..cols).map(|_| words(&mut rng, cjk, 1)).collect();
                let _ = writeln!(out, "| {} |", header.join(" | "));
                let _ = writeln!(out, "|{}", " --- |".repeat(cols));
                for _ in 0..rows {
                    let cells: Vec<String> =
                        (0..cols).map(|_| words_r(&mut rng, cjk, 1, 3)).collect();
                    let _ = writeln!(out, "| {} |", cells.join(" | "));
                }
                out.push('\n');
                inv.bump("table");
                tables -= 1;
            }
            Block::TaskList => {
                // Indentation only nests when a parent item is already open:
                // at block start (or after a jump of ≥2 levels) the same spaces
                // are an indented code block, and pulldown emits no
                // `TaskListMarker`. Clamp each line to at most one level deeper
                // than the item above it — the first line is therefore depth 0 —
                // so every planted line stays a recognizable list item. The draw
                // is unchanged, so the rng stream is identical to before.
                let mut max_depth = 0usize;
                for _ in 0..tasks.max(1) {
                    let depth = range(&mut rng, 0, 3).min(max_depth);
                    max_depth = depth + 1;
                    let mark = if draw(&mut rng, 0.4) { 'x' } else { ' ' };
                    out.push_str(&"  ".repeat(depth));
                    let _ = write!(out, "- [{mark}] ");
                    out.push_str(&words_r(&mut rng, cjk, 3, 8));
                    out.push('\n');
                    inv.bump("task");
                }
                out.push('\n');
                task_lists = 0;
            }
            Block::Paragraph => {
                write_paragraph(&mut rng, cjk, &mut out, &mut inline, &mut inv);
            }
        }
    }

    // Pathology: an open fence swallows the rest of the file, so it is always
    // the final block, and nothing counted may follow it.
    if unterminated {
        write_fence(&mut rng, cjk, &mut out, true);
        inv.bump("fence");
        inv.bump("fence_unterminated");
    }

    let rel_path = format!("d{:03}/f{index:05}.md", index / 1000);
    GeneratedFile {
        rel_path,
        text: out,
        inventory: inv,
    }
}

fn sample_size(rng: &mut ChaCha8Rng, p: &crate::profile::Profile) -> usize {
    let s = &p.size;
    let raw = if s.sigma <= 0.0 {
        s.median_bytes as f64
    } else {
        s.median_bytes as f64 * (s.sigma * normal(rng)).exp()
    };
    (raw as u64).clamp(64, s.max_bytes) as usize
}

fn write_frontmatter(rng: &mut ChaCha8Rng, cjk: f64, out: &mut String) {
    out.push_str("---\n");
    let n_keys = range(rng, 2, 5);
    for key in &FM_KEYS[..n_keys] {
        match *key {
            "tags" => {
                let _ = writeln!(out, "tags: [{}, {}]", word(rng, 0.0), word(rng, 0.0));
            }
            "created" => {
                let _ = writeln!(
                    out,
                    "created: 2026-{:02}-{:02}",
                    range(rng, 1, 13),
                    range(rng, 1, 29)
                );
            }
            "aliases" => {
                let _ = writeln!(out, "aliases: [{}]", word(rng, 0.0));
            }
            _ => {
                let _ = writeln!(out, "{key}: {}", words_r(rng, cjk, 1, 4));
            }
        }
    }
    out.push_str("---\n\n");
}

fn write_fence(rng: &mut ChaCha8Rng, cjk: f64, out: &mut String, unterminated: bool) {
    let info = FENCE_INFOS[range(rng, 0, FENCE_INFOS.len())];
    out.push_str("```");
    out.push_str(info);
    out.push('\n');
    for _ in 0..range(rng, 1, 6) {
        // Fence body: plain words — never counted constructs.
        out.push_str(&words_r(rng, cjk, 3, 9));
        out.push('\n');
    }
    if !unterminated {
        out.push_str("```\n\n");
    }
}

/// A paragraph hosts 0–2 plants drained from the inline pool, then plain
/// filler words wrapped ~12 per line. An anchor plant lands at the very end of
/// the block (` ^id` — Obsidian block-anchor position).
fn write_paragraph(
    rng: &mut ChaCha8Rng,
    cjk: f64,
    out: &mut String,
    inline: &mut [(&str, u64)],
    inv: &mut Inventory,
) {
    let mut tokens: Vec<String> = Vec::new();
    let n_words = range(rng, 15, 60);
    for _ in 0..n_words {
        tokens.push(word(rng, cjk).to_owned());
    }
    let mut anchor: Option<String> = None;
    let mut plants = 0usize;
    for (id, remaining) in inline.iter_mut() {
        if plants >= 2 || *remaining == 0 {
            continue;
        }
        let token = match *id {
            "wikilink" => {
                let t = format!("note-{}", hex4(rng));
                match range(rng, 0, 4) {
                    0 => format!("[[{t}]]"),
                    1 => format!("[[{t}|{}]]", words(rng, 0.0, 2)),
                    2 => format!("[[{t}#{}]]", word(rng, 0.0)),
                    _ => format!("[[{t}#^{}]]", hex4(rng)),
                }
            }
            "embed" => {
                if draw(rng, 0.5) {
                    format!("![[img-{}.png]]", hex4(rng))
                } else {
                    format!("![[note-{}]]", hex4(rng))
                }
            }
            "inline_code" => format!("`{}`", words_r(rng, 0.0, 1, 4)),
            "comment" => format!("%%{}%%", words_r(rng, 0.0, 1, 4)),
            "anchor" => {
                anchor = Some(format!("^p{}", hex4(rng)));
                *remaining -= 1;
                inv.bump("anchor");
                plants += 1;
                continue;
            }
            _ => unreachable!("inline pool ids are fixed"),
        };
        // Insert mid-paragraph, never at position 0 (line starts stay plain).
        let pos = range(rng, 1, tokens.len().max(2));
        tokens.insert(pos, token);
        inv.bump(id);
        *remaining -= 1;
        plants += 1;
    }
    // Wrap ~12 tokens per line; filler never starts a line with marker chars
    // (lexicon is plain words), but a planted token could land at a wrap
    // boundary — nudge it off line-start by joining with its predecessor.
    let mut line_len = 0usize;
    let mut text = String::new();
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 {
            if line_len >= 12 && !starts_with_marker(tok) {
                text.push('\n');
                line_len = 0;
            } else {
                text.push(' ');
            }
        }
        text.push_str(tok);
        line_len += 1;
    }
    if let Some(a) = anchor {
        text.push(' ');
        text.push_str(&a);
    }
    out.push_str(&text);
    out.push_str("\n\n");
}

fn starts_with_marker(tok: &str) -> bool {
    matches!(
        tok.as_bytes().first(),
        Some(b'#' | b'>' | b'|' | b'-' | b'`' | b'%' | b'[' | b'!' | b'^')
    )
}
