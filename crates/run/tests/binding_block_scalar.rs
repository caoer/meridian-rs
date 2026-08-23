//! **Why the binding plane keeps its trim** — card
//! `scalar-text-trims-config-key-block-scalars`.
//!
//! That card routed three flat-map readers through `model::fm_doc_publish` so
//! a block scalar stops being trimmed at a value-publishing seam. This is the
//! FOURTH site it named, `run::address`'s `unquote`, and it was audited in the
//! same pass and deliberately LEFT ALONE. This file is why.
//!
//! The card's own framing was that a block scalar here "refuses rather than
//! mis-reads", so the site needed only a comment. The refusal verdict is
//! right, but the reasoning is incomplete in a way that matters: routing this
//! site through the seam would not harden it, it would REGRESS it. A folded
//! binding works today precisely BECAUSE of the trim, and publishing verbatim
//! turns that working page into a typed refusal.
//!
//! Mechanism. `task.build: >` over `[[#^build]]` is stored already decoded as
//! `"[[#^build]]\n"` — clip chomping's trailing break. `parse_binding_value`
//! strips `[[` and `]]` as a MATCHED pair. Trimmed, the suffix strip matches
//! and the binding resolves. Untrimmed, `strip_suffix("]]")` misses the
//! newline, the whole `"[[#^build]]\n"` reaches `split_once("#^")`, and the
//! non-empty target `"[["` refuses `CrossFileRef`. The later `v.trim()` does
//! not rescue it — that runs AFTER the bracket strip has already failed.
//!
//! So the binding plane is not a value-publishing seam. It reads a value whose
//! grammar is `[[#^id]]`, where surrounding whitespace is never content.

mod support;

use run::address::{self, AddressError};
use support::doc;

