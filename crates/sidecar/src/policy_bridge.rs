//! The policy ADMISSION edge (P6-VERDICTS, advisor Ruling 1): the sidecar-mode
//! place that compiles `policy` rule packs through the load gate over the REAL
//! `syntax`→`model` parse pipeline. Fixtures demonstrate over the SAME fact plane
//! production evaluation runs, so no synthetic drift can admit a pack the wire
//! never reproduces — then a corpus-class pack is refused LOUD (`daemon_only`): a
//! sidecar-mode engine holds no resident corpus name index (§8/§11.3
//! `BudgetClass::Corpus` law).
//!
//! Evaluation itself — the ONE production `policy::evaluate` call over the touched
//! doc's post-batch state — is the write path's, so it lives with the shared
//! `splice → commit` choke-point (`wire_serve::write::evaluate_verdicts`); both
//! hosts evaluate through one implementation. Admission stays here because the
//! `daemon_only` refusal is sidecar-mode-specific: the resident daemon holds the
//! corpus index a sidecar cannot.
//!
//! The dependency edge runs ONE way (sidecar → policy): policy names no wire type
//! and gains no new dep (it consumes supplied facts).

use wire::{ErrorBody, ErrorCode};

/// The real parse→facts builder injected into policy's load gate: fixture bytes →
/// world-model facts through `syntax::parse` → `model::build` → `facts_from_document`
/// — the SAME plane evaluation runs. Stamps the fixture path (`model::build` is
/// I/O-free and leaves it empty). Kept in policy vocabulary (`&str`s in,
/// `policy::FactDoc` out) — no policy signature names a `syntax`/`model` type.
fn real_facts(path: &str, body: &str) -> policy::FactDoc {
    let mut doc = model::build(body.to_string(), syntax::parse(body));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    policy::facts_from_document(&doc)
}

/// Admit a rule pack sidecar-mode. Compiles through the real load gate, then — the
/// §8/§11.3 `daemon_only` law — refuses a corpus-class pack LOUD: its WHEN needs the
/// resident corpus name index a sidecar-mode engine does not hold. Node/file-class
/// packs return admitted; only corpus-class raises `daemon_only`, and every §4
/// single-file op is served from disk bytes alone, so none other can (§10.3).
///
/// # Errors
/// [`ErrorCode::DaemonOnly`] for a corpus-class pack; otherwise a wire envelope for
/// the underlying compile refusal (pack sourcing/config is the daemon's concern —
/// surfaced as `internal` until a Go-side pack-admission surface lands).
pub fn admit(
    pin: &policy::RulesetPin,
    source: &str,
    files: &dyn policy::PackFiles,
) -> Result<policy::CompiledRuleset, Box<ErrorBody>> {
    let compiled = policy::compile(pin, source, files, &real_facts).map_err(|e| {
        let mut w = ErrorBody::new(ErrorCode::Internal);
        w.message = Some(format!("rule pack '{}' failed admission: {e:?}", pin.id));
        Box::new(w)
    })?;
    if compiled.budget_class() == policy::BudgetClass::Corpus {
        let mut e = ErrorBody::new(ErrorCode::DaemonOnly);
        e.message = Some(format!(
            "corpus-class pack '{}' needs the resident corpus name index — refused in \
             sidecar mode (no resident index; §8/§11.3 BudgetClass::Corpus law)",
            compiled.id()
        ));
        return Err(Box::new(e));
    }
    Ok(compiled)
}
