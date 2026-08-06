//! Seam bench: `policy::evaluate` — per-eval wall-time p99 + declared budgets.
//!
//! Budget enforcement splits in two (ruling 008), mirroring this harness's
//! criterion-vs-hdr split:
//!
//! - Step/mem budgets — deterministic post-eval accounting: same bytes + same
//!   facts ⇒ same count. Platform-free, so the budget *envelope* is data
//!   (staged as a claim value) and budget *conformance* is a test/finding,
//!   not a wall-time bench.
//! - Wall-time p99 — real latency of one `policy::evaluate` call on real
//!   hardware: this bench's hdr job, on the fleet runner.
//!
//! # Metric contract (claims.toml)
//! - `policy.evaluate.p99.{vault_1gb,fence_bomb}` — µs per `policy::evaluate`
//!   over one real `model::build` AST, p99 via the hdr path.
//!   `threshold_source = "ruleset"`: gates come from each rule's manifest
//!   budget, never from claims.toml — a local number renders MEASURED, never
//!   a false PASS/FAIL.
//! - `policy.eval.budget.{steps,mem}.vault_1gb` — the `EvalBudget` the
//!   compiled pack was admitted under, staged as data; runtime grounding
//!   logged: how many evals emitted a `budget_exceeded` finding.
//! - `policy.assertion.p99` / `policy.pack_load.fixtures` — the generic
//!   per-rule and pack-load claims; their gates land with `policy::vocab()`.
//!
//! The in-process Starlark step/heap guards are best-effort (they stop a
//! runaway, not a single allocating expression); the hard bound is
//! subprocess/OS limits — a daemon concern. Nothing here asserts a hard
//! in-process bound.

#![allow(
    clippy::cast_precision_loss,
    reason = "budget values (10k steps, 4 MiB) stage as f64 Measurement.value; exact in f64 — same latitude the perfsuite lib takes for stats plumbing"
)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::io;
use std::io::Write as _;

use criterion::{Criterion, criterion_group};

use model::Document;
use policy::{CompiledRuleset, PackFiles, RulesetPin, compile, evaluate};

use perfsuite::corpus;
use perfsuite::measure::{LatencyRun, Measurement, record_measurement};
use perfsuite::profile::{Profile, Recipe};

/// The canonical `blurb-required` rule page (a literate page whose predicate is a
/// fenced Starlark `def check(doc)`), matching the policy crate's own unit fixture —
/// so this bench evaluates the exact predicate the load gate admits.
const BLURB_RULE_PAGE: &str = "\
# blurb-required

Every section must open with a blurb line: a heading immediately followed by
another heading (or the end of the document) has no blurb and is a violation.

```starlark
def check(doc):
    nodes = doc.nodes
    count = len(nodes)
    for i in range(count):
        node = nodes[i]
        if node.kind != \"heading\":
            continue
        has_blurb = i + 1 < count and nodes[i + 1].kind != \"heading\"
        if not has_blurb:
            violation(
                rule = \"blurb-required\",
                severity = \"warn\",
                span = node.span,
                node_rev = node.node_rev,
                hpath = node.hpath,
                message = \"section has no blurb line\",
            )
```
";

/// The §11.3 manifest: `rulepack-api@1`, the shipped `{steps, mem}` envelope
/// (10k steps, 4 MiB — same as the policy crate's shipped fixtures), one fixture
/// (the load gate) and the one rule page.
const MANIFEST: &str = "\
id: perf-blurb
api: rulepack-api@1
budgets:
  steps: 10000
  mem: 4194304
fixtures:
  - fixtures/pass.md
rules:
  - rules/blurb-required.md
";

/// The load-gate fixture: a conforming doc declared `pass`. The blurb is a real
/// dialect node (a wikilink) — the world model is paragraph-blind, so a bare prose
/// line would not be a node and the heading would fire (P6-VERDICTS load-gate
/// unification: fixtures demonstrate over the SAME plane `evaluate` runs).
const PASS_FIXTURE: &str = "---\nexpect: pass\n---\n# Goals\n\n[[intro]]\n";

/// In-memory pack-file resolver — policy stays I/O-free; the caller injects file
/// access (here, the two manifest-relative pack files).
struct MemFiles(BTreeMap<String, String>);

impl PackFiles for MemFiles {
    fn read(&self, rel_path: &str) -> io::Result<String> {
        self.0
            .get(rel_path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, rel_path.to_owned()))
    }
}

/// The real parse→facts builder the serving host injects into the load gate — fixtures
/// demonstrate over the SAME fact plane `evaluate` runs (P6-VERDICTS unification);
/// the signature stays in policy vocabulary (no `syntax::`/`model::` type crosses it).
fn real_facts(path: &str, body: &str) -> policy::FactDoc {
    let mut doc = model::build(body.to_string(), syntax::parse(body));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    policy::facts_from_document(&doc)
}

