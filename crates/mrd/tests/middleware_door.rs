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
    // The act loads every firing winner — a `path → bytes` map IS a `PageSource`.
    let source: BTreeMap<String, String> = pages
        .iter()
        .map(|(path, bytes, ..)| ((*path).to_string(), (*bytes).to_string()))
        .collect();
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
        &source,
        policy::CheckLimits::default(),
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
    assert_eq!(
        armed.set.as_ref().map(Vec::len),
        Some(0),
        "set is present-empty when the middleware touched no OTHER file (§ A.2.1)"
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

    // § A.2.1 `armed.set`: the response names every OTHER file the sealed
    // set committed — repeated from the Delta rows above, caller absent,
    // each member attributed to the middleware that compiled it.
    let (armed, _) = splice_parts(&out.body);
    let set = armed
        .set
        .as_ref()
        .expect("set present on a mw-door success");
    let mut set_paths: Vec<&str> = set.iter().map(|m| m.path.0.as_str()).collect();
    set_paths.sort_unstable();
    assert_eq!(
        set_paths,
        vec!["agents/a1.md", "agents/a2.md"],
        "members only, never the caller"
    );
    for m in set {
        assert_eq!(m.change, wire::FileChange::Modified, "{m:?}");
        assert_eq!(m.rules, vec!["000-reports-to".to_string()], "{m:?}");
        let delta_row = frame
            .delta
            .files
            .iter()
            .find(|f| f.path == m.path)
            .expect("every set row has its Delta row");
        assert_eq!(
            m.file_rev_after, delta_row.file_rev_after,
            "the set row repeats the commit's own rev, never re-derives one"
        );
    }
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

    // § A.2.1 `armed.set`: the birth is a cross-file effect and rides the
    // response as a `created` member attributed to its middleware.
    let (armed, _) = splice_parts(&out.body);
    let set = armed
        .set
        .as_ref()
        .expect("set present on a mw-door success");
    assert_eq!(set.len(), 1, "{set:?}");
    assert_eq!(set[0].path.0, "tasks/followup.md");
    assert_eq!(set[0].change, wire::FileChange::Created);
    assert_eq!(set[0].rules, vec!["000-scaffold-followup".to_string()]);

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
            props: BTreeMap::default(),
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

/// The birth-stamp regression: THREE `set_field` emits on a
/// frontmatterless birth land ONE `---` block with every key parsing — the
/// broken door compiled each upsert against the pre-edit blockless snapshot
/// and stacked three blocks, so only `created` survived the parse. A birth
/// already carrying a block keeps merging into it, caller keys preserved.
#[test]
fn create_door_stamps_a_frontmatterless_birth_with_one_block() {
    let (_dir, root) = tmp_ws();
    let page = mw_page(
        "000-born-identity",
        "def middleware(ctx):\n    if ctx.op != \"create\":\n        return\n    set_field(path = ctx.after.path, key = \"created\", value = \"2026-08-18T13:24\")\n    set_field(path = ctx.after.path, key = \"session\", value = \"18-00-adhoc\")\n    set_field(path = ctx.after.path, key = \"spawned-by\", value = \"[[64cb50a1]]\")\n",
    );
    write_page(&root, "rules/born-identity.md", &page);
    arm(
        &root,
        &[(
            "rules/born-identity.md",
            &page,
            "000-born-identity",
            Mode::Block,
        )],
    );

    let birth = |path: &str, body: &str| CreateArgs {
        id: None,
        path: Path(path.into()),
        body: body.into(),
        actor: None,
        now: None,
        if_root: None,
        dry: false,
        fields: BTreeMap::new(),
        props: BTreeMap::default(),
    };
    create(
        &root,
        None,
        &birth("tasks/blockless.md", "# zz-born-card\n"),
        &[],
    )
    .expect("the frontmatterless birth lands");
    assert_eq!(
        read(&root, "tasks/blockless.md"),
        "---\ncreated: 2026-08-18T13:24\nsession: 18-00-adhoc\n\
         spawned-by: \"[[64cb50a1]]\"\n---\n# zz-born-card\n",
        "one merged block, every stamp a parsing key"
    );

    create(
        &root,
        None,
        &birth(
            "tasks/blocked.md",
            "---\nstatus: Todo\n---\n# zz-born-card2\n",
        ),
        &[],
    )
    .expect("the block-carrying birth lands");
    assert_eq!(
        read(&root, "tasks/blocked.md"),
        "---\ncreated: 2026-08-18T13:24\nsession: 18-00-adhoc\n\
         spawned-by: \"[[64cb50a1]]\"\nstatus: Todo\n---\n# zz-born-card2\n",
        "stamps merge into the existing block, caller keys preserved"
    );
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

/// **D6 × born identity** (card 17): a birth through `props=` still meets the
/// armed door — the door composes the caller's frontmatter FIRST, so a
/// fill-if-absent middleware reads the caller's keys and stamps beside them,
/// one block, one receipt. This is the born-card shape end to end: the program
/// hands over a dict, the door quotes it, `000` adds `created`/`session`.
#[test]
fn a_birth_through_props_is_composed_before_middleware_and_still_stamped() {
    let (_dir, root) = tmp_ws();
    // The born-card scope is `agents/**`, not the shared helper's `tasks/**`.
    let page = format!(
        "---\ntags: [type/rule, rules/middleware]\nid: 000-born-identity\npaths:\n  - agents/**\n---\n\n# 000-born-identity\n\n```starlark\n{}```\n",
        "def middleware(ctx):\n    if ctx.op != \"create\":\n        return\n    if ctx.after.frontmatter.get(\"type\") != \"agent\":\n        return\n    created = ctx.fields.get(\"created\")\n    if created != None:\n        set_field(path = ctx.after.path, key = \"created\", value = created)\n    session = ctx.fields.get(\"session\")\n    if session != None:\n        set_field(path = ctx.after.path, key = \"session\", value = session)\n",
    );
    write_page(&root, "rules/born-identity.md", &page);
    arm(
        &root,
        &[(
            "rules/born-identity.md",
            &page,
            "000-born-identity",
            Mode::Block,
        )],
    );

    let mut fields = BTreeMap::new();
    fields.insert(
        "created".to_string(),
        "2026-08-23T01:09:34-04:00".to_string(),
    );
    fields.insert(
        "session".to_string(),
        "19-20-mrd-statusd-integration".to_string(),
    );
    let mut props = BTreeMap::new();
    props.insert(
        "type".to_string(),
        wire_serve::write::PropValue::Scalar("agent".to_string()),
    );
    props.insert(
        "manifest".to_string(),
        // The hostile half rides the same birth: a manifest a hand-rolled
        // escaper would have leaked a key through.
        wire_serve::write::PropValue::Scalar("worker: card 17 — \"props\" at the door".to_string()),
    );
    props.insert(
        "tags".to_string(),
        wire_serve::write::PropValue::List(vec!["type/agent".to_string()]),
    );
    // F4 (review of PR 185): a props key COLLIDING with a machinery key. The
    // caller cannot win it — `created` is born identity, stamped by the door's
    // middleware from the put frame's `fields`, and the props value is the
    // caller's guess at a key that is not theirs. Pinned so the precedence is a
    // decision, not an accident of which write runs last.
    props.insert(
        "created".to_string(),
        wire_serve::write::PropValue::Scalar("1999-01-01T00:00:00-00:00".to_string()),
    );
    create(
        &root,
        None,
        &CreateArgs {
            id: None,
            path: Path("agents/f6656ff1/f6656ff1.md".into()),
            body: "# Memo\n\n# Todo\n".into(),
            actor: Some("agent:f6656ff1".into()),
            now: None,
            if_root: None,
            dry: false,
            fields,
            props,
        },
        &[],
    )
    .expect("the props birth lands");

    let born = read(&root, "agents/f6656ff1/f6656ff1.md");
    assert!(
        born.contains("created: 2026-08-23T01:09:34-04:00"),
        "000 still stamps created on a card born through props=: {born}"
    );
    // F4: the collision resolves to the MACHINERY value, and the caller's
    // forged one is gone from the bytes — not merely outranked in a reader.
    assert!(
        !born.contains("1999-01-01"),
        "a props key colliding with a machinery key loses: the middleware's \
         stamp is the one in the file, and the caller's value is not in the \
         bytes at all: {born}"
    );
    assert_eq!(
        born.lines().filter(|l| l.starts_with("created:")).count(),
        1,
        "one created key, not two: {born}"
    );
    assert!(
        born.contains("session: 19-20-mrd-statusd-integration"),
        "000 still stamps session: {born}"
    );
    assert!(
        born.contains("tags: [type/agent]") && born.contains("type: agent"),
        "the caller's own props landed: {born}"
    );
    assert_eq!(
        born.lines().filter(|l| *l == "---").count(),
        2,
        "one frontmatter block, hostile manifest and all: {born}"
    );
    let meta = policy::defs::parse_meta(&born)
        .expect("the born frontmatter parses")
        .expect("frontmatter present");
    assert_eq!(
        meta.get("manifest"),
        Some(&policy::defs::FmValue::Str(
            "worker: card 17 — \"props\" at the door".to_string()
        )),
        "the hostile manifest reads back byte for byte: {born}"
    );
    assert!(
        born.ends_with("# Memo\n\n# Todo\n"),
        "body verbatim: {born}"
    );
}
