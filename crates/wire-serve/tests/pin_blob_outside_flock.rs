//! A pin's blob oid is asked of git BEFORE the write flock (`wire-contract.md`
//! § A.3, the blob hash): git runs the path's clean filter, a child process
//! whose run time the engine does not own, so it never runs inside the door.
//!
//! Each fixture is a git workspace whose every path goes through a clean
//! filter the test controls. Drives `write::splice` in-process.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, PinSpec, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";
const TARGET: &str = "# Guide\n\n## Rule\n\nreview before you close.\n";
const OTHER: &str = "# Notes\n\nship by w0\n";

/// A git workspace whose clean filter is the shell snippet `clean`.
fn workspace(clean: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, body) in [
        ("plan.md", PINNER),
        ("guide.md", TARGET),
        ("other.md", OTHER),
    ] {
        std::fs::write(dir.path().join(name), body).expect("fixture page");
    }
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "pin@example.invalid"],
        vec!["config", "user.name", "pin"],
        vec!["config", "filter.held.clean", clean],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    std::fs::write(dir.path().join(".git/info/attributes"), "* filter=held\n").expect("attributes");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn args(path: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: None,
        fields: BTreeMap::default(),
    }
}

/// `plan.md` pins `guide.md § Guide/Rule`, pin-only.
fn pin_args() -> SpliceArgs {
    SpliceArgs {
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: wire::ReadSel::parse("Guide/Rule"),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
            retire: None,
        }),
        ..args("plan.md")
    }
}

/// An ordinary edit of `other.md` — the competing writer.
fn edit_args() -> SpliceArgs {
    SpliceArgs {
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![HpathSeg {
                    h: "Notes".into(),
                    n: None,
                }],
            },
            edit: EditShape::Match {
                old: "w0".into(),
                new: "w1".into(),
            },
            if_node_rev: None,
        }],
        ..args("other.md")
    }
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// While git's clean filter is still running for a pin, another writer takes
/// the write flock and lands. The filter here blocks until the test lets it
/// go, so "still running" is a fact, not a sleep.
#[test]
fn a_write_lands_while_a_pin_waits_on_gits_filter() {
    let gate = tempfile::tempdir().expect("gate dir");
    let started = gate.path().join("started");
    let release = gate.path().join("release");
    let (_dir, root) = workspace(&format!(
        "touch '{}'; while [ ! -e '{}' ]; do sleep 0.02; done; cat",
        started.display(),
        release.display()
    ));

    let pinning = {
        let root = root.clone();
        std::thread::spawn(move || splice(&root, None, &pin_args(), &[], None))
    };
    wait_for(&started);
    let competing = splice(&root, None, &edit_args(), &[], None);
    std::fs::write(&release, "").expect("release the filter");
    let pinned = pinning.join().expect("the pin thread ran");

    competing.expect("a writer is admitted while the pin's git child runs");
    assert!(read(&root, "other.md").contains("ship by w1"));
    let ResponseBody::Splice { pin, .. } = pinned.expect("the pin lands once git answers").body
    else {
        panic!("splice body");
    };
    assert!(pin.expect("a pin fact").blob.is_some());
    assert!(read(&root, "plan.md").contains("meridian-lock"));
}

