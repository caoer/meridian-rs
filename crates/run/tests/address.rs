//! Addressing + fence gates: `fm_key`→block resolution, the distinct typed
//! errors (dangling ≠ no-task), TASK-omitted arity, cross-file NON-GOAL, and
//! fence-language classification.

mod support;

use run::address::{self, AddressError};
use run::fence::{self, FenceError, TaskLanguage};
use support::{PAGE, code_block_span, doc};

#[test]
fn named_task_resolves_to_its_fenced_block() {
    let d = doc(PAGE);
    let t = address::resolve_task(&d, Some("fix-drift")).unwrap();
    assert_eq!(t.binding.name, "fix-drift");
    assert_eq!(t.binding.anchor, "fix-1");
    assert_eq!(t.block.lang, TaskLanguage::Bash);
    assert_eq!(t.block.source, "echo hi");
}

#[test]
fn starlark_task_classifies_hermetic_language() {
    let d = doc(PAGE);
    let t = address::resolve_task(&d, Some("check-links")).unwrap();
    assert_eq!(t.block.lang, TaskLanguage::Starlark);
    assert_eq!(t.block.source, "def run(ctx):\n    pass");
}

#[test]
fn missing_task_is_no_task_with_the_declared_list() {
    let d = doc(PAGE);
    let err = address::resolve_task(&d, Some("nope")).unwrap_err();
    let AddressError::NoTask {
        name, available, ..
    } = err
    else {
        panic!("expected NoTask, got {err:?}");
    };
    assert_eq!(name, "nope");
    assert!(available.contains(&"fix-drift".to_owned()));
}

#[test]
fn dangling_binding_is_distinct_from_no_task() {
    // The fm key EXISTS but its block does not — must not report NoTask.
    let d = doc(PAGE);
    let err = address::resolve_task(&d, Some("dangling")).unwrap_err();
    assert_eq!(
        err,
        AddressError::DanglingBinding {
            name: "dangling".to_owned(),
            anchor: "gone".to_owned(),
        }
    );
}

#[test]
fn task_omitted_with_many_bindings_lists_and_refuses() {
    let d = doc(PAGE);
    let err = address::resolve_task(&d, None).unwrap_err();
    let AddressError::ManyTasks { available } = err else {
        panic!("expected ManyTasks, got {err:?}");
    };
    assert_eq!(available.len(), 4, "{available:?}");
}

/// The 2026-08-19 default-task election: several bindings, one named
/// `default` — TASK omitted runs it instead of the list-and-refuse.
#[test]
fn task_omitted_with_many_bindings_runs_the_default_election() {
    let page = "\
---
task.default: \"[[#^d-1]]\"
task.other: \"[[#^o-1]]\"
---

```bash
true
```
^d-1

```bash
false
```
^o-1
";
    let t = address::resolve_task(&doc(page), None).unwrap();
    assert_eq!(t.binding.name, "default");
    assert_eq!(t.binding.anchor, "d-1");
}

/// An explicit TASK always wins — the election only fills an omitted TASK.
#[test]
fn named_task_wins_over_the_default_election() {
    let page = "\
---
task.default: \"[[#^d-1]]\"
task.other: \"[[#^o-1]]\"
---

```bash
true
```
^d-1

```bash
false
```
^o-1
";
    let t = address::resolve_task(&doc(page), Some("other")).unwrap();
    assert_eq!(t.binding.name, "other");
    assert_eq!(t.binding.anchor, "o-1");
}

/// A broken `default` binding surfaces its own row-scoped fault when elected —
/// the election addresses the row exactly as naming it would.
#[test]
fn elected_default_with_dangling_block_is_dangling_binding() {
    let page = "\
---
task.default: \"[[#^gone]]\"
task.other: \"[[#^o-1]]\"
---

```bash
false
```
^o-1
";
    let err = address::resolve_task(&doc(page), None).unwrap_err();
    assert_eq!(
        err,
        AddressError::DanglingBinding {
            name: "default".to_owned(),
            anchor: "gone".to_owned(),
        }
    );
}

#[test]
fn task_omitted_with_one_binding_runs_it() {
    let page = "\
---
task.only: \"[[#^t-1]]\"
---

```bash
true
```
^t-1
";
    let t = address::resolve_task(&doc(page), None).unwrap();
    assert_eq!(t.binding.name, "only");
}

#[test]
fn task_omitted_with_no_bindings_is_no_tasks() {
    let err = address::resolve_task(&doc("# empty\n"), None).unwrap_err();
    assert_eq!(
        err,
        AddressError::NoTasks {
            declaration_keys: Vec::new(),
        }
    );
}