/// A page binding its task through a FOLDED block scalar. Valid YAML, and the
/// shape that breaks if this site is "fixed".
const FOLDED_BINDING: &str = "\
---
task.build: >
  [[#^build-1]]
---

# Tasks

```bash
echo hi
```
^build-1
";

/// The same binding as a LITERAL block scalar with strip chomping — no
/// trailing break at all, so it survives either way. Kept as the control: it
/// proves the fixture pair differs only in the byte the trim removes.
const LITERAL_STRIP_BINDING: &str = "\
---
task.build: |-
  [[#^build-1]]
---

# Tasks

```bash
echo hi
```
^build-1
";

/// The oracle: `PyYAML` on the frontmatter block, so the claim "the stored value
/// carries a trailing newline" is YAML's, not the engine's.
fn pyyaml(raw: &str, key: &str) -> String {
    let fm = raw
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(fm, _)| format!("{fm}\n")))
        .expect("the fixture carries frontmatter");
    let out = std::process::Command::new("python3")
        .args([
            "-c",
            "import sys, yaml\nd = yaml.safe_load(sys.argv[1])\nsys.stdout.write(d[sys.argv[2]])\n",
            &fm,
            key,
        ])
        .output()
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "PyYAML rejects the fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// The premise, from the foreign parser: the folded binding's value really
/// does carry the trailing break, and the strip variant really does not. If
/// this ever fails, the regression test below has gone vacuous.
#[test]
fn pyyaml_confirms_the_folded_binding_carries_a_trailing_break() {
    assert_eq!(pyyaml(FOLDED_BINDING, "task.build"), "[[#^build-1]]\n");
    assert_eq!(pyyaml(LITERAL_STRIP_BINDING, "task.build"), "[[#^build-1]]");
}

/// **The regression this site is protected against.** Both block-scalar
/// bindings resolve to their fenced block. Route `unquote` through
/// `fm_doc_publish` and the folded case becomes `CrossFileRef`.
#[test]
fn a_block_scalar_binding_resolves_to_its_block() {
    for (name, page) in [
        ("folded clip", FOLDED_BINDING),
        ("literal strip", LITERAL_STRIP_BINDING),
    ] {
        let d = doc(page);
        let t = address::resolve_task(&d, Some("build"))
            .unwrap_or_else(|e| panic!("{name}: expected the binding to resolve, got {e:?}"));
        assert_eq!(t.binding.name, "build", "{name}");
        assert_eq!(t.binding.anchor, "build-1", "{name}");
        assert_eq!(t.block.source, "echo hi", "{name}");
    }
}

/// The failure the trim prevents, demonstrated directly rather than asserted
/// about: feed the parser the UNTRIMMED bytes and watch it refuse. This is the
/// evidence for the doc comment on `unquote` — without it, "the trim is
/// load-bearing" is a claim about code nobody ran.
#[test]
fn the_untrimmed_value_is_what_would_refuse() {
    // What `fm_doc_publish` would hand the parser for the folded fixture.
    let published = pyyaml(FOLDED_BINDING, "task.build");
    assert!(
        published.ends_with("]]\n"),
        "the published form keeps the break: {published:?}"
    );
    assert!(
        published
            .strip_prefix("[[")
            .is_some_and(|s| s.strip_suffix("]]").is_none()),
        "the matched-pair strip cannot see past the trailing break — this is the regression"
    );

    // And the trimmed form, which is what the site actually uses, is clean.
    let trimmed = published.trim();
    assert_eq!(
        trimmed
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]")),
        Some("#^build-1"),
        "trimmed, the pair strips and the block ref is reachable"
    );
}

/// A block scalar that is NOT a block ref still refuses with the grammar's own
/// typed error — the card's "refuses rather than mis-reads" half, kept as an
/// assertion rather than a claim.
#[test]
fn a_block_scalar_that_is_not_a_block_ref_still_refuses() {
    let page = "\
---
task.build: |
  not a block ref
---

# Tasks

```bash
echo hi
```
^build-1
";
    let d = doc(page);
    let err = address::resolve_task(&d, Some("build")).expect_err("must refuse");
    assert!(
        matches!(err, AddressError::InvalidBinding { .. }),
        "expected InvalidBinding, got {err:?}"
    );
}

/// **The pin the prose leaned on without a test** (card
/// `wire-contract-go-yaml-11-claim-overstated`, PR 194 review N1): a LITERAL
/// block scalar carrying TWO refs.
///
/// This is the case that makes "a block scalar here refuses rather than
/// mis-reads" true rather than lucky. The stored value is
/// `"[[#^build-1]]\n[[#^build-2]]\n"`; the trim removes the clip break, and
/// then `strip_prefix("[[")` and `strip_suffix("]]")` BOTH match — they are a
/// matched pair around the whole two-line value — so the bracket strip
/// succeeds and hands `split_once("#^")` an inner value spanning both refs.
/// The target half is empty, so `CrossFileRef` does not fire either. What
/// refuses is the LAST guard: the block id
/// `build-1]]\n[[#^build-2` is outside `[A-Za-z0-9-]`
/// (`syntax::is_block_id`, reached at `address::parse_binding_value`'s charset
/// arm). Nothing before it says no, which is why the guard is load-bearing and
/// why this fixture exists: a "harmless" widening of the charset would turn
/// this page into a binding that silently resolves to ONE of two blocks.
#[test]
fn a_literal_block_with_two_refs_refuses_invalid_binding() {
    let page = "\
---
task.build: |
  [[#^build-1]]
  [[#^build-2]]
---

# Tasks

```bash
echo one
```
^build-1

```bash
echo two
```
^build-2
";
    let d = doc(page);
    let err = address::resolve_task(&d, Some("build")).expect_err("must refuse");
    match &err {
        AddressError::InvalidBinding { reason, value, .. } => {
            assert!(
                reason.contains("charset"),
                "the charset guard is what refuses this, not the bracket strip: {reason}"
            );
            assert!(
                value.contains("build-1") && value.contains("build-2"),
                "the refusal must carry the value the author wrote: {value}"
            );
        }
        other => panic!("expected InvalidBinding, got {other:?}"),
    }
}