/// While one pin waits on git, a second pin into the same root is refused
/// `workspace_busy` without starting a git child of its own. The second pin
/// runs on its own thread: were it to ask git, its filter would block on the
/// same gate and no answer would arrive before the deadline.
#[test]
fn a_second_pin_is_refused_while_the_first_waits_on_git() {
    let gate = tempfile::tempdir().expect("gate dir");
    let started = gate.path().join("started");
    let release = gate.path().join("release");
    let (_dir, root) = workspace(&format!(
        "touch '{}'; while [ ! -e '{}' ]; do sleep 0.02; done; cat",
        started.display(),
        release.display()
    ));

    let first = {
        let root = root.clone();
        std::thread::spawn(move || splice(&root, None, &pin_args(), &[], None))
    };
    wait_for(&started);
    let (answered, second) = std::sync::mpsc::channel();
    {
        let root = root.clone();
        std::thread::spawn(move || {
            let pin = SpliceArgs {
                pin: pin_args().pin,
                ..args("other.md")
            };
            let _ = answered.send(splice(&root, None, &pin, &[], None));
        });
    }
    let second = second.recv_timeout(Duration::from_secs(20));
    std::fs::write(&release, "").expect("release the filter");
    first
        .join()
        .expect("the first pin's thread ran")
        .expect("the first pin lands");

    let refused = second
        .expect("the second pin answers while the first is still in git")
        .expect_err("one git child per root");
    assert_eq!(refused.code, ErrorCode::WorkspaceBusy);
    assert_eq!(read(&root, "other.md"), OTHER);
}

/// A target that changed between git's answer and the flock refuses
/// `workspace_busy` with nothing written; the same request then lands, and its
/// hash is git's oid of the bytes the target carries. The filter itself is the
/// competing writer: on its first run it appends to the target it is hashing.
#[test]
fn a_target_changed_while_git_hashed_refuses_and_the_retry_lands() {
    let gate = tempfile::tempdir().expect("gate dir");
    let once = gate.path().join("once");
    let (_dir, root) = workspace(&format!(
        "cat; [ -e '{once}' ] || {{ touch '{once}'; printf '\\nlate line.\\n' >> guide.md; }}",
        once = once.display()
    ));

    let refused = splice(&root, None, &pin_args(), &[], None).expect_err("the target moved");
    assert_eq!(refused.code, ErrorCode::WorkspaceBusy);
    assert_eq!(read(&root, "plan.md"), PINNER);
    assert_eq!(read(&root, "guide.md"), format!("{TARGET}\nlate line.\n"));

    let ResponseBody::Splice { pin, .. } = splice(&root, None, &pin_args(), &[], None)
        .expect("the retry lands")
        .body
    else {
        panic!("splice body");
    };
    let landed = std::process::Command::new("git")
        .arg("-C")
        .arg(&root.0)
        .args(["hash-object", "--", "guide.md"])
        .output()
        .expect("git hash-object");
    assert_eq!(
        pin.expect("a pin fact").blob.as_deref(),
        Some(String::from_utf8_lossy(&landed.stdout).trim())
    );
}

/// On a git-annex root where `.git/info/attributes` sets `* filter=annex` and
/// `.gitattributes` exempts `*.md annex.largefiles=nothing`, a pin never
/// spawns `filter.annex` and still records git's exact blob oid.
#[test]
fn a_pin_skips_annex_filter_when_largefiles_is_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, body) in [
        ("plan.md", PINNER),
        ("guide.md", TARGET),
        (".gitattributes", "*.md annex.largefiles=nothing\n"),
    ] {
        std::fs::write(dir.path().join(name), body).expect("fixture page");
    }
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "pin@example.invalid"],
        vec!["config", "user.name", "pin"],
        vec!["config", "filter.annex.clean", "exit 42"],
        vec!["config", "filter.annex.process", "exit 42"],
        vec!["config", "filter.annex.required", "true"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    std::fs::write(dir.path().join(".git/info/attributes"), "* filter=annex\n")
        .expect("info/attributes");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let ResponseBody::Splice { pin, .. } = splice(&root, None, &pin_args(), &[], None)
        .expect("pin bypasses no-op filter.annex and lands")
        .body
    else {
        panic!("splice body");
    };
    let landed = std::process::Command::new("git")
        .arg("-C")
        .arg(&root.0)
        .args(["hash-object", "--no-filters", "--", "guide.md"])
        .output()
        .expect("git hash-object");
    assert_eq!(
        pin.expect("a pin fact").blob.as_deref(),
        Some(String::from_utf8_lossy(&landed.stdout).trim())
    );
}
