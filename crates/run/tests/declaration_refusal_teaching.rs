//! r3 dogfood declaration-refusal teachings (card `refusal-teaching-gaps-r3`,
//! receipts D-USER f5 + D-PROBER F6 gap 6c).
//!
//! The run plane's declaration refusals must carry the near-miss fact the
//! engine can already measure: `task.<name>.caps/.args/.env` keys ON the page
//! with no `task.<name>` binding. The register law binds — reason first, a
//! measured fact, the engine's own declaration grammar; never an unmeasured
//! remedy. Codes and variants stay typed; teaching text only.

mod support;

use run::address::{self, AddressError};
use support::doc;

/// The r3 D-USER f5 page shape verbatim: the block declarations are present,
/// the binding that names WHICH block runs is not.
const DECLARATION_ONLY: &str = "\
---
task.index.caps: \"md.append_section:Log\"
---

```bash
echo hi
```
^ix-1
";

/// A page with one healthy binding — the control for the no-near-miss arms.
const BOUND: &str = "\
---
task.ok: \"[[#^t-ok]]\"
---

```bash
echo ok
```
^t-ok
";

/// A page whose frontmatter carries no `task.*` keys at all.
const BARE: &str = "---\ntitle: T\n---\n\nprose only\n";

/// f5 shape 1 — TASK named, only its declaration keys exist. The refusal
/// names the measured near-miss and the binding grammar.
#[test]
fn named_task_miss_names_the_declaration_only_keys() {
    let err = address::resolve_task(&doc(DECLARATION_ONLY), Some("index"))
        .expect_err("no binding, only declarations");
    assert!(matches!(err, AddressError::NoTask { .. }));
    let m = err.to_string();
    assert!(
        m.contains("found task.index.caps but no task.index binding"),
        "names the measured near-miss: {m}"
    );
    assert!(
        m.contains("task.index: \"[[#^"),
        "teaches the binding grammar for the asked name: {m}"
    );
}

/// f5 shape 2 — TASK omitted on the same page. "this page declares no tasks"
/// carries the same measured fact instead of standing bare.
#[test]
fn omitted_task_on_a_declaration_only_page_names_the_near_miss() {
    let err = address::resolve_task(&doc(DECLARATION_ONLY), None)
        .expect_err("no binding, only declarations");
    assert!(matches!(err, AddressError::NoTasks { .. }));
    let m = err.to_string();
    assert!(
        m.contains("found task.index.caps but no task.index binding"),
        "names the measured near-miss: {m}"
    );
}

/// Gap 6c control — a page with no `task.*` keys at all still teaches the
/// declaration grammar instead of a bare statement.
#[test]
fn omitted_task_on_a_bare_page_teaches_the_binding_grammar() {
    let err = address::resolve_task(&doc(BARE), None).expect_err("no tasks declared");
    assert!(matches!(err, AddressError::NoTasks { .. }));
    let m = err.to_string();
    assert!(
        m.contains("task.<name>: \"[[#^block-id]]\""),
        "teaches the declaration grammar: {m}"
    );
}

/// Control — a miss beside a healthy binding keeps the declared list and
/// invents no near-miss.
#[test]
fn named_task_miss_without_declaration_keys_keeps_the_declared_list() {
    let err = address::resolve_task(&doc(BOUND), Some("nope")).expect_err("no such task");
    let m = err.to_string();
    assert!(m.contains("declared: ok"), "keeps the declared list: {m}");
    assert!(
        !m.contains("found task."),
        "invents no near-miss where none was measured: {m}"
    );
}
