//! The middleware door's five card scenarios (armed-plane Part A2,
//! wire-contract § A.2.1), driven through the REAL write path
//! (`wire_serve::write::{splice, create}`) on throwaway workspaces:
//!
//! (a) transform-this-file from `fields`;
//! (b) `ctx.sql`-selected other files land in the SAME sealed set;
//! (c) a birth lands in the same set;
//! (d) a middleware refusal rolls back everything;
//! (e) `send` appears as an intent on the response, never as a delivery.
//!
//! Tests live in `mrd` (not `wire-serve`) because `ctx.sql` needs the
//! view-backed backend only hosts that link `view` can install (C2 topology
//! law) — `registry::mw_sql::install` here, exactly as `mrd`'s own entry does.

use std::collections::BTreeMap;
use std::path::Path as FsPath;

use policy::armed::Mode;
use wire::{Edit, EditShape, ErrorCode, Path, PutAt, ResponseBody, SecRef};
use wire_serve::write::{CreateArgs, SpliceArgs, create, splice};

/// A middleware page: registers by TAG (`rules/middleware`), scoped to
/// `tasks/**`, entry `def middleware(ctx)`.
fn mw_page(id: &str, starlark: &str) -> String {
    format!(
        "---\ntags: [type/rule, rules/middleware]\nid: {id}\npaths:\n  - tasks/**\n---\n\n\
         # {id}\n\n```starlark\n{starlark}```\n"
    )
}

fn tmp_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn write_page(root: &fs::WorkspaceRoot, path: &str, bytes: &str) {
    let full = root.0.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, bytes).unwrap();
}

/// Arm every `(page, bytes, id, mode)` at the workspace root through the real
/// ARM act, and stamp the once-armed marker.
fn arm(root: &fs::WorkspaceRoot, pages: &[(&str, &str, &str, Mode)]) {
    let index =
        policy::RuleIndex::discover(pages.iter().map(|(path, bytes, ..)| policy::PageRef {
            layer: policy::ScopeLayer::Workspace,
            page: path,
            bytes,
        }));
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        pages
            .iter()
            .map(|(_, bytes, id, mode)| policy::armed::ArmRequest {
                id: policy::RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: policy::page_rev(bytes),
            }),
    )
    .expect("the fixture arms")
    .render();
    write_page(root, fs::domain::ARMED_RULES_PATH, &artifact);
    write_page(root, fs::domain::ATTESTED_MARKER_PATH, "");
}

/// A single-form splice flipping `status` on `path` via the native fm upsert.
fn status_splice(path: &str, value: &str, fields: &[(&str, &str)]) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: Path(path.into()),
        actor: Some("agent:bob".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target: SecRef::FmKey {
                fm_key: "status".into(),
            },
            edit: EditShape::Put {
                at: PutAt::Upsert,
                text: value.into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: fields
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

const CARD: &str = "---\nowner: agent:alice\nstatus: open\n---\n# Fix parser\n\nbody\n";

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// The splice response's armed group + intents.
fn splice_parts(body: &ResponseBody) -> (&wire::Armed, Option<&Vec<wire::MwIntent>>) {
    let ResponseBody::Splice { armed, .. } = body else {
        panic!("splice returns a Splice body");
    };
    (armed, armed.intents.as_ref())
}

// ── (a) transform-this-file from `fields` ───────────────────────────────────

#[test]
fn a_transform_this_file_from_fields_lands_in_the_same_write() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    let page = mw_page(
        "000-stamp-session",
        "def middleware(ctx):\n    session = ctx.fields.get(\"session\")\n    if session != None:\n        set_field(path = ctx.after.path, key = \"session\", value = session)\n",
    );
    write_page(&root, "rules/stamp.md", &page);
    arm(
        &root,
        &[("rules/stamp.md", &page, "000-stamp-session", Mode::Block)],
    );

    let out = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "review", &[("session", "0ecc9d6a")]),
        &[],
        None,
    )
    .expect("the transformed write lands");

    let landed = read(&root, "tasks/fix.md");
    assert!(
        landed.contains("status: review"),
        "caller edit landed: {landed}"
    );
    assert!(
        landed.contains("session: 0ecc9d6a"),
        "the middleware transform landed IN the same write: {landed}"
    );
    let (armed, intents) = splice_parts(&out.body);
    assert_eq!(
        intents.map(Vec::len),
        Some(0),
        "intents is present-empty on a middleware door success"
    );
    assert!(
        armed.effects.is_empty(),
        "the put-path hook feed is retired: no reaction envelopes"
    );
    // The engine-added upsert is reported as an armed edit row too.
    assert_eq!(armed.edits.len(), 2, "caller row + middleware row");
}

