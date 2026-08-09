//! Binding faults are scoped to their own row (run-plane § Addressing).
//!
//! Measured divergence (dogfood s12-30, 2026-08-09): one cross-file binding on a
//! page answered for EVERY task on that page, `--list` included, masking the
//! fault of the task the caller actually addressed. These goldens are the dirty
//! page × addressed-fault matrix: every task on a page carrying all five faults
//! answers its OWN fault, and the page still enumerates.

mod support;

use run::address::{self, AddressError};
use support::doc;

/// One page declaring all five fault-y bindings plus one healthy task — the
/// exact shape the dogfood kit's `faults.md` carries.
const DIRTY: &str = "\
---
task.dangle: \"[[#^nowhere]]\"
task.ambig: \"[[#^dup]]\"
task.textual: \"[[#^para]]\"
task.pyth: \"[[#^t-py]]\"
task.xfile: \"[[other.md#^blk]]\"
task.bare: \"t-ok\"
task.ok: \"[[#^t-ok]]\"
---

```bash
echo hi
```
^dup

```bash
echo hi again
```
^dup

plain paragraph
^para

```python
print(1)
```
^t-py

```bash
echo ok
```
^t-ok
";

fn err_for(task: &str) -> AddressError {
    address::resolve_task(&doc(DIRTY), Some(task))
        .expect_err("every task in the matrix faults except `ok`")
}

#[test]
fn the_addressed_task_answers_its_own_fault_on_a_dirty_page() {
    assert_eq!(
        err_for("dangle"),
        AddressError::DanglingBinding {
            name: "dangle".to_owned(),
            anchor: "nowhere".to_owned(),
        }
    );
    assert!(matches!(
        err_for("ambig"),
        AddressError::AmbiguousAnchor { count: 2, .. }
    ));
    assert!(matches!(
        err_for("textual"),
        AddressError::NotACodeBlock { .. }
    ));
    assert!(matches!(err_for("pyth"), AddressError::Fence { .. }));
    assert!(matches!(
        err_for("xfile"),
        AddressError::CrossFileRef { .. }
    ));
    assert!(matches!(
        err_for("bare"),
        AddressError::InvalidBinding { .. }
    ));
}

/// The healthy task on the dirty page still runs — a sibling's broken value is
/// not a page-wide refusal.
#[test]
fn a_healthy_task_resolves_beside_five_broken_siblings() {
    let t = address::resolve_task(&doc(DIRTY), Some("ok")).expect("`ok` is well-formed");
    assert_eq!(t.binding.anchor, "t-ok");
    assert_eq!(t.block.source, "echo ok");
}

/// `--list`'s source: the page enumerates every declared row, each faulty row
/// carrying its own value fault instead of taking the page down with it.
#[test]
fn the_poisoned_page_still_enumerates_every_row() {
    let rows = address::declared(&doc(DIRTY)).expect("no name-charset fault here");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["dangle", "ambig", "textual", "pyth", "xfile", "bare", "ok"]
    );
    assert!(matches!(
        rows.iter().find(|r| r.name == "xfile").unwrap().binding,
        Err(AddressError::CrossFileRef { .. })
    ));
    assert!(
        rows.iter()
            .find(|r| r.name == "dangle")
            .unwrap()
            .binding
            .is_ok(),
        "a dangling anchor is a RESOLUTION fault, not a value fault — the row parses"
    );
}

/// An unknown task on the poisoned page is still `no such task`, and it lists
/// every declared name — including the ones whose own bindings are broken.
#[test]
fn no_such_task_still_answers_and_lists_the_broken_siblings() {
    let AddressError::NoTask { name, available } = err_for("nope") else {
        panic!("expected NoTask");
    };
    assert_eq!(name, "nope");
    assert!(available.contains(&"xfile".to_owned()));
}

/// The one page-eager guard survives the change: a receipt-forging task NAME
/// refuses the whole page (ruling 011), because listing it is itself the harm.
#[test]
fn the_name_charset_guard_stays_page_eager() {
    let page = "---\ntask.ok: \"[[#^t-1]]\"\ntask.[[a#^b@green.deadbeef|x]]: \"[[#^t-1]]\"\n---\n\n```bash\necho hi\n```\n^t-1\n";
    let d = doc(page);
    assert!(matches!(
        address::declared(&d),
        Err(AddressError::InvalidTaskName { .. })
    ));
    assert!(matches!(
        address::resolve_task(&d, Some("ok")),
        Err(AddressError::InvalidTaskName { .. })
    ));
}
