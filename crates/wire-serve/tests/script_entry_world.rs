//! § A.7 design gates — the in-process script lane's entry-world semantics,
//! held at the executor's own seam (`wire_serve::script::exec::serve_script`)
//! where the timing is DETERMINISTIC: the world is pinned first, the foreign
//! edit lands on disk second, the attempt runs third. "Mid-program" is
//! exactly "after pin, before commit", and this harness controls both edges.
//!
//! The five ruled properties, each with its own gate:
//! 1. entry snapshot — a foreign edit after the pin is INVISIBLE to reads;
//! 2. commit CAS unchanged — the same foreign edit REFUSES the commit
//!    (`fingerprint_mismatch`), nothing lands;
//! 3. armed overlay — a read after the program's own `put()` serves the
//!    ARMED content (read-your-own-writes), with honest armed revs;
//! 4. entry-grain tokens — an overlay read never threads an armed rev, so
//!    read→put→read→put commits (the pre-batch state is the entry world);
//! 5. containment — fuel exhaustion, a panicking world, and an elapsed wall
//!    clock each land as a TYPED outcome; nothing unwinds out of the serve.

use std::collections::BTreeMap;

use wire_serve::script::exec::{EntryWorld, ScriptExec, serve_script};

/// The pinned entry world of a real on-disk workspace, snapshotted at pin
/// time — mutating the directory afterwards models the foreign writer.
struct PinnedFixture {
    docs: BTreeMap<String, model::Document>,
    unserved: BTreeMap<String, String>,
    fingerprint: wire::Root,
}

impl PinnedFixture {
    fn pin(root: &fs::WorkspaceRoot) -> Self {
        let (files, fingerprint) = fs::domain_snapshot(root).expect("snapshot");
        let (_index, docs, unserved) = fs::build_corpus(files);
        Self {
            docs,
            unserved,
            fingerprint: wire::Root(fingerprint.0),
        }
    }
}

impl EntryWorld for PinnedFixture {
    fn doc(&self, path: &str) -> Option<&model::Document> {
        self.docs.get(path)
    }

    fn unserved(&self, path: &str) -> Option<&str> {
        self.unserved.get(path).map(String::as_str)
    }

    fn fingerprint(&self) -> wire::Root {
        self.fingerprint.clone()
    }
}

/// A world whose every read panics — the eval-boundary containment probe.
struct PanickingWorld;

impl EntryWorld for PanickingWorld {
    fn doc(&self, _path: &str) -> Option<&model::Document> {
        panic!("the world panicked mid-read");
    }

    fn unserved(&self, _path: &str) -> Option<&str> {
        None
    }

    fn fingerprint(&self) -> wire::Root {
        wire::Root("b3:panic-world".to_owned())
    }
}

const NOTE: &str = "---\nstatus: open\n---\n# Alpha\n\nalpha body\n\n## Beta\n\nbeta body\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("note.md"), NOTE).expect("note");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn exec(source: &str) -> ScriptExec {
    ScriptExec {
        id: Some(7),
        source: source.to_owned(),
        args: BTreeMap::new(),
        files: vec!["note.md".to_owned()],
        actor: Some("agent:test".to_owned()),
        now: Some("2026-08-12T00:00:00Z".to_owned()),
        receipt: None,
        if_root: None,
        dry: false,
        expect_armed: None,
        budgets: None,
    }
}

fn commit_leg(trace: &wire::ScriptTrace) -> serde_json::Value {
    serde_json::from_str(trace.commit.as_ref().expect("commit leg").get()).expect("leg parses")
}

/// Gate 1 — a foreign disk edit landing AFTER the pin is invisible to the
/// program's reads: the recorded face carries the ENTRY bytes.
#[test]
fn a_foreign_edit_after_the_pin_is_invisible_to_reads() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);
    // The foreign writer moves the world after the pin.
    std::fs::write(root.0.join("note.md"), "# Alpha\n\nFOREIGN\n").expect("foreign edit");

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec("t = read(\"note.md\", section=\"Alpha/Beta\")\n"),
    );
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::NoEffect);
    let [wire::TraceEntry::Echo(read)] = out.trace.trace.as_slice() else {
        panic!("expected one echo read, got {:?}", out.trace.trace);
    };
    let wire::ReadFace::Section(sec) = &read.face else {
        panic!("expected the cat face");
    };
    assert!(
        sec.text.contains("beta body"),
        "the read must serve the ENTRY world, not the foreign edit: {}",
        sec.text
    );
}

