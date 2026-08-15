//! Card `run-churn-refusal-recovery-lines`: the corpus-scoped refusal a
//! CHURN member mints carries a fitted recovery line.
//!
//! Receipt (dogfood r9 prober § F3, receipt B): a run target refused with
//! `corpus root: the corpus cannot be served: <another seat's agent file>
//! cannot be read (No such file or directory (os error 2))`. The member was
//! not the caller's page and nothing in the call was wrong — an unrelated
//! `ccc-cli session join` MOVED that file while the corpus was being read.
//! The refusal said what failed and left the caller with no next move.
//!
//! WORDING ONLY. Re-deriving the corpus instead of refusing is the churn-grain
//! design (card `guard-grain-batch-doors`), not this card.
//!
//! The class is `retry`, not the `refresh` the prober's proposal sketched: §8
//! binds a vanished corpus member to `corpus_race`/`retry` (wire `ErrorCode`
//! docs — "the same request re-derives from the current world"), and the
//! registry seam already maps it that way
//! (`member_vanished_mid_snapshot_teaches_retry_not_env`). A recovery line
//! naming a different class than the frame it rides would teach two answers.

use std::io;

/// The churn arm — the member left the corpus mid-read. Reason first, then
/// one fitted line: nothing the caller named is wrong, so the move is to
/// re-issue the call.
#[test]
fn a_vanished_member_teaches_re_issue_with_recovery_retry() {
    let said = fs::CorpusMemberError {
        kind: io::ErrorKind::NotFound,
        member: "year=2026/month=08/15-00-adhoc/agents/b87d6be2.md".to_owned(),
        condition: "cannot be read (No such file or directory (os error 2))".to_owned(),
    }
    .to_string();

    let (reason, recovery) = said
        .split_once('\n')
        .unwrap_or_else(|| panic!("the refusal carries a recovery line: {said:?}"));
    assert!(
        reason.starts_with("the corpus cannot be served: ")
            && reason.contains("agents/b87d6be2.md"),
        "reason first, naming the member (Law A-3c): {reason:?}"
    );
    assert_eq!(
        recovery,
        "  → the member left the corpus while it was being read, and nothing you \
         named is wrong — re-issue the call (recovery: retry)",
        "the fitted line, in the face's recovery-hint grammar"
    );
}

/// The same churn shape the stat sweep mints for a member the walk listed and
/// the stat missed — same class, same line.
#[test]
fn a_member_vanished_between_walk_and_stat_teaches_the_same_line() {
    let said = fs::CorpusMemberError {
        kind: io::ErrorKind::NotFound,
        member: "notes/x.md".to_owned(),
        condition: "vanished between the domain walk and its stat".to_owned(),
    }
    .to_string();
    assert!(
        said.contains("(recovery: retry)"),
        "the stat-sweep churn arm teaches the same recovery: {said:?}"
    );
}

/// Control — an unreadable member is an environment fault. Re-issuing never
/// fixes permissions, so no line promises it will.
#[test]
fn an_unreadable_member_teaches_no_recovery_line() {
    let said = fs::CorpusMemberError {
        kind: io::ErrorKind::PermissionDenied,
        member: "notes/x.md".to_owned(),
        condition: "cannot be read (permission denied)".to_owned(),
    }
    .to_string();
    assert_eq!(
        said, "the corpus cannot be served: notes/x.md cannot be read (permission denied)",
        "a permission fault teaches nothing about re-issuing: {said:?}"
    );
}

/// Control — a poison member persists. The recovery is fixing the file, and
/// this face is not where that is taught.
#[test]
fn a_poison_member_teaches_no_recovery_line() {
    let said = fs::CorpusMemberError {
        kind: io::ErrorKind::InvalidData,
        member: "notes/poison.md".to_owned(),
        condition: "is not UTF-8 (invalid byte at offset 3)".to_owned(),
    }
    .to_string();
    assert!(
        !said.contains("recovery:"),
        "a persistent condition carries no re-issue line: {said:?}"
    );
}
