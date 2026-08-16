//! B-04 cutover record — durable-state and never-guess gates
//! (merkle-spec §4.2.6; price paper §11 gates 17/20 record half).

use fs::WorkspaceRoot;
use fs::cutover::{
    self, CutoverRecord, CutoverState, CutoverStatus, is_cutover_in_progress,
    is_cutover_state_error,
};

fn ws() -> (tempfile::TempDir, WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn building(domain: u32) -> CutoverRecord {
    CutoverRecord {
        state: CutoverState::NewBuilding,
        cutover_id: cutover::mint_cutover_id(),
        from: model::RootVersion::law1(domain),
        to: model::RootVersion::law2(domain),
        old_instance: Some("dead-beef.1.2".to_owned()),
        started_unix_ms: 1_755_300_000_000,
        committed_unix_ms: None,
    }
}

#[test]
fn absent_and_crash_before_rename_are_the_pre_cutover_world() {
    let (_d, root) = ws();
    assert_eq!(
        cutover::status(&root).expect("status"),
        CutoverStatus::Absent
    );

    // Crash BEFORE the rename: only the tmp file exists. The record is
    // Absent and the old law serves — the correct side of the boundary.
    std::fs::create_dir_all(root.0.join(".meridian")).expect("mkdir");
    std::fs::write(root.0.join(".meridian/cutover.tmp-cutover"), b"partial").expect("tmp");
    assert_eq!(
        cutover::status(&root).expect("status"),
        CutoverStatus::Absent
    );
    let law = cutover::serving(&root).expect("serving");
    assert_eq!(law.hash_law, model::HASH_LAW_FLAT);
    assert_eq!(law.law_boundary, 0);
}

#[test]
fn the_no_return_act_is_durable_and_round_trips() {
    let (_d, root) = ws();
    let rec = building(3);
    cutover::begin_new_building(&root, &rec).expect("begin");
    let read = match cutover::status(&root).expect("status") {
        CutoverStatus::Present(read) => read,
        CutoverStatus::Absent => panic!("record must exist past the boundary"),
    };
    assert_eq!(read, rec);

    // Serving mid-build is the typed in-progress refusal, never a guess and
    // never the old law.
    let err = cutover::serving(&root).expect_err("mid-build serving must refuse");
    assert!(is_cutover_in_progress(&err), "typed: {err}");

    // One cutover per workspace: a second boundary crossing refuses.
    let err = cutover::begin_new_building(&root, &building(3)).expect_err("double begin");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

#[test]
fn commit_advances_and_retirement_begins_exactly_there() {
    let (_d, root) = ws();

    // Committing with no boundary crossed refuses.
    let mut done = building(1);
    done.state = CutoverState::NewCommitted;
    done.committed_unix_ms = Some(1_755_300_100_000);
    let err = cutover::commit(&root, &done).expect_err("commit without begin");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

    cutover::begin_new_building(&root, &building(1)).expect("begin");
    cutover::commit(&root, &done).expect("commit");
    let read = match cutover::status(&root).expect("status") {
        CutoverStatus::Present(read) => read,
        CutoverStatus::Absent => panic!("record must exist"),
    };
    assert_eq!(read.state, CutoverState::NewCommitted);
    assert_eq!(read.committed_unix_ms, Some(1_755_300_100_000));

    // Re-commit refuses: the record only ever advances.
    let err = cutover::commit(&root, &done).expect_err("re-commit");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

    // Retirement begins at the boundary (spec §4.2.5): a held law-1 token
    // now classifies Retired; before the cutover it was unreachable.
    let law = cutover::serving(&root).expect("serving");
    assert_eq!(law.hash_law, model::HASH_LAW_RADIX);
    let serving = law.version(1);
    let old = model::RootVersion::law1(1).token([7u8; 32]);
    assert_eq!(
        serving.classify(&old.0),
        model::RootTokenVerdict::Retired,
        "a held old-law token refuses fingerprint_version_retired past the boundary"
    );
    let new = model::RootVersion::law2(1).token([7u8; 32]);
    assert_eq!(serving.classify(&new.0), model::RootTokenVerdict::Current);
}

#[test]
fn unrecognized_records_are_loud_never_guessed() {
    let arms: &[(&str, &[u8])] = &[
        ("empty", b""),
        ("no checksum", b"MRD-CUTOVER/1\nstate=new_building\n"),
        (
            "foreign checksum",
            b"MRD-CUTOVER/1\nstate=new_building\nchecksum=deadbeef\n",
        ),
        ("non-utf8", b"\xff\xfe\x00"),
    ];
    for (name, bytes) in arms {
        let (_d, root) = ws();
        std::fs::create_dir_all(root.0.join(".meridian")).expect("mkdir");
        std::fs::write(root.0.join(".meridian/cutover"), bytes).expect("write");
        let err = cutover::status(&root).expect_err(name);
        assert!(is_cutover_state_error(&err), "{name}: {err}");
        let err = cutover::serving(&root).expect_err(name);
        assert!(is_cutover_state_error(&err), "{name} serving: {err}");
    }
}

#[test]
fn truncation_and_unknown_words_are_loud() {
    // A genuine record truncated mid-file cannot decode: the checksum line
    // is gone or covers different bytes (the intent-convention proof).
    let (_d, root) = ws();
    cutover::begin_new_building(&root, &building(0)).expect("begin");
    let path = root.0.join(".meridian/cutover");
    let full = std::fs::read(&path).expect("read");
    for cut in [1, full.len() / 2, full.len() - 1] {
        std::fs::write(&path, &full[..cut]).expect("truncate");
        let err = cutover::status(&root).expect_err("truncated");
        assert!(is_cutover_state_error(&err), "cut={cut}: {err}");
    }

    // Unknown state word and unknown format line: loud, with the checksum
    // valid — the never-guess arm is semantic, not just integrity.
    for body in [
        "MRD-CUTOVER/1\nstate=old_serving\ncutover=x\nfrom=1 0\nto=2 0\nstarted_unix_ms=1\n",
        "MRD-CUTOVER/9\nstate=new_building\ncutover=x\nfrom=1 0\nto=2 0\nstarted_unix_ms=1\n",
    ] {
        let sum = blake3::hash(body.as_bytes()).to_hex();
        std::fs::write(&path, format!("{body}checksum={sum}\n")).expect("write");
        let err = cutover::status(&root).expect_err("semantic arm");
        assert!(is_cutover_state_error(&err), "{body:?}: {err}");
    }
}