/// Compile the blurb pack through the real `policy::compile` load gate (fixtures run
/// under the declared budget) — the admitted `CompiledRuleset` this bench evaluates.
fn compiled_pack() -> CompiledRuleset {
    let files = MemFiles(BTreeMap::from([
        (
            "rules/blurb-required.md".to_owned(),
            BLURB_RULE_PAGE.to_owned(),
        ),
        ("fixtures/pass.md".to_owned(), PASS_FIXTURE.to_owned()),
    ]));
    let pin = RulesetPin {
        id: "perf-blurb".to_owned(),
        path: "perf-blurb.md".to_owned(),
        sha256: None,
    };
    compile(&pin, MANIFEST, &files, &real_facts)
        .expect("perf blurb pack compiles under its load gate")
}

/// Generate-or-reuse a bounded sample from a profile's SHAPE (per-document eval
/// latency depends on document shape, not corpus size), then build a real
/// `model::Document` AST per file via `syntax::parse` → `model::build`.
fn sample_docs(profile_name: &str, files: u32, seed: u64) -> Vec<Document> {
    let profile = Profile::resolve(profile_name).unwrap_or_else(|e| panic!("{profile_name}: {e}"));
    let recipe = Recipe::new(profile, seed, Some(files));
    let dir = corpus::ensure(&recipe).expect("corpus generate-or-reuse");
    corpus::load_files(&dir)
        .expect("load corpus files")
        .into_iter()
        .map(|(_rel, content)| model::build(content.clone(), syntax::parse(&content)))
        .collect()
}

/// Criterion seam: batch `policy::evaluate` over a real vault-1gb-shaped sample —
/// the mean-estimate companion to the hdr p99 claim below.
fn bench_policy_eval(c: &mut Criterion) {
    let pack = compiled_pack();
    let docs = sample_docs("vault-1gb", 200, 1);
    let refs: Vec<&Document> = docs.iter().collect();
    c.bench_function("policy/evaluate_vault1gb_batch", |b| {
        b.iter(|| {
            let mut n = 0usize;
            for doc in &refs {
                n += evaluate(&pack, std::slice::from_ref(doc), None).len();
            }
            black_box(n)
        });
    });
}

/// hdr p99 of one `policy::evaluate` call, cycling through the sample docs.
fn eval_p99_measurement(pack: &CompiledRuleset, docs: &[Document], id: &str) -> Measurement {
    let refs: Vec<&Document> = docs.iter().collect();
    let mut i = 0usize;
    let run = LatencyRun::run(500, 8_000, || {
        let doc = refs[i % refs.len()];
        i += 1;
        let v = evaluate(pack, std::slice::from_ref(&doc), None);
        black_box(&v);
    });
    run.to_p99_measurement(id, "us")
}

/// Stage one eval-p99 claim and log the headline.
fn record_eval_p99(pack: &CompiledRuleset, docs: &[Document], id: &str) {
    let m = eval_p99_measurement(pack, docs, id);
    match record_measurement(&m) {
        Ok(path) => {
            let _ = writeln!(
                io::stderr(),
                "claim {id}: p99 {:.2} µs over {} samples ({} docs) → {}",
                m.value,
                m.samples,
                docs.len(),
                path.display()
            );
        }
        Err(e) => eprintln!("claim {id} staging failed: {e}"),
    }
}

/// Stage the declared per-eval budget as data (`EvalBudget{steps,mem}` the pack was
/// admitted under), with the runtime grounding: how many of the sample's evals
/// emitted a `budget_exceeded` finding (0 ⇒ the declared envelope holds at scale).
fn record_budget(pack: &CompiledRuleset, docs: &[Document], steps_id: &str, mem_id: &str) {
    let budget = pack.budget();
    let refs: Vec<&Document> = docs.iter().collect();
    let exhausted = refs
        .iter()
        .filter(|doc| {
            evaluate(pack, std::slice::from_ref(*doc), None)
                .iter()
                .any(|v| v.message.starts_with("budget_exceeded:"))
        })
        .count();

    for (id, value, unit) in [
        (steps_id, budget.steps as f64, "steps"),
        (mem_id, budget.mem as f64, "bytes"),
    ] {
        let m = Measurement {
            id: id.to_owned(),
            value,
            unit: unit.to_owned(),
            samples: refs.len() as u64,
            dist: None,
        };
        match record_measurement(&m) {
            Ok(path) => {
                let _ = writeln!(
                    io::stderr(),
                    "claim {id}: declared {value:.0} {unit}; {exhausted}/{} evals exhausted → {}",
                    refs.len(),
                    path.display()
                );
            }
            Err(e) => eprintln!("claim {id} staging failed: {e}"),
        }
    }
}

criterion_group!(benches, bench_policy_eval);

fn main() {
    benches();
    Criterion::default().configure_from_args().final_summary();

    let pack = compiled_pack();
    let vault = sample_docs("vault-1gb", 200, 1);
    let fence = sample_docs("fence-bomb", 100, 1);

    record_eval_p99(&pack, &vault, "policy.evaluate.p99.vault_1gb");
    record_eval_p99(&pack, &fence, "policy.evaluate.p99.fence_bomb");
    record_budget(
        &pack,
        &vault,
        "policy.eval.budget.steps.vault_1gb",
        "policy.eval.budget.mem.vault_1gb",
    );
}
