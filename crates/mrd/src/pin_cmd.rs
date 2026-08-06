//! `mrd pin` — the attestation verb: record, in a page's `meridian-lock` block,
//! that it draws from a named section of another page at a content-addressed
//! fingerprint.
//!
//! ```text
//! mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry] [--json]
//! ```
//!
//! `PAGE` is the pinning page — the drawing end, whose lock records the claim
//! (you pin what you draw from). `TARGET#SELECTOR` is the pinned content, in
//! the same `<PATH>[#FRAG]` grammar `mrd read` takes; the selector is a
//! sanitized heading path (`Notes/Q3`) or a block anchor (`^id`).
//!
//! The write routes through the production splice choke-point
//! ([`wire_serve::write::splice`]) in-process with the pin riding as a
//! Splice-sibling field, so the page's content and its lock block land in one
//! `commit_batch` — one flock, one rename — inheriting the CAS guards, the
//! armed gate, and the write flock.
//!
//! # No `--actor`
//! The read-mint gate keys on a daemon-derived session identity, and a CLI
//! invocation has no session: the bare `mrd pin` is local-operator-trusted and
//! the gate is bypassed, exactly as `mrd put` bypasses the host's authz.
//!
//! Exit triad: 0 pinned (or `--dry` rehearsed) / 1 refused (`read_mint_required`,
//! `pin_target_missing`, `write_conflict`, `workspace_busy`, an armed gate
//! refusal — the engine's verbatim message) / 2 bad invocation.

use serde_json::{Value, json};
use wire::{Path as WirePath, PinSpec};
use wire_serve::write::{SpliceArgs, splice};

use crate::{Fail, Format, current_dir, engine};

/// Run `mrd pin <PAGE> <TARGET><SELECTOR> [flags]`. Errors [`Fail`] — exit 2 on a bad
/// invocation (missing or malformed positionals, unknown flags, a `bad_request` refusal); exit
/// 1 on any other engine refusal, message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Pin::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);

    let splice_args = SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WirePath(parsed.page.clone()),
        // The CLI stamps no provenance, and an absent actor is the local-operator trust door
        // the read-mint gate reads.
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: parsed.dry,
        force: false,
        // A pin is the whole batch: the lock block rides as the engine-minted
        // edit, so there is nothing for the caller to edit.
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WirePath(parsed.target.clone()),
            // The wire carries a tagged selector; this is the only place the human string a
            // person typed becomes one.
            selector: wire::ReadSel::parse(&parsed.selector),
            vibe: parsed.vibe.then_some(true),
        }),
    };
    // seq 0, like the resident daemon (no epoch ring); no read-mint ledger
    // exists in a CLI process, which is why the gate is bypassed above.
    let outcome =
        splice(&root, None, &splice_args, &[], None).map_err(|e| refusal_with_cause(&e))?;

    let body = serde_json::to_value(&outcome.body)
        .map_err(|e| Fail::tool(format!("cannot render the answer: {e}")))?;
    // The CLI speaks the v3 vocabulary (`fingerprint`, never bare `root`).
    let mut frame = json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    let body = frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null);

    match parsed.format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "pin": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print_human(&parsed, &body),
    }
    Ok(())
}

/// The engine's refusal with its `cause` carried, in the same `class (cause)` shape
/// [`crate::status_cmd`] degrades in (`unknown (not a git repository: …)`).
fn refusal_with_cause(error: &wire::ErrorBody) -> Fail {
    let mut fail = engine::refusal_fail(error);
    if let Some(cause) = &error.cause {
        fail.message = format!("{} ({cause})", fail.message);
    }
    fail
}