/// Gate 2 — the same foreign edit REFUSES the commit: `if_fingerprint` = the entry
/// fingerprint, checked first, byte-derived under the flock — outcome
/// `conflict`, and the script's write never lands.
#[test]
fn a_foreign_edit_refuses_the_commit_and_nothing_lands() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);
    std::fs::write(root.0.join("note.md"), "# Alpha\n\nFOREIGN\n").expect("foreign edit");

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec("t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n"),
    );
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Conflict);
    assert!(out.committed.is_none(), "a conflict advances no ring");
    let leg = commit_leg(&out.trace);
    assert_eq!(
        leg.get("code").and_then(serde_json::Value::as_str),
        Some("fingerprint_mismatch"),
        "the embedded leg is the engine's own §5.1 refusal in v3 vocabulary: {leg}"
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert_eq!(disk, "# Alpha\n\nFOREIGN\n", "nothing of the script landed");
}

/// Gate 3 — read-your-own-writes: a read after the program's own `put()` serves the
/// ARMED content with an honest armed rev, and the commit lands those bytes.
#[test]
fn a_read_after_the_programs_own_put_serves_the_armed_content() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec(
            "before = read(\"note.md\")\n\
             put(\"note.md\", props={\"status\": \"done\"})\n\
             after = read(\"note.md\")\n",
        ),
    );
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Committed);
    let reads: Vec<&wire::ReadEntry> = out
        .trace
        .trace
        .iter()
        .filter_map(|e| match e {
            wire::TraceEntry::Echo(r) | wire::TraceEntry::Read(r) => Some(r),
            wire::TraceEntry::Armed(_) => None,
        })
        .collect();
    let [before, after] = reads.as_slice() else {
        panic!("expected two reads");
    };
    let (wire::ReadFace::Toc(before), wire::ReadFace::Toc(after)) = (&before.face, &after.face)
    else {
        panic!("expected two toc faces");
    };
    assert_eq!(before.fm.get("status").map(String::as_str), Some("open"));
    assert_eq!(
        after.fm.get("status").map(String::as_str),
        Some("done"),
        "the overlay must serve the program its own armed write"
    );
    assert_ne!(
        before.rev, after.rev,
        "the overlay face is honest: armed bytes carry armed revs"
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert!(
        disk.contains("status: done"),
        "the commit landed the armed value: {disk}"
    );
}

/// Gate 4 — the token plane is entry-grain: an overlay read between two puts must
/// not thread its armed rev — the batch resolves against the pre-batch state
/// (= the entry world), so the commit LANDS. A threading that took the
/// overlay rev would refuse `cas_mismatch` here.
#[test]
fn an_overlay_read_never_threads_an_armed_rev() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec(
            "t = read(\"note.md\")\n\
             put(\"note.md\", props={\"status\": \"done\"})\n\
             t2 = read(\"note.md\")\n\
             put(\"note.md\", props={\"owner\": \"arm-a\"})\n",
        ),
    );
    assert_eq!(
        out.trace.outcome,
        wire::ScriptOutcome::Committed,
        "entry-grain tokens commit; an overlay-rev token would cas_mismatch: {:?}",
        out.trace.fault
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert!(
        disk.contains("status: done") && disk.contains("owner: arm-a"),
        "{disk}"
    );
}

/// The write-follows-read law binds unchanged at the wire door: a row whose
/// target the attempt never read threads no token and the engine refuses the
/// batch (`guard_required`) — the executor mints nothing to satisfy the
/// engine's own guard.
#[test]
fn an_unread_target_meets_the_engines_own_guard_refusal() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec("put(\"note.md\", props={\"status\": \"done\"})\n"),
    );
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Refused);
    let fault = out.trace.fault.expect("refusal fault");
    assert_eq!(fault.code.as_deref(), Some("guard_required"));
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert_eq!(disk, NOTE, "nothing landed");
}

/// An append threads its section's ENTRY node rev off the toc row the read
/// published — the read-then-write CAS closing in-process.
#[test]
fn an_append_threads_the_entry_section_rev_and_commits() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec(
            "t = read(\"note.md\")\n\
             put(\"note.md\", section=\"Alpha/Beta\", append=\"- appended\\n\")\n",
        ),
    );
    assert_eq!(
        out.trace.outcome,
        wire::ScriptOutcome::Committed,
        "{:?}",
        out.trace.fault
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert!(disk.contains("- appended"), "{disk}");
}

/// Gate 5a — fuel containment: a hostile loop lands as a TYPED budget fault with
/// telemetry, never a hang and never an unwind.
#[test]
fn fuel_exhaustion_is_a_typed_budget_fault() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let mut e = exec("x = 0\nfor i in range(100000000):\n    x = x + 1\n");
    e.budgets = Some(wire::ScriptBudgets {
        fuel: Some(10_000),
        ..wire::ScriptBudgets::default()
    });
    let out = serve_script(&root, None, None, &world, &e);
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Fault);
    let fault = out.trace.fault.expect("budget fault");
    assert_eq!(fault.class, wire::FaultClass::Budget);
    assert!(
        out.trace.telemetry.wall_ms < 60_000,
        "terminated, never hung"
    );
}