// ── (b) sql-selected other files join the sealed set ────────────────────────

#[test]
fn b_sql_selected_files_land_in_the_same_sealed_set_as_the_caller_put() {
    registry::mw_sql::install();
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/handoff.md", CARD);
    write_page(
        &root,
        "agents/a1.md",
        "---\nreports-to: old-leader\n---\n# a1\n",
    );
    write_page(
        &root,
        "agents/a2.md",
        "---\nreports-to: old-leader\n---\n# a2\n",
    );
    write_page(&root, "agents/a3.md", "---\nreports-to: other\n---\n# a3\n");
    // On status → review, repoint every agent whose reports-to names
    // old-leader at the new one — the RELATIONSHIP-SYNC missing half.
    let page = mw_page(
        "000-reports-to",
        "def middleware(ctx):\n    if \"status\" not in ctx.put.fields_changed:\n        return\n    rows = ctx.sql(\"SELECT path FROM frontmatter WHERE key = 'reports-to' AND value = 'old-leader'\")\n    for row in rows:\n        set_field(path = row[\"path\"], key = \"reports-to\", value = \"new-leader\")\n",
    );
    write_page(&root, "rules/reports-to.md", &page);
    arm(
        &root,
        &[("rules/reports-to.md", &page, "000-reports-to", Mode::Block)],
    );

    let out = splice(
        &root,
        None,
        &status_splice("tasks/handoff.md", "review", &[]),
        &[],
        None,
    )
    .expect("the handoff set lands");

    assert!(read(&root, "tasks/handoff.md").contains("status: review"));
    assert!(
        read(&root, "agents/a1.md").contains("reports-to: new-leader"),
        "a1 repointed in the same sealed set"
    );
    assert!(
        read(&root, "agents/a2.md").contains("reports-to: new-leader"),
        "a2 repointed in the same sealed set"
    );
    assert!(
        read(&root, "agents/a3.md").contains("reports-to: other"),
        "a3 was never selected"
    );
    // One sealed commit: one Delta, one seq, files[] carrying every member.
    let frame = out.committed.expect("committed");
    let files: Vec<&str> = frame
        .delta
        .files
        .iter()
        .map(|f| f.path.0.as_str())
        .collect();
    assert!(files.contains(&"tasks/handoff.md"), "{files:?}");
    assert!(files.contains(&"agents/a1.md"), "{files:?}");
    assert!(files.contains(&"agents/a2.md"), "{files:?}");
    assert!(!files.contains(&"agents/a3.md"), "{files:?}");
}

// ── (c) birth in the same set ───────────────────────────────────────────────

