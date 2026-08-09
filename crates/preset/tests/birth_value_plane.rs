//! The BIRTH DOOR is a value-plane write door (wire-contract § A.6.3a) — the
//! third door beside `set_property` and `put{at:"upsert"}`.
//!
//! Measured defect this gate holds shut (dogfood pass 1, f03, re-measured at
//! HEAD): `mrd new --actor $'zt\nstatus: closed'` interpolated the caller's
//! bytes into the born frontmatter, so the record carried `status:` TWICE. Disk
//! said `closed`, every read door served `open` (§ A.3 first-occurrence-wins),
//! and no governed edit could reach the shadow line — the tool wrote something
//! only a non-meridian editor could remove.
//!
//! Both halves are asserted here: a value that cannot be represented as ONE
//! frontmatter line is REFUSED (never sanitized — §3.4 stamps the caller's
//! actor exactly as given), and a value that CAN be represented round-trips
//! through the § A.6.1 decode the read seams serve.

use preset::{BirthOptions, NewOutcome, new_record};

/// A def whose `^template` puts the caller's `{{actor}}` in a frontmatter VALUE
/// position — the shape the dogfood preset used, and the shape the corpus's
/// session presets use.
const ACTOR_PRESET: &str = r#"---
type: def
defines: session
root: SESSION.md
births: "sessions/{{id}}.md"
---

# Properties ^properties

- `type` required
- `status` required

# Template ^template

```record
---
type: session
status: open
owner: {{actor}}
---

# Session {{id}}
```

# Unfold

- SESSION.md
"#;

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("presets/session.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, ACTOR_PRESET).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn birth(root: &fs::WorkspaceRoot, id: &str, actor: &str) -> NewOutcome {
    let opts = BirthOptions {
        actor: Some(actor.to_owned()),
        now: Some("2026-08-09T12:00:00Z".to_owned()),
        dry: false,
    };
    new_record(root, "presets/session.md", id, &opts).unwrap()
}

/// The `owner:` line of a born record, and how many `status:` lines it carries —
/// the two facts that decide this defect.
fn lines(root: &fs::WorkspaceRoot, id: &str) -> (String, usize) {
    let body = std::fs::read_to_string(root.0.join(format!("sessions/{id}.md"))).unwrap();
    let owner = body
        .lines()
        .find(|l| l.starts_with("owner:"))
        .unwrap_or_default()
        .to_owned();
    let statuses = body.lines().filter(|l| l.starts_with("status:")).count();
    (owner, statuses)
}

/// HALF ONE — a multi-line actor births NOTHING. At the release pin this born a
/// record carrying `status:` twice; the value cannot be one frontmatter line, an
/// escaped-scalar workaround leaks (§ A.6.3), and sanitizing would falsify the
/// provenance §3.4 requires stamped exactly as given. So: refusal.
#[test]
fn multi_line_actor_refuses_the_birth() {
    let (_tmp, root) = workspace();

    let outcome = birth(&root, "inj1", "zt\nstatus: closed");

    let NewOutcome::Refused(refusal) = outcome else {
        panic!("a multi-line actor must not birth a record");
    };
    assert_eq!(refusal.reason.code, "bad_request");
    assert_eq!(refusal.reason.recovery, "fix");
    assert!(
        !root.0.join("sessions/inj1.md").exists(),
        "the refusal wrote no bytes"
    );
    // The uniform § A.6.3a sentence — the key by name, the v1 single-line rule,
    // the body-section escape — plus this door's provenance clause.
    let message = &refusal.reason.message;
    assert!(message.contains("\"owner\""), "names the key: {message}");
    assert!(
        message.contains("put multi-line content in a body section"),
        "teaches the escape: {message}"
    );
    assert!(
        message.contains("{{actor}}"),
        "names the birth placeholder that carried the newline: {message}"
    );
}

/// A carriage return is the same refusal — the newline set is `\n` AND `\r` at
/// every other value-plane door, and a door that refuses one but not the other
/// is a second dialect of one law.
#[test]
fn carriage_return_actor_refuses_the_birth() {
    let (_tmp, root) = workspace();

    let outcome = birth(&root, "inj2", "zt\rstatus: closed");

    let NewOutcome::Refused(refusal) = outcome else {
        panic!("a `\\r` actor must not birth a record");
    };
    assert_eq!(refusal.reason.code, "bad_request");
}

/// HALF TWO — a value that CAN be one line is born as one line and reads back
/// as exactly the caller's string. `zt: closed` is the injection shape without
/// the newline: at the pin it landed as a bare mapping in value position.
#[test]
fn mapping_shaped_actor_is_encoded_not_injected() {
    let (_tmp, root) = workspace();

    let outcome = birth(&root, "a3", "zt: closed");

    assert!(matches!(outcome, NewOutcome::Born(_)));
    let (owner, statuses) = lines(&root, "a3");
    assert_eq!(owner, r#"owner: "zt: closed""#);
    assert_eq!(statuses, 1, "one key, one line");
    assert_eq!(
        model::scalar::text(owner.strip_prefix("owner: ").unwrap()),
        "zt: closed",
        "the § A.6.1 decode gives back the caller's exact string"
    );
}

/// The fleet-canonical wikilink value: quoted at birth, so the born record is
/// I4-conformant and the read seams decode it back whole. The same encoder the
/// other two doors use — a third dialect here is the drift § A.6.3 forbids.
#[test]
fn wikilink_actor_round_trips_through_the_decode() {
    let (_tmp, root) = workspace();

    let outcome = birth(&root, "a2", "[[b1892b5a]]");

    assert!(matches!(outcome, NewOutcome::Born(_)));
    let (owner, _) = lines(&root, "a2");
    assert_eq!(owner, r#"owner: "[[b1892b5a]]""#);
    assert_eq!(
        model::scalar::text(owner.strip_prefix("owner: ").unwrap()),
        "[[b1892b5a]]"
    );
}

/// The CONTROL — an ordinary value gains nothing. The encoder emits the plain
/// form whenever the plain form decodes back to the caller's string, so this fix
/// does not re-spell the corpus's existing births.
#[test]
fn plain_actor_stays_plain() {
    let (_tmp, root) = workspace();

    let outcome = birth(&root, "a4", "zt");

    assert!(matches!(outcome, NewOutcome::Born(_)));
    let (owner, statuses) = lines(&root, "a4");
    assert_eq!(owner, "owner: zt");
    assert_eq!(statuses, 1);
}

/// The BODY is not the frontmatter plane: a multi-line value is legal there and
/// stamps verbatim, because §3.4's "exactly as given" is the whole point and no
/// § A.6 law governs body bytes. Refusing here would be the sanitizing fix
/// wearing a refusal's clothes.
#[test]
fn a_body_placeholder_is_not_governed_by_the_value_plane() {
    let tmp = tempfile::tempdir().unwrap();
    let def = r#"---
type: def
defines: note
births: "notes/{{id}}.md"
---

# Properties ^properties

- `type` required

# Template ^template

```record
---
type: note
---

# {{id}}

born by {{actor}}
```
"#;
    let path = tmp.path().join("presets/note.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, def).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());

    let opts = BirthOptions {
        actor: Some("zt\nsecond line".to_owned()),
        now: None,
        dry: false,
    };
    let outcome = new_record(&root, "presets/note.md", "n1", &opts).unwrap();

    assert!(matches!(outcome, NewOutcome::Born(_)));
    let body = std::fs::read_to_string(root.0.join("notes/n1.md")).unwrap();
    assert!(
        body.contains("born by zt\nsecond line"),
        "the body carries the caller's bytes verbatim: {body:?}"
    );
}