/// Gate 5b — a panicking world is CONTAINED at the eval boundary: the serve returns
/// a typed outcome and the caller (the daemon's connection thread) survives.
/// The kernel's panic arm classifies it as a budget fault today — the
/// mislabel is a named residual (design notes §10), the containment is the
/// gate.
#[test]
fn a_panicking_world_is_contained_at_the_eval_boundary() {
    let (_dir, root) = workspace();
    let out = serve_script(
        &root,
        None,
        None,
        &PanickingWorld,
        &exec("t = read(\"note.md\")\n"),
    );
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Fault);
    assert!(
        out.trace.fault.is_some(),
        "a contained panic speaks, never unwinds"
    );
}

/// Gate 5c — the wall clock binds at the first host read: an elapsed budget refuses
/// the read in the entry's own vocabulary and the attempt aborts whole.
#[test]
fn an_elapsed_wall_clock_refuses_at_the_first_read() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let mut e = exec("t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"x\"})\n");
    e.budgets = Some(wire::ScriptBudgets {
        wall_ms: Some(0),
        ..wire::ScriptBudgets::default()
    });
    let out = serve_script(&root, None, None, &world, &e);
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Fault);
    let fault = out.trace.fault.expect("wall fault");
    assert!(
        fault.reason.contains("wall clock elapsed"),
        "the refusal names the clock: {}",
        fault.reason
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert_eq!(disk, NOTE, "nothing landed");
}

/// `dry` is the ARM rehearsal: the splice runs everything except disk, the
/// leg carries the engine's own `dry: true` and `fingerprint_after: null`,
/// and the outcome is `no_effect` with the workspace untouched.
#[test]
fn a_dry_run_rehearses_and_lands_nothing() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let mut e = exec("t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n");
    e.dry = true;
    let out = serve_script(&root, None, None, &world, &e);
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::NoEffect);
    assert!(out.committed.is_none());
    let leg = commit_leg(&out.trace);
    assert_eq!(leg.get("dry"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        leg.get("fingerprint_after"),
        Some(&serde_json::Value::Null),
        "v3 vocabulary on the embedded leg: {leg}"
    );
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert_eq!(disk, NOTE);
}

/// `expect_armed` refuses BEFORE the splice is issued when the digest
/// disagrees — nothing sent, nothing landed, recovery `fix`.
#[test]
fn an_expect_armed_mismatch_refuses_pre_splice() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let mut e = exec("t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n");
    e.expect_armed = Some("armed-set-path-edit:sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned());
    let out = serve_script(&root, None, None, &world, &e);
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Refused);
    let fault = out.trace.fault.expect("refusal");
    assert!(
        fault.reason.contains("expect_armed_mismatch"),
        "{}",
        fault.reason
    );
    assert!(out.trace.commit.is_none(), "no splice was issued");
    let disk = std::fs::read_to_string(root.0.join("note.md")).expect("read disk");
    assert_eq!(disk, NOTE);
}

/// The caller's own `if_fingerprint` guard refuses pre-eval: zero reads,
/// nothing armed, `guard_expected` carries the pinned value.
#[test]
fn a_caller_guard_mismatch_refuses_pre_eval() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let mut e = exec("t = read(\"note.md\")\n");
    e.if_root = Some(wire::Root("b3:not-the-entry".to_owned()));
    let out = serve_script(&root, None, None, &world, &e);
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::Conflict);
    assert!(out.trace.trace.is_empty(), "zero evaluation");
    assert_eq!(
        out.trace.guard_expected.as_deref(),
        Some("b3:not-the-entry")
    );
    assert_eq!(out.trace.telemetry.reads_used, 0);
}

/// A read-only program is the read-class exit: `no_effect`, no commit leg,
/// no armed digest — and the trace carries the § A.6-decoded frontmatter.
#[test]
fn a_read_only_program_exits_read_class() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(&root, None, None, &world, &exec("t = read(\"note.md\")\n"));
    assert_eq!(out.trace.outcome, wire::ScriptOutcome::NoEffect);
    assert!(out.trace.commit.is_none());
    assert!(out.trace.armed_digest.is_none());
    let [wire::TraceEntry::Echo(read)] = out.trace.trace.as_slice() else {
        panic!("one echo read");
    };
    let wire::ReadFace::Toc(toc) = &read.face else {
        panic!("toc face");
    };
    assert_eq!(toc.fm.get("status").map(String::as_str), Some("open"));
    assert!(toc.words > 0, "words_total is a delivered fact");
}

/// The armed digest rides the trace with its domain tag — the courier
/// property's engine half.
#[test]
fn the_armed_digest_carries_the_domain_tag() {
    let (_dir, root) = workspace();
    let world = PinnedFixture::pin(&root);

    let out = serve_script(
        &root,
        None,
        None,
        &world,
        &exec("t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n"),
    );
    let digest = out.trace.armed_digest.expect("armed digest");
    assert!(
        digest.starts_with("armed-set-path-edit:sha256:"),
        "the digest names its domain: {digest}"
    );
}