/// The human summary: what was pinned, at which digest, with the stable anchor
/// the claim now has a handle on.
fn print_human(parsed: &Pin, body: &Value) {
    let pin = body.get("pin");
    let field = |key: &str| {
        pin.and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .unwrap_or("?")
    };
    let verb = if parsed.dry { "would pin" } else { "pinned" };
    // The wire fact carries no joined `declared_ref` echo, so the line is composed from the two
    // fields that carry the same address structurally — which is also what the caller typed.
    println!(
        "{verb} {}#{} into {}",
        field("target"),
        parsed.selector,
        parsed.page
    );
    println!("  fingerprint: {}", field("fingerprint"));
    let promoted = pin
        .and_then(|p| p.get("promoted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let anchor = field("anchor");
    if promoted {
        println!("  anchor:      ^{anchor} (written into {})", parsed.target);
    } else {
        println!("  anchor:      ^{anchor} (already present)");
    }
    match pin.and_then(|p| p.get("blob")).and_then(Value::as_str) {
        Some(blob) => println!("  blob:        {blob}"),
        // Git could not answer, so the retrieval plane carries no entry — never a fabricated sha.
        None => println!(
            "  blob:        (none — git could not address {})",
            parsed.target
        ),
    }
    if parsed.dry {
        println!("  dry run: nothing written");
        return;
    }
    if let Some(after) = body.get("fingerprint_after").and_then(Value::as_str) {
        println!("  workspace:   {after}");
    }
}

/// The parsed `pin` invocation.
#[derive(Debug)]
struct Pin {
    /// The pinning page (workspace-relative) — the drawing end.
    page: String,
    /// The pinned page.
    target: String,
    /// The selector inside the target.
    selector: String,
    vibe: bool,
    dry: bool,
    format: Format,
}

impl Pin {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Vec<String> = Vec::new();
        let mut vibe = false;
        let mut dry = false;
        let mut json = false;
        for arg in args {
            match arg.as_str() {
                "--json" => json = true,
                "--vibe" => vibe = true,
                "--dry" => dry = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => positional.push(value.to_owned()),
            }
        }
        let [page, spec] = positional.as_slice() else {
            return Err(Fail::tool(
                "pin needs PAGE and TARGET#SELECTOR — e.g. \
                 `mrd pin notes/plan.md guide.md#Leaders Guideline`"
                    .to_owned(),
            ));
        };
        // The same `<PATH>[#FRAG]` grammar `mrd read` takes.
        let Some((target, selector)) = spec.split_once('#') else {
            return Err(Fail::tool(format!(
                "pin needs a SECTION to pin: {spec}#<Heading/Path> or {spec}#^<id>. \
                 A page-level pin cannot localize drift — a change anywhere in the page \
                 would redden every dependent, which section-level pins exist to avoid."
            )));
        };
        if target.is_empty() || selector.is_empty() {
            return Err(Fail::tool(format!(
                "pin wants TARGET#SELECTOR (both parts non-empty): {spec}"
            )));
        }
        Ok(Pin {
            page: page.clone(),
            target: target.to_owned(),
            selector: selector.to_owned(),
            vibe,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[cfg(test)]
mod tests {
    //! Invocation parsing only — the pin behavior is gated engine-side
    //! (`crates/wire-serve/tests/s7_pin.rs`), where a real workspace and a real read-mint ledger
    //! exist.

    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_ref_grammar_splits_target_from_selector() {
        let p = Pin::parse(&argv(&["notes/plan.md", "guide.md#Notes/Q3"])).expect("parses");
        assert_eq!(p.page, "notes/plan.md");
        assert_eq!(p.target, "guide.md");
        assert_eq!(p.selector, "Notes/Q3");
        assert!(!p.vibe && !p.dry);

        let anchor = Pin::parse(&argv(&["a.md", "b.md#^claim"])).expect("parses");
        assert_eq!(anchor.selector, "^claim");
    }

    #[test]
    fn flags_are_recognized_and_unknown_ones_are_loud() {
        let p =
            Pin::parse(&argv(&["a.md", "b.md#A", "--vibe", "--dry", "--json"])).expect("parses");
        assert!(p.vibe && p.dry);
        assert!(matches!(p.format, Format::Json));

        let bad = Pin::parse(&argv(&["a.md", "b.md#A", "--nope"])).expect_err("refused");
        assert_eq!(bad.code, 2);
        assert!(bad.message.contains("--nope"), "{}", bad.message);
    }

    /// A page-level pin cannot localize drift, so the grammar refuses one and says why.
    #[test]
    fn a_bare_page_ref_refuses_and_teaches_the_section_grammar() {
        let err = Pin::parse(&argv(&["a.md", "b.md"])).expect_err("refused");
        assert_eq!(err.code, 2);
        assert!(err.message.contains("needs a SECTION"), "{}", err.message);

        for bad in [["a.md", "#A"], ["a.md", "b.md#"]] {
            let err = Pin::parse(&argv(&bad)).expect_err("both parts required");
            assert!(err.message.contains("non-empty"), "{}", err.message);
        }
    }

    #[test]
    fn missing_positionals_refuse_exit_two() {
        for args in [vec!["a.md"], vec![], vec!["a.md", "b.md#A", "c.md"]] {
            let err = Pin::parse(&argv(&args)).expect_err("refused");
            assert_eq!(err.code, 2);
        }
    }
}