#[test]
fn cross_file_ref_is_a_typed_non_goal_refusal() {
    let page = "---\ntask.remote: \"[[other#^t-1]]\"\n---\n";
    let err = address::resolve_task(&doc(page), Some("remote")).unwrap_err();
    let AddressError::CrossFileRef { name, .. } = err else {
        panic!("expected CrossFileRef, got {err:?}");
    };
    assert_eq!(name, "remote");
}

#[test]
fn non_block_ref_value_is_invalid_binding() {
    let page = "---\ntask.bad: \"[[#Heading]]\"\n---\n";
    let err = address::resolve_task(&doc(page), Some("bad")).unwrap_err();
    assert!(
        matches!(err, AddressError::InvalidBinding { ref name, .. } if name == "bad"),
        "{err:?}"
    );
}

#[test]
fn binding_to_a_non_code_block_is_typed() {
    let d = doc(PAGE);
    let err = address::resolve_task(&d, Some("bad-block")).unwrap_err();
    assert_eq!(
        err,
        AddressError::NotACodeBlock {
            name: "bad-block".to_owned(),
            anchor: "para-1".to_owned(),
        }
    );
}

#[test]
fn duplicate_anchor_is_ambiguous_never_picked() {
    let page = "\
---
task.dup: \"[[#^d-1]]\"
---

```bash
a
```
^d-1

```bash
b
```
^d-1
";
    let err = address::resolve_task(&doc(page), Some("dup")).unwrap_err();
    assert_eq!(
        err,
        AddressError::AmbiguousAnchor {
            name: "dup".to_owned(),
            anchor: "d-1".to_owned(),
            count: 2,
        }
    );
}

#[test]
fn bare_linktext_and_alias_sugar_both_resolve() {
    let page = "\
---
task.bare: \"#^t-1\"
task.alias: \"[[#^t-1|the task]]\"
---

```bash
true
```
^t-1
";
    let d = doc(page);
    assert_eq!(
        address::resolve_task(&d, Some("bare"))
            .unwrap()
            .binding
            .anchor,
        "t-1"
    );
    assert_eq!(
        address::resolve_task(&d, Some("alias"))
            .unwrap()
            .binding
            .anchor,
        "t-1"
    );
}

#[test]
fn sub_key_declarations_are_not_bindings() {
    let d = doc(PAGE);
    let names: Vec<String> = address::declared(&d)
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect();
    let reserved = address::RESERVED_SUFFIXES;
    assert!(
        !names
            .iter()
            .any(|n| reserved.iter().any(|s| n.split('.').next_back() == Some(s)))
    );
    assert_eq!(names.len(), 4);
}

#[test]
fn unknown_fence_language_is_a_typed_load_error() {
    let page = "\
---
task.py: \"[[#^p-1]]\"
---

```python
print(1)
```
^p-1
";
    let err = address::resolve_task(&doc(page), Some("py")).unwrap_err();
    assert_eq!(
        err,
        AddressError::Fence {
            name: "py".to_owned(),
            error: FenceError::UnknownLanguage {
                lang: "python".to_owned()
            },
        }
    );
}

#[test]
fn languageless_fence_is_refused() {
    let page = "\
---
task.plain: \"[[#^p-1]]\"
---

```
whatever
```
^p-1
";
    let err = address::resolve_task(&doc(page), Some("plain")).unwrap_err();
    assert_eq!(
        err,
        AddressError::Fence {
            name: "plain".to_owned(),
            error: FenceError::UnknownLanguage {
                lang: String::new()
            },
        }
    );
}

#[test]
fn unterminated_fence_is_refused() {
    let page = "# t\n\n```bash\necho unclosed\n";
    let d = doc(page);
    let span = code_block_span(&d, "unclosed");
    assert_eq!(
        fence::classify(&d, &span).unwrap_err(),
        FenceError::Unterminated
    );
}

#[test]
fn load_page_splits_not_found_from_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());

    let err = address::load_page(&root, std::path::Path::new("absent.md")).unwrap_err();
    assert!(matches!(err, AddressError::PageNotFound { .. }), "{err:?}");

    std::fs::write(tmp.path().join("bad.md"), [0xFF, 0xFE, 0x00]).unwrap();
    let err = address::load_page(&root, std::path::Path::new("bad.md")).unwrap_err();
    assert!(matches!(err, AddressError::PageInvalid { .. }), "{err:?}");
}
