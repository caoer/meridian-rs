//! Policy admission (sidecar-mode): compile packs through the load gate over
//! the real `syntax`→`model` pipeline, then refuse corpus-class LOUD
//! (`daemon_only` — no resident corpus name index, §8/§11.3).
//!
//! Evaluation lives on the shared write path
//! (`wire_serve::write::evaluate_verdicts`). Admission stays here: the
//! `daemon_only` refusal is sidecar-specific. Dep edge is one-way
//! (sidecar → policy).

use wire::{ErrorBody, ErrorCode};

/// Real parse→facts for the load gate: `syntax::parse` → `model::build` →
/// `facts_from_document` (same plane as evaluation). Stamps fixture path.
/// Policy vocabulary only — no `syntax`/`model` types in policy signatures.
fn real_facts(path: &str, body: &str) -> policy::FactDoc {
    let mut doc = model::build(body.to_string(), syntax::parse(body));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    policy::facts_from_document(&doc)
}

/// Admit a pack sidecar-mode. Real load gate, then §8/§11.3: corpus-class →
/// `daemon_only` (needs resident index). Node/file-class admit; §4 single-file
/// ops are disk-only so no other class can need corpus (§10.3).
///
/// # Errors
/// [`ErrorCode::DaemonOnly`] for corpus-class; else wire envelope for compile
/// refusal (pack config is the daemon's — `internal` until Go-side admission).
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
