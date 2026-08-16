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
//! The write is a wire `splice` to the running daemon ([`crate::write_ipc`]) with
//! the pin riding as a Splice-sibling field, so the page's content and its lock
//! block land in one commit. There is no in-process publication path.
//!
//! # No `--actor`
//! The read-mint gate keys on a daemon-derived session identity, and a CLI
//! invocation has no session: the bare `mrd pin` is local-operator-trusted and
//! the gate is bypassed, exactly as `mrd put` bypasses the host's authz.
//!
//! Exit triad: 0 pinned (or `--dry` rehearsed) / 1 refused (`read_mint_required`,
//! `pin_target_missing`, `write_conflict`, an armed gate refusal — the engine's
//! verbatim message) / 2 bad invocation (including a down daemon).

use serde_json::{Value, json};
use wire::{Path as WirePath, PinSpec};

use crate::{Fail, Format, current_dir, engine, write_ipc};

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
    // The CLI stamps no provenance: an absent actor is the local-operator trust
    // door the read-mint gate reads. A pin is the whole batch. `--force` is
    // the local-operator refuse→rewrite so the wire guard does not demand a
    // node rev the pin verb never held (the lock-block edit is engine-authored).
    let pin = PinSpec {
        target: WirePath(parsed.target.clone()),
        selector: wire::ReadSel::parse(&parsed.selector),
        vibe: parsed.vibe.then_some(true),
    };
    let mut request = json!({
        "op": "splice",
        "path": parsed.page,
        "pin": pin,
        "force": true,
    });
    if parsed.dry {
        request["dry"] = json!(true);
    }

    let mut door = write_ipc::connect(&resolved.workspace)?;
    let body = write_ipc::call(&mut door, &request)
        .map_err(|e| engine::json_refusal(parsed.format, &resolved.workspace, &e))?;
    let body = write_ipc::project_body(&body);

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
