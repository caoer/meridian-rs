//! The policy composition edge (P6-VERDICTS, advisor Ruling 1): the ONE place the
//! sidecar composes `policy` + the real `syntax`→`model` parse pipeline. Two jobs:
//!
//! - **Admission** ([`admit`]): compile a rule pack through policy's load gate with
//!   the REAL parse→facts builder injected — fixtures demonstrate over the SAME
//!   fact plane production [`evaluate_verdicts`] runs, so no synthetic drift can
//!   admit a pack the wire never reproduces — then refuse a corpus-class pack LOUD
//!   (`daemon_only`): a sidecar-mode engine holds no resident corpus name index
//!   (§8/§11.3 `BudgetClass::Corpus` law).
//! - **Evaluation** ([`evaluate_verdicts`]): the splice arm's ONE production
//!   `policy::evaluate` call site — run every admitted pack over the touched doc's
//!   post-batch state and project the findings into `wire::Verdict` (§11.1).
//!
//! The dependency edge runs ONE way (sidecar → policy): policy names no wire type
//! and gains no new dep (it consumes supplied facts). `wire::Severity` and
//! `policy::Severity` are distinct enums projected here, so the fence holds at the
//! type level too.

use wire::{ErrorBody, ErrorCode, HpathSeg, NodeRev, Path, Severity, Span, Verdict};

/// The real parse→facts builder injected into policy's load gate: fixture bytes →
/// world-model facts through `syntax::parse` → `model::build` → `facts_from_document`
/// — the SAME plane `evaluate` runs. Stamps the fixture path (`model::build` is
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

/// The splice arm's ONE production `policy::evaluate` call site (advisor Ruling 3 —
/// the checkable form of the non-divergence claim): run every admitted pack over the
/// touched doc's post-batch state and project the §11.1 findings to `wire::Verdict`.
/// `corpus` is `None` — sidecar mode holds no resident index, and every admitted pack
/// is node/file-class (corpus-class was refused at admission). Dry and real share this
/// call over the SAME simulated after-doc, so their verdict sets are byte-identical by
/// construction (advisor Ruling 2).
pub(crate) fn evaluate_verdicts(
    rulesets: &[policy::CompiledRuleset],
    after_doc: &model::Document,
) -> Vec<Verdict> {
    let docs = std::slice::from_ref(&after_doc);
    rulesets
        .iter()
        .flat_map(|rs| policy::evaluate(rs, docs, None))
        .map(violation_to_verdict)
        .collect()
}

/// Project one `policy::Violation` into a `wire::Verdict` (§11.1) — findings in THE
/// grammar: hpath strings become `{h, n:None}` segments (§2.1), byte span →
/// `[u64,u64]`. `wire::Severity` is a distinct enum (no wire→policy edge).
fn violation_to_verdict(v: policy::Violation) -> Verdict {
    Verdict {
        rule: v.rule,
        severity: match v.severity {
            policy::Severity::Error => Severity::Error,
            policy::Severity::Warn => Severity::Warn,
            policy::Severity::Info => Severity::Info,
        },
        path: Path(v.path),
        hpath: v
            .hpath
            .map(|segs| segs.into_iter().map(|h| HpathSeg { h, n: None }).collect()),
        span: Span(v.span.start as u64, v.span.end as u64),
        node_rev: NodeRev(v.node_rev.0),
        message: v.message,
    }
}
