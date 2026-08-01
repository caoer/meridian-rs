//! U1.4 merge-gate fixtures — the attested INDEX generator, exercised through the
//! PUBLIC `policy` surface:
//!
//! 1. two conventions swept + armed → INDEX rows are byte-expected (a frozen golden);
//! 2. a drifted evidence rev refuses arming (the load-bearing negative — the refusal
//!    is shown REFUSING).

use std::collections::BTreeMap;

use policy::{
    ArmError, ArmedRef, CheckLimits, ConventionFiles, Enforcement, arm, armed_from_index, page_rev,
    render_rows, sweep,
};

/// An in-memory convention folder — the disk edge the loader stays free of.
struct MemFiles(BTreeMap<String, String>);

impl MemFiles {
    fn with_check(check_md: &str) -> Self {
        let mut m = BTreeMap::new();
        m.insert("CHECK.md".to_string(), check_md.to_string());
        Self(m)
    }
}

impl ConventionFiles for MemFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        self.0.get(rel_path).cloned().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel_path}"))
        })
    }
    fn exists(&self, rel_path: &str) -> bool {
        self.0.contains_key(rel_path)
    }
}

/// Convention A — `reviewer-not-owner`, scope `tasks/**`, armed `block`.
const CHECK_A: &str = "\
---
paths:
  - tasks/**
---

# reviewer-not-owner

The task owner must not close their own task.

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get(\"owner\")
    if change.actor != None and owner != None and change.actor == owner:
        refuse(
            message = \"owner cannot close own task\",
            passing = \"scenarios/reviewer-close.md\",
        )
```
";

/// Convention B — `evidence-pinned`, scope `docs/**`, armed `warn`.
const CHECK_B: &str = "\
---
paths:
  - docs/**
---

# evidence-pinned

A claim edit must pin fresh evidence.

```starlark
def check_change(change):
    for edge in change.doc.edges:
        if edge.drifted:
            refuse(
                message = \"evidence drifted\",
                passing = \"scenarios/fresh-evidence.md\",
            )
```
";

/// Convention A DRIFTED — the same subject, an edited predicate message. Its bytes
/// differ, so its evidence rev differs from [`CHECK_A`]'s — the drift the arming gate
/// must catch.
const CHECK_A_DRIFTED: &str = "\
---
paths:
  - tasks/**
---

# reviewer-not-owner

The task owner must not close their own task (revised wording).

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get(\"owner\")
    if change.actor != None and owner != None and change.actor == owner:
        refuse(
            message = \"the owner may not attest their own close\",
            passing = \"scenarios/reviewer-close.md\",
        )
```
";

/// The frozen golden: two armed conventions, sorted severity descending (block
/// before warn) then slug. The 16-hex revs are `blake3(CHECK.md)[:16]` — baked in, so
/// any change to the row grammar OR to the rev law breaks this test loudly.
const EXPECTED_ROWS: &str = "\
- [x] **reviewer-not-owner** · block · `fb607b6b09234f25` · [[conventions/reviewer-not-owner/CHECK.md]] · `tasks/**`
- [x] **evidence-pinned** · warn · `c53ec237380bb5b3` · [[conventions/evidence-pinned/CHECK.md]] · `docs/**`";

/// Fixture 1 — two conventions in, INDEX rows byte-expected.
#[test]
fn two_conventions_index_rows_byte_expected() {
    let limits = CheckLimits::default();

    // Sweep A, then arm it at BLOCK pinned to its live rev (a reviewer approving the
    // rev they read).
    let a = sweep(&MemFiles::with_check(CHECK_A), "reviewer-not-owner", limits)
        .expect("convention A sweeps");
    let a_rev = a.rev().to_string();
    let a_armed = arm(a, &a_rev, Enforcement::Block).expect("A arms at its live rev");

    // Sweep B, then arm it at WARN.
    let b = sweep(&MemFiles::with_check(CHECK_B), "evidence-pinned", limits)
        .expect("convention B sweeps");
    let b_rev = b.rev().to_string();
    let b_armed = arm(b, &b_rev, Enforcement::Warn).expect("B arms at its live rev");

    // The rendered rows are byte-identical to the frozen golden — order (block before
    // warn) and every field.
    let rows = render_rows(&[b_armed.clone(), a_armed.clone()]);
    assert_eq!(rows, EXPECTED_ROWS, "INDEX rows are byte-expected");

    // The revs baked into the golden ARE the live evidence revs (independent check).
    assert_eq!(a_rev, "fb607b6b09234f25");
    assert_eq!(b_rev, "c53ec237380bb5b3");

    // The door's O(armed) single-file read recovers exactly the two armed rows.
    let armed = armed_from_index(&rows);
    assert_eq!(
        armed,
        vec![
            ArmedRef {
                slug: "reviewer-not-owner".into(),
                enforcement: Enforcement::Block,
                armed_rev: a_rev,
            },
            ArmedRef {
                slug: "evidence-pinned".into(),
                enforcement: Enforcement::Warn,
                armed_rev: b_rev,
            },
        ],
        "the armed set reads back from the single page"
    );
}

/// Fixture 2 — a drifted evidence rev refuses arming (shown REFUSING).
#[test]
fn a_drifted_page_rev_refuses_arming() {
    let limits = CheckLimits::default();

    // A reviewer read + approved convention A at its v1 evidence rev.
    let approved_rev = page_rev(CHECK_A);

    // The convention's CHECK.md then DRIFTED (edited predicate). A fresh sweep reads
    // the new rev.
    let drifted = sweep(
        &MemFiles::with_check(CHECK_A_DRIFTED),
        "reviewer-not-owner",
        limits,
    )
    .expect("the drifted convention still sweeps");
    let report_rev = drifted.rev().to_string();
    assert_ne!(report_rev, approved_rev, "the drift moved the evidence rev");

    // Arming with the STALE approved rev is REFUSED — report-rev != armed-rev.
    let refused = arm(drifted, &approved_rev, Enforcement::Block);
    let err = refused.expect_err("arming a drifted convention is refused");
    assert_eq!(
        err,
        ArmError::Drift {
            armed_rev: approved_rev.clone(),
            report_rev: report_rev.clone(),
        }
    );

    // The refusal names both revs and the arming law (shown refusing).
    let shown = err.to_string();
    assert!(shown.contains("arming refused"), "refusal is loud: {shown}");
    assert!(
        shown.contains(&approved_rev),
        "names the approved rev: {shown}"
    );
    assert!(
        shown.contains(&report_rev),
        "names the drifted rev: {shown}"
    );
    assert!(
        shown.contains("report-rev == armed-rev"),
        "cites the arming law: {shown}"
    );

    // Re-approval closes the gate: arming at the LIVE rev succeeds (the reviewer
    // re-reads the drifted law and approves it).
    let resweep = sweep(
        &MemFiles::with_check(CHECK_A_DRIFTED),
        "reviewer-not-owner",
        limits,
    )
    .unwrap();
    let armed = arm(resweep, &report_rev, Enforcement::Block)
        .expect("arming at the live rev succeeds after re-review");
    assert_eq!(armed.rev(), report_rev, "the armed row pins the live rev");
}