#[test]
fn c_a_birth_lands_in_the_same_sealed_set() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    let page = mw_page(
        "000-scaffold-followup",
        "def middleware(ctx):\n    if ctx.after.frontmatter.get(\"status\") == \"done\":\n        create(path = \"tasks/followup.md\", body = \"---\\nstatus: open\\n---\\n# Follow up\\n\")\n",
    );
    write_page(&root, "rules/scaffold.md", &page);
    arm(
        &root,
        &[(
            "rules/scaffold.md",
            &page,
            "000-scaffold-followup",
            Mode::Block,
        )],
    );

    let out = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "done", &[]),
        &[],
        None,
    )
    .expect("the birth-carrying set lands");

    assert!(read(&root, "tasks/fix.md").contains("status: done"));
    let born = read(&root, "tasks/followup.md");
    assert!(
        born.contains("# Follow up"),
        "the follow-up card was born: {born}"
    );
    let frame = out.committed.expect("committed");
    let files: Vec<&str> = frame
        .delta
        .files
        .iter()
        .map(|f| f.path.0.as_str())
        .collect();
    assert!(
        files.contains(&"tasks/followup.md"),
        "the birth rides the ONE Delta: {files:?}"
    );

    // The flip is atomic with the birth: replaying the same put now refuses
    // (occupied birth path) and the caller file stays at its landed bytes.
    let before = read(&root, "tasks/fix.md");
    let err = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "done", &[]),
        &[],
        None,
    )
    .expect_err("re-birthing an occupied path refuses the whole set");
    assert_eq!(err.code, ErrorCode::CasMismatch, "{err:?}");
    assert_eq!(
        read(&root, "tasks/fix.md"),
        before,
        "nothing landed on the refusal"
    );
}

// ── (d) refuse rolls back everything ────────────────────────────────────────

#[test]
fn d_a_middleware_refusal_lands_nothing() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    let page = mw_page(
        "000-no-self-review",
        "def middleware(ctx):\n    if ctx.after.frontmatter.get(\"status\") == \"review\":\n        set_field(path = \"agents/witness.md\", key = \"saw\", value = \"yes\")\n        create(path = \"tasks/never.md\", body = \"# never\\n\")\n        refuse(message = \"review is closed on this board\", passing = \"rules/no-self-review.md#legal\")\n",
    );
    write_page(&root, "rules/no-self-review.md", &page);
    write_page(&root, "agents/witness.md", "---\nsaw: no\n---\n# w\n");
    arm(
        &root,
        &[(
            "rules/no-self-review.md",
            &page,
            "000-no-self-review",
            Mode::Block,
        )],
    );

    let before_card = read(&root, "tasks/fix.md");
    let before_witness = read(&root, "agents/witness.md");
    let err = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "review", &[]),
        &[],
        None,
    )
    .expect_err("the middleware refusal refuses the whole write");
    assert_eq!(err.code, ErrorCode::ConventionFault);
    let msg = err.message.as_deref().unwrap_or("");
    assert!(msg.contains("000-no-self-review"), "names the rule: {msg}");
    assert!(
        msg.contains("rules/no-self-review.md#legal"),
        "cites the passing scenario: {msg}"
    );
    assert_eq!(
        read(&root, "tasks/fix.md"),
        before_card,
        "caller bytes untouched"
    );
    assert_eq!(
        read(&root, "agents/witness.md"),
        before_witness,
        "the pre-refusal set_field never landed"
    );
    assert!(
        !root.0.join("tasks/never.md").exists(),
        "the pre-refusal birth never landed"
    );
}

// ── (e) send is an intent, never a delivery ─────────────────────────────────

#[test]
fn e_send_rides_the_response_as_an_intent_and_nothing_claims_delivery() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    let page = mw_page(
        "000-notify-review",
        "def middleware(ctx):\n    if ctx.after.frontmatter.get(\"status\") == \"review\":\n        send(to = [ctx.after.frontmatter.get(\"owner\")], body = \"fix -> review\")\n",
    );
    write_page(&root, "rules/notify.md", &page);
    arm(
        &root,
        &[("rules/notify.md", &page, "000-notify-review", Mode::Block)],
    );

    let out = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "review", &[]),
        &[],
        None,
    )
    .expect("the send-emitting write lands");
    let (armed, intents) = splice_parts(&out.body);
    let intents = intents.expect("intents present on a middleware door success");
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].kind, "send");
    assert_eq!(intents[0].to, vec!["agent:alice".to_string()]);
    assert_eq!(intents[0].body, "fix -> review");
    assert_eq!(intents[0].rule_id, "000-notify-review");
    assert!(
        armed.effects.is_empty(),
        "no reaction envelope on the put path"
    );
    // Nothing anywhere claims delivery — the serialized response carries the
    // intent verbatim and no delivery vocabulary.
    let json = serde_json::to_string(&out.body).expect("serializes");
    assert!(json.contains("\"intents\""), "{json}");
    assert!(!json.contains("delivered"), "no delivery claim: {json}");
}

