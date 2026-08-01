//! `mrd test` — the verb's dispatcher, plus the literate-fixture readers its two
//! surviving tiers share.
//!
//! # The scenario tier retired (ruling D3)
//! `mrd test <DIR>` used to run a third tier: literate scenario pages that mounted
//! a sibling `base/` into a tmpdir, routed a `^put` through the production write
//! path, and asserted a `^expect` starlark block over `t.result` / `t.doc(path)` /
//! `t.journal`. Its fixture space was folder-shaped BY CONSTRUCTION — a scenario
//! was addressed as `conventions/<slug>/scenarios/<name>.md`, and the convention
//! it exercised was the sibling `CHECK.md`/`HOOK.md`. The registration cutover
//! made a rule a PAGE with no folder around it, so there is no sibling to hydrate
//! and no slug to address; the tier had nothing left to stand on and RETIRES
//! rather than re-keys. Its coverage went to the corpus tier
//! ([`crate::corpus_tier`]) and to the write-path suites that already own the
//! confinement, CAS and journaling laws.
//!
//! What remains here is the dispatcher and three readers the two surviving tiers
//! genuinely share: [`confine`] (the mount law), [`parse_frontmatter`] and
//! [`scan_blocks`] (the literate-fixture grammar). They live here rather than in
//! `corpus_tier` because `--history` reads them too, and one grammar with two
//! readers is how two tiers come to disagree about what a fixture says.
//!
//! # The two tiers
//! - `--corpus SPEC` — [`crate::corpus_tier`], the pre-arming runner over
//!   SYNTHETIC changes derived from a governed corpus.
//! - `--history WORKSPACE` — [`crate::history_cmd`], the same law replayed against
//!   a workspace's own committed past.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::{Fail, Format};

/// Run `mrd test --corpus SPEC` / `mrd test --history WORKSPACE …`.
///
/// # Errors
/// [`Fail`] — exit 2 (bad usage, unreadable input, a malformed spec) or exit 1
/// (the tier reported findings).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    // The history tier is a distinct mode: `--history` reconstructs journal rows
    // against git and runs a rule's `check_change` over each.
    if args.iter().any(|a| a == "--history") {
        return crate::history_cmd::dispatch(args);
    }

    let mut path: Option<String> = None;
    let mut json = false;
    let mut corpus = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--corpus" => corpus = true,
            flag if flag.starts_with('-') => {
                return Err(Fail::tool(format!("unknown flag: {flag}")));
            }
            value if path.is_none() => path = Some(value.to_owned()),
            value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
        }
    }
    let format = if json { Format::Json } else { Format::Human };

    // Tier 2 — the corpus runner: the positional is a corpus-test SPEC file.
    if corpus {
        let spec = path.ok_or_else(|| Fail::tool("test --corpus needs a SPEC file".to_owned()))?;
        return crate::corpus_tier::run(&spec, format);
    }

    // A bare positional named a scenario file or directory. That tier is gone, and
    // saying so beats a "no *.md scenarios here" that reads like a missing fixture.
    Err(Fail::tool(
        "mrd test needs a tier: `--corpus SPEC` or `--history WORKSPACE --rule PAGE`. \
         The bare scenario tier (`mrd test <DIR>`) retired with the convention-folder \
         loader — a rule is a page now, so a scenario has no sibling folder to run in"
            .to_owned(),
    ))
}

// ── mount confinement (load-bearing, taxonomy row 22) ────────────────────────

/// Canonicalize `path` WITHIN a fixture mount: the §1 path law the strict writer
/// enforces — no absolute path, no `.` / `..` / empty segment — so a fixture write
/// can never escape the tmpdir via `mount.join`. Returns the workspace-relative
/// path; a violation is the caller's `bad_path`.
pub(crate) fn confine(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("empty path escapes the mount".to_owned());
    }
    if path.starts_with('/') {
        return Err(format!("absolute path {path:?} escapes the mount"));
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!(
                "path {path:?} has an illegal segment {seg:?} (escapes the mount)"
            ));
        }
    }
    Ok(PathBuf::from(path))
}

// ── the literate-fixture grammar, read once for both tiers ───────────────────

/// Parse the leading `--- … ---` YAML frontmatter as flat `key: value` pairs
/// (quotes stripped). Enough for the spec keys; not a general YAML parser.
pub(crate) fn parse_frontmatter(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return map;
    }
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_owned();
            let val = v.trim().trim_matches(['"', '\'']).to_owned();
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}

/// Every fenced block as `(info-string, body)`. The body preserves inner lines
/// verbatim (each with its trailing newline) — that is the JSON / the id list.
/// Frontmatter carries no fence, so scanning the whole text is safe.
pub(crate) fn scan_blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            let info = info.trim().to_owned();
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            out.push((info, body));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{confine, parse_frontmatter, scan_blocks};

    #[test]
    fn confine_rejects_absolute_and_dotdot_and_dot_and_empty() {
        assert!(confine("/etc/passwd").is_err(), "absolute rejected");
        assert!(confine("../secret.md").is_err(), ".. rejected");
        assert!(confine("a/../b.md").is_err(), "interior .. rejected");
        assert!(confine("a/./b.md").is_err(), ". rejected");
        assert!(confine("").is_err(), "empty rejected");
        assert!(confine("a//b.md").is_err(), "empty segment rejected");
    }

    #[test]
    fn confine_admits_a_plain_relative_path() {
        assert_eq!(
            confine("notes/todo.md").unwrap().to_str(),
            Some("notes/todo.md")
        );
    }

    #[test]
    fn frontmatter_reads_flat_pairs_and_strips_quotes() {
        let fm =
            parse_frontmatter("---\nrule: \"../rules/a.md\"\ncorpus: ../tree\n---\n\n# body\n");
        assert_eq!(fm.get("rule").map(String::as_str), Some("../rules/a.md"));
        assert_eq!(fm.get("corpus").map(String::as_str), Some("../tree"));
        assert!(
            parse_frontmatter("# no frontmatter\n").is_empty(),
            "a page without frontmatter has no keys"
        );
    }

    #[test]
    fn scan_blocks_keeps_the_info_string_and_the_verbatim_body() {
        let blocks = scan_blocks("intro\n\n```rules\nalpha\nbeta\n```\n\n```case\n{}\n```\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, "rules");
        assert_eq!(blocks[0].1, "alpha\nbeta\n");
        assert_eq!(blocks[1].0, "case");
        assert_eq!(blocks[1].1, "{}\n");
    }
}
