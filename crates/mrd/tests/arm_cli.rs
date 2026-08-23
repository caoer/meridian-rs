//! `mrd arm` end-to-end gates — the ARM act as a verb (armed-plane rung 4),
//! driven through the real binary over its process boundary.
//!
//! The subject is the ATTEST PATH the binding law's refusals name: a
//! tool-written road from "I have a rule page" to "an armed row governs writes
//! at this root", with no hand-edited `meridian/` file anywhere on it. The
//! after-receipt test drives the whole trip: author → read the rev where a
//! reviewer would (`mrd rules --json`) → arm → a violating put refuses naming
//! the rule.

use std::path::PathBuf;
use std::process::Output;

mod common;

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A check rule page with a real predicate: a close must carry a verdict — the
/// U4.4 `close-verdict` floor, minimally. `paths: tasks/**` scopes it, so the
/// violating write must live under `tasks/`.
const CLOSE_VERDICT: &str = r#"---
tags: [type/rule, rules/check]
id: close-verdict
paths:
  - tasks/**
---

# close-verdict

A close carries a Verdict; a bare status flip is refused.

```starlark
def check_change(change):
    if "status" not in change.fields_changed:
        return
    if change.doc.frontmatter.get("status") != "closed":
        return
    verdict = change.doc.frontmatter.get("verdict")
    if verdict == None or verdict == "":
        refuse(
            message = "close_verdict: a close must carry a Verdict — a bare status flip is refused",
            passing = "close-with-verdict",
        )
```
"#;

/// A hook rule page — the other kind, for the mode-vocabulary gate.
const NOTIFY_HOOK: &str = "---\ntags: [type/rule, rules/hook]\nid: task.notify\n---\n\n# notify\n";

/// A governed task card in `close-verdict`'s declared scope.
const CARD: &str = "---\ntype: task\nstatus: todo\n---\n\n# a card\n";

/// The two PRODUCTION shapes, as files — the arm gate's wiring is only proved
/// by shapes the live corpus actually arms.
///
/// They are FIXTURES on purpose. The sessions root's own
/// `rules/010-middleware-spawned-by.md` is being rewritten under card
/// `rules-010-middleware-wrong-entry-point`; a test that read the live page
/// would flip with that rewrite and stop testing the wiring it exists to test.
mod shapes {
    /// `rules/middleware` tag, `check_change` body — registers, cannot load.
    pub(super) const MIDDLEWARE_WRONG_ENTRY: &str =
        include_str!("fixtures/middleware-wrong-entry.md");
    /// Its twin, `def middleware` — the control that must arm.
    pub(super) const MIDDLEWARE_RIGHT_ENTRY: &str =
        include_str!("fixtures/middleware-right-entry.md");
    /// A `caps: []` hook declaring `budget:`/`how:` — the live hook shape.
    pub(super) const HOOK_CAPS_EMPTY: &str = include_str!("fixtures/hook-caps-empty.md");

    pub(super) const MIDDLEWARE_WRONG_ENTRY_ID: &str = "010-fixture-middleware-wrong-entry";
    pub(super) const MIDDLEWARE_RIGHT_ENTRY_ID: &str = "010-fixture-middleware-right-entry";
    pub(super) const HOOK_CAPS_EMPTY_ID: &str = "050-fixture-hook-caps-empty";
}

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.ws.join(rel)).expect("readable")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_stdin(args, "")
    }

    fn run_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }

    /// stdout of a run asserted to exit 0.
    fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "mrd {args:?} exited {:?}\n{}{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// The rev a REVIEWER reads: the winner line of `mrd rules --json` for `id`
    /// at the workspace root — the same surface a human attests from, so the
    /// trip never computes a rev the operator could not have seen. Tolerates a
    /// findings exit (1): a drifted row or an unreadable armed set still prints
    /// the registration view the reviewer reads the rev from.
    fn reviewed_rev(&self, id: &str) -> String {
        let out = self.run(&["rules", ".", "--json"]);
        assert_ne!(
            out.status.code(),
            Some(2),
            "rules refused the invocation: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let raw = String::from_utf8(out.stdout).expect("utf-8 stdout");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("rules --json parses");
        let rows = json["rules"]["rules"].as_array().expect("rows");
        let row = rows
            .iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not in the effective set: {raw}"));
        row["chain"][0]["rev"]
            .as_str()
            .expect("the winner's rev")
            .to_owned()
    }

    /// The key-grain token the wire door compares for `fm_key: status`.
    /// `--force` would also satisfy fingerprint-or-force, but it escapes the
    /// armed block this trip exists to prove.
    fn status_prop_rev(&self) -> String {
        let raw = self.stdout(&["read", "tasks/card.md", "--json"]);
        let json: serde_json::Value = serde_json::from_str(&raw).expect("read --json");
        json["read"]["props"]
            .as_array()
            .expect("props plane")
            .iter()
            .find(|p| p["key"] == "status")
            .and_then(|p| p["prop_rev"].as_str())
            .expect("status prop_rev")
            .to_owned()
    }
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("ws");
    for dir in [&home, &cache_home, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    let sandbox = Sandbox {
        tmp,
        home,
        cache_home,
        ws,
    };
    sandbox.write(
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n",
    );
    std::fs::create_dir_all(sandbox.home.join(".config")).ok();
    std::fs::write(
        sandbox.home.join("MERIDIAN.md"),
        "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n",
    )
    .expect("home anchor");
    sandbox
}

/// A sandbox with the check rule and a governed card, nothing armed yet.
fn authored() -> Sandbox {
    let s = sandbox();
    s.write("rules/close-verdict.md", CLOSE_VERDICT);
    s.write("tasks/card.md", CARD);
    s
}

// ── the after-receipt: the whole trip, no hand-edited rows ────────────────────

/// **The card's after-receipt.** Fresh workspace → author the rule → read the
/// rev where a reviewer would → `mrd arm` → a violating put REFUSES naming the
/// rule → the legal write lands. Every `meridian/` byte on the trip is
/// tool-written.
#[test]
fn the_arm_round_trip_ends_with_the_rule_refusing_a_violating_write() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");

    let armed = s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);
    assert!(
        armed.contains("close-verdict") && armed.contains("block"),
        "the act names what it armed: {armed}"
    );

    // Both halves of the disk edge exist, tool-written.
    assert!(
        s.ws.join(policy::armed::ARMED_RULES_PATH).is_file(),
        "the artifact landed"
    );
    assert!(
        s.ws.join(policy::ATTESTED_MARKER_PATH).is_file(),
        "the once-armed marker landed"
    );

    // The violating write: a bare status flip to closed, no verdict.
    // Guarded with the key-grain token so the wire door admits the edit
    // and the armed block is what refuses — `--force` would skip that block.
    let status_rev = s.status_prop_rev();
    let violating = format!(
        r#"[{{"target":{{"fm_key":"status"}},"edit":{{"match":{{"old":"todo","new":"closed"}}}},"if_node_rev":"{status_rev}"}}]"#
    );
    let out = s.run_stdin(&["put", "tasks/card.md"], &violating);
    let refusal = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "the armed law refuses the violating write: {refusal}"
    );
    assert!(
        refusal.contains("close_verdict"),
        "the refusal names the rule: {refusal}"
    );
    let card = s.read("tasks/card.md");
    assert!(
        card.contains("status: todo"),
        "the refused bytes never landed: {card}"
    );

    // The legal path: the same close carrying its verdict — `at: upsert`
    // births the key, exactly as the refusal above teaches. Birth is
    // absence-guarded; the status match still carries the key-grain token.
    let legal = format!(
        r#"[{{"target":{{"fm_key":"status"}},"edit":{{"match":{{"old":"todo","new":"closed"}}}},"if_node_rev":"{status_rev}"}},{{"target":{{"fm_key":"verdict"}},"edit":{{"put":{{"at":"upsert","text":"approve"}}}}}}]"#
    );
    let out = s.run_stdin(&["put", "tasks/card.md"], &legal);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the close WITH a verdict lands: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── the disk edge ─────────────────────────────────────────────────────────────

/// Genesis: the first arm writes the artifact (strictly parseable) and the
/// once-armed marker — and nothing else: the journal era is retired (its
/// producer side is deleted), so the act's permanence IS the attested row and
/// the marker, and no `meridian/journal.md` may reappear through this door.
#[test]
fn the_first_arm_writes_artifact_and_marker_and_nothing_else() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");
    s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);

    let artifact = s.read(policy::armed::ARMED_RULES_PATH);
    let parsed = policy::armed::parse_artifact(&artifact).expect("the artifact parses strictly");
    assert_eq!(parsed.rows().len(), 1);
    let row = &parsed.rows()[0];
    assert_eq!(row.id().as_str(), "close-verdict");
    assert_eq!(row.rev(), rev, "the row pins the reviewed rev");
    assert_eq!(row.scope().as_str(), "", "armed at the workspace root");

    assert!(
        s.ws.join(policy::ATTESTED_MARKER_PATH).is_file(),
        "the marker exists"
    );
    assert!(
        !s.ws.join("meridian/journal.md").exists(),
        "the retired journal did not come back through the attest path"
    );
}

/// Re-running the exact same arm is a no-op success — the crash-recovery
/// re-run costs nothing and changes nothing.
#[test]
fn re_running_the_same_arm_is_a_no_op() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");
    s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);
    let before = s.read(policy::armed::ARMED_RULES_PATH);

    let again = s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);
    assert!(
        again.contains("unchanged"),
        "the no-op says it changed nothing: {again}"
    );
    assert_eq!(
        s.read(policy::armed::ARMED_RULES_PATH),
        before,
        "the artifact is byte-identical"
    );
}

/// A stale `--rev` is the drift refusal: the reviewer's approval no longer
/// covers the live law, and the refusal names both revs.
#[test]
fn a_stale_rev_is_refused_naming_both_revs() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");
    let stale = "0123456789abcdef";
    assert_ne!(rev, stale);
    let out = s.run(&["arm", "close-verdict", "--mode", "block", "--rev", stale]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "drift refuses: {text}");
    assert!(
        text.contains(stale) && text.contains(&rev),
        "the refusal names the approved rev and the live one: {text}"
    );
    assert!(
        !s.ws.join(policy::armed::ARMED_RULES_PATH).exists(),
        "nothing landed"
    );
}

/// The re-arm: after the pinned page is edited the old row reddens; arming at
/// the live rev REPLACES the (id, root) row — one row, firing again.
#[test]
fn an_edited_page_re_arms_at_the_live_rev_replacing_the_row() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");
    s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);

    // The law is amended after arming.
    s.write(
        "rules/close-verdict.md",
        &format!("{CLOSE_VERDICT}\n<!-- amended -->\n"),
    );
    let red = s.run(&["rules", "tasks"]);
    assert_eq!(
        red.status.code(),
        Some(1),
        "the drifted row is a finding before the re-arm"
    );

    let live = s.reviewed_rev("close-verdict");
    assert_ne!(live, rev, "the edit moved the rev");
    let rearmed = s.stdout(&["arm", "close-verdict", "--mode", "block", "--rev", &live]);
    assert!(
        rearmed.contains(&live),
        "the act names the new pin: {rearmed}"
    );

    let parsed = policy::armed::parse_artifact(&s.read(policy::armed::ARMED_RULES_PATH))
        .expect("still strictly parseable");
    assert_eq!(parsed.rows().len(), 1, "replaced, not accumulated");
    assert_eq!(parsed.rows()[0].rev(), live);
    s.stdout(&["rules", "tasks"]); // exit 0 — the re-armed set is clean
}

// ── the arm-plane refusals reach the operator ─────────────────────────────────

/// The [[armed-scope-vocabulary]] interplay: the resolver's `layer:depth`
/// spelling is refused AT THE VERB with the fitted teaching — the sealed
/// `ArmRoot::parse` guard reached this surface with zero new code.
#[test]
fn the_resolver_scope_spelling_is_refused_at_the_verb() {
    let s = authored();
    let rev = s.reviewed_rev("close-verdict");
    let out = s.run(&[
        "arm",
        "close-verdict",
        "--mode",
        "block",
        "--rev",
        &rev,
        "--at",
        "workspace:0",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "not a legal arm root: {text}");
    assert!(
        text.contains("layer:depth") && text.contains("ARM ROOT"),
        "the fitted teaching names both vocabularies: {text}"
    );
}

/// An absolute `--at` is a bad invocation (exit 2) whose refusal teaches the
/// law instead of leaving it to trial: it names the workspace-relative
/// constraint, quotes what was passed, and shows the `.` default. Acceptance
/// is untouched — absolute paths stay refused.
#[test]
fn an_absolute_at_is_a_bad_invocation_teaching_the_constraint() {
    let s = authored();
    let out = s.run(&[
        "arm",
        "close-verdict",
        "--mode",
        "block",
        "--rev",
        "deadbeefdeadbeef",
        "--at",
        "/tmp",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "a bad invocation: {text}");
    assert!(
        text.contains("workspace-relative") && text.contains("/tmp") && text.contains("`.`"),
        "the refusal names the constraint, what was passed, and the default: {text}"
    );
}

/// A corrupt EXISTING artifact refuses the arm and is left untouched —
/// overwriting an unreadable artifact would silently disarm unknown rows.
#[test]
fn a_corrupt_existing_artifact_refuses_the_arm_untouched() {
    let s = authored();
    std::fs::create_dir_all(s.ws.join("meridian")).unwrap();
    let garbage = "# Not the armed set\n\nnothing here is a § 4 row.\n";
    s.write(policy::armed::ARMED_RULES_PATH, garbage);
    s.write(policy::ATTESTED_MARKER_PATH, "");

    let rev = s.reviewed_rev("close-verdict");
    let out = s.run(&["arm", "close-verdict", "--mode", "block", "--rev", &rev]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "corrupt refuses: {text}");
    assert!(
        text.contains("corrupt"),
        "the refusal says what is wrong: {text}"
    );
    assert_eq!(
        s.read(policy::armed::ARMED_RULES_PATH),
        garbage,
        "the unreadable artifact was not clobbered"
    );
}

/// An id that resolves to nothing at the root is `Unresolved` — through the
/// process boundary, naming the root it was narrowed to.
#[test]
fn arming_an_unregistered_id_is_refused_naming_the_root() {
    let s = authored();
    let out = s.run(&[
        "arm",
        "no.such.rule",
        "--mode",
        "block",
        "--rev",
        "0123456789abcdef",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("no.such.rule") && text.contains("resolves to nothing"),
        "the refusal names the id and the fact: {text}"
    );
}

/// An id whose ONLY carrier the hash domain excludes refuses with the fitted
/// teaching: the file, and why it is out of domain — for both §12.1 classes.
/// Silent non-registration must not read as a working door
/// (card rules-silent-nonregistration).
#[test]
fn arming_a_domain_excluded_candidate_names_the_file_and_the_reason() {
    let s = sandbox();

    // Class 1: the dot-segment structural floor.
    s.write(
        ".hidden/rules/x.md",
        "---\ntags: [type/rule, rules/check]\nid: hidden.law\npaths:\n  - tasks/**\n---\n\n# hidden\n",
    );
    let out = s.run(&[
        "arm",
        "hidden.law",
        "--mode",
        "block",
        "--rev",
        "0123456789abcdef",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("hidden.law")
            && text.contains(".hidden/rules/x.md")
            && text.contains("dot-prefixed path segment"),
        "the refusal names the id, the file, and the exclusion class: {text}"
    );

    // Class 2: an operator-declared `meridian/domain.md` ignore rule.
    s.write(
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"parked/**\"\n---\n\n# domain\n",
    );
    s.write(
        "parked/rules/y.md",
        "---\ntags: [type/rule, rules/check]\nid: parked.law\npaths:\n  - tasks/**\n---\n\n# parked\n",
    );
    let out = s.run(&[
        "arm",
        "parked.law",
        "--mode",
        "block",
        "--rev",
        "0123456789abcdef",
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("parked.law")
            && text.contains("parked/rules/y.md")
            && text.contains("ignore rule"),
        "the refusal names the id, the file, and the ignore-rule class: {text}"
    );
}

/// A hook asked to `block` is the `ModeKind` refusal, teaching the vocabulary —
/// the § 4 split, reaching the operator at the verb.
#[test]
fn a_hook_armed_in_check_vocabulary_is_refused_with_the_teaching() {
    let s = sandbox();
    s.write("notify.md", NOTIFY_HOOK);
    let rev = s.reviewed_rev("task.notify");
    let out = s.run(&["arm", "task.notify", "--mode", "block", "--rev", &rev]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("off|armed") && text.contains("never veto"),
        "the refusal teaches the hook vocabulary and why: {text}"
    );
}

/// `--rev` is REQUIRED: the rev IS the attestation. Its absence is a bad
/// invocation whose message teaches where a reviewer reads the rev.
#[test]
fn a_missing_rev_is_a_bad_invocation_that_teaches_where_to_read_it() {
    let s = authored();
    let out = s.run(&["arm", "close-verdict", "--mode", "block"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "{text}");
    assert!(
        text.contains("--rev") && text.contains("mrd rules"),
        "the refusal teaches the attestation and where to read the rev: {text}"
    );
}

// ── the LOAD gate's production wiring ─────────────────────────────────────────
//
// `policy::armed::arm` takes its page bytes from an injected `PageSource`, and
// `arm_cmd.rs` is the ONE place that hands it the production one —
// `wire_serve::armed_disk::DiskPages`, the same source the fire path reads.
// The gate's law has six unit tests over an in-memory source; none of them
// touches that argument. Swap it for an empty source and every one stays green
// while the shipped `mrd` arms law that can never fire.
//
// These four drive the gate through the real binary on the two shapes the live
// corpus actually arms, in both directions — so the argument is asserted, not
// assumed.

/// The refusal. A `rules/middleware` page whose fenced block defines
/// `check_change` instead of `middleware` REGISTERS and cannot LOAD; `block`
/// fires, so the winner is loaded, and the act refuses with the loader's own
/// `EntryMissing` words. Nothing lands.
#[test]
fn a_middleware_page_with_the_wrong_entry_point_refuses_the_arm() {
    let s = sandbox();
    s.write("rules/wrong.md", shapes::MIDDLEWARE_WRONG_ENTRY);
    let rev = s.reviewed_rev(shapes::MIDDLEWARE_WRONG_ENTRY_ID);

    let out = s.run(&[
        "arm",
        shapes::MIDDLEWARE_WRONG_ENTRY_ID,
        "--mode",
        "block",
        "--rev",
        &rev,
    ]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a winner that does not load refuses the act: {text}"
    );
    assert!(
        text.contains(shapes::MIDDLEWARE_WRONG_ENTRY_ID) && text.contains("rules/wrong.md"),
        "the refusal names the id and the page: {text}"
    );
    assert!(
        text.contains("rules/middleware") && text.contains("def middleware"),
        "the loader's own EntryMissing teaching survives the process boundary — \
         the leg and the entry point it is evaluated through: {text}"
    );
    assert!(
        text.contains("can never fire"),
        "and the act teaches why bytes alone are not an attestation: {text}"
    );
    assert!(
        !s.ws.join(policy::armed::ARMED_RULES_PATH).exists(),
        "the artifact is byte-untouched — here, never created"
    );
    assert!(
        !s.ws.join(policy::ATTESTED_MARKER_PATH).exists(),
        "and the workspace was not marked armed"
    );
}

/// The same bytes at the same rev arm `off`: the gate runs on the modes that
/// FIRE, and an attested-off row is precisely the reviewer's record that they
/// read a page too broken to load and did not activate it. Same page, opposite
/// outcome — so the refusal above is the LOAD gate, not this shape being
/// unarmable through the binary.
#[test]
fn the_same_unloadable_page_still_arms_off() {
    let s = sandbox();
    s.write("rules/wrong.md", shapes::MIDDLEWARE_WRONG_ENTRY);
    let rev = s.reviewed_rev(shapes::MIDDLEWARE_WRONG_ENTRY_ID);

    let armed = s.stdout(&[
        "arm",
        shapes::MIDDLEWARE_WRONG_ENTRY_ID,
        "--mode",
        "off",
        "--rev",
        &rev,
    ]);
    assert!(
        armed.contains(shapes::MIDDLEWARE_WRONG_ENTRY_ID) && armed.contains("off"),
        "the act names what it attested: {armed}"
    );

    let parsed = policy::armed::parse_artifact(&s.read(policy::armed::ARMED_RULES_PATH))
        .expect("the artifact parses strictly");
    assert_eq!(parsed.rows().len(), 1);
    assert_eq!(parsed.rows()[0].mode().as_str(), "off");
    assert_eq!(parsed.rows()[0].rev(), rev, "and it pins the reviewed rev");
}

/// The control on the other axis: the twin that differs only in defining
/// `def middleware` arms `block` clean. Without it, the refusal above could
/// mean the gate discriminates on the entry point — or that no
/// `rules/middleware` page arms through this binary at all.
#[test]
fn the_twin_defining_the_middleware_entry_point_arms_block() {
    let s = sandbox();
    s.write("rules/right.md", shapes::MIDDLEWARE_RIGHT_ENTRY);
    let rev = s.reviewed_rev(shapes::MIDDLEWARE_RIGHT_ENTRY_ID);

    s.stdout(&[
        "arm",
        shapes::MIDDLEWARE_RIGHT_ENTRY_ID,
        "--mode",
        "block",
        "--rev",
        &rev,
    ]);

    let parsed = policy::armed::parse_artifact(&s.read(policy::armed::ARMED_RULES_PATH))
        .expect("the artifact parses strictly");
    assert_eq!(parsed.rows().len(), 1);
    assert_eq!(
        parsed.rows()[0].id().as_str(),
        shapes::MIDDLEWARE_RIGHT_ENTRY_ID
    );
    assert_eq!(parsed.rows()[0].mode().as_str(), "block");
    assert_eq!(parsed.rows()[0].rev(), rev);
}

/// The second live shape. Seven of the sessions root's armed rows are `caps: []`
/// hooks declaring `budget:` and `how:` — the loader's `Some(how_value)` arm.
/// It arms `armed` clean through the binary, so the wiring admits a page the
/// fire path loads, and not only refuses ones it does not.
#[test]
fn the_caps_empty_hook_shape_arms_clean() {
    let s = sandbox();
    s.write("rules/idle.md", shapes::HOOK_CAPS_EMPTY);
    let rev = s.reviewed_rev(shapes::HOOK_CAPS_EMPTY_ID);

    s.stdout(&[
        "arm",
        shapes::HOOK_CAPS_EMPTY_ID,
        "--mode",
        "armed",
        "--rev",
        &rev,
    ]);

    let parsed = policy::armed::parse_artifact(&s.read(policy::armed::ARMED_RULES_PATH))
        .expect("the artifact parses strictly");
    assert_eq!(parsed.rows().len(), 1);
    assert_eq!(parsed.rows()[0].id().as_str(), shapes::HOOK_CAPS_EMPTY_ID);
    assert_eq!(parsed.rows()[0].mode().as_str(), "armed");
    assert_eq!(parsed.rows()[0].rev(), rev);
    assert!(
        s.ws.join(policy::ATTESTED_MARKER_PATH).is_file(),
        "the once-armed marker landed"
    );
}