// ── the ctx.fields wire law rides the frame, opaque ─────────────────────────

#[test]
fn fields_are_opaque_and_absent_fields_is_the_empty_map() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    let page = mw_page(
        "000-fields-echo",
        "def middleware(ctx):\n    if len(ctx.fields) == 0:\n        set_field(path = ctx.after.path, key = \"fields-seen\", value = \"none\")\n",
    );
    write_page(&root, "rules/echo.md", &page);
    arm(
        &root,
        &[("rules/echo.md", &page, "000-fields-echo", Mode::Block)],
    );
    splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "open", &[]),
        &[],
        None,
    )
    .expect("fieldless put still evaluates middleware");
    assert!(read(&root, "tasks/fix.md").contains("fields-seen: none"));
}

// ── the birth door: this-file transform + intents ───────────────────────────

#[test]
fn create_door_transforms_the_born_bytes_and_carries_intents() {
    let (_dir, root) = tmp_ws();
    let page = mw_page(
        "000-stamp-birth",
        "def middleware(ctx):\n    if ctx.op != \"create\":\n        return\n    created = ctx.fields.get(\"created\")\n    if created != None:\n        set_field(path = ctx.after.path, key = \"created\", value = created)\n    send(to = \"leader\", body = \"born: \" + ctx.after.path)\n",
    );
    write_page(&root, "rules/stamp-birth.md", &page);
    arm(
        &root,
        &[(
            "rules/stamp-birth.md",
            &page,
            "000-stamp-birth",
            Mode::Block,
        )],
    );

    let mut fields = BTreeMap::new();
    fields.insert("created".to_string(), "2026-08-17".to_string());
    let out = create(
        &root,
        None,
        &CreateArgs {
            id: None,
            path: Path("tasks/born.md".into()),
            body: "---\nstatus: open\n---\n# Born\n".into(),
            actor: Some("agent:bob".into()),
            now: None,
            if_root: None,
            dry: false,
            fields,
        },
        &[],
    )
    .expect("the transformed birth lands");

    let born = read(&root, "tasks/born.md");
    assert!(
        born.contains("created: 2026-08-17"),
        "the stamp landed IN the born bytes — one receipt, no unstamped birth: {born}"
    );
    let intents = out.intents.expect("intents present on a landed birth");
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].to, vec!["leader".to_string()]);
    assert!(FsPath::new("tasks/born.md").is_relative());
}

// ── fail-closed: an unevaluable middleware refuses ──────────────────────────

#[test]
fn an_unevaluable_middleware_fails_closed() {
    let (_dir, root) = tmp_ws();
    write_page(&root, "tasks/fix.md", CARD);
    // Reaching for hook vocabulary faults at eval — the door must refuse, not
    // silently skip the law.
    let page = mw_page(
        "000-broken",
        "def middleware(ctx):\n    intent(action = \"notify\")\n",
    );
    write_page(&root, "rules/broken.md", &page);
    arm(
        &root,
        &[("rules/broken.md", &page, "000-broken", Mode::Block)],
    );
    let before = read(&root, "tasks/fix.md");
    let err = splice(
        &root,
        None,
        &status_splice("tasks/fix.md", "review", &[]),
        &[],
        None,
    )
    .expect_err("a middleware that cannot complete never reads as a pass");
    assert_eq!(err.code, ErrorCode::ConventionFault);
    assert!(
        err.message.as_deref().unwrap_or("").contains("000-broken"),
        "{err:?}"
    );
    assert_eq!(read(&root, "tasks/fix.md"), before, "nothing landed");
}
