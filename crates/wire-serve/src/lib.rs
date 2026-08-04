//! Shared typed edge: strict decode + read-op arms for both hosts (A6: lift, don't duplicate).
//!
//! Owns [`decode`] (wire §3.2: unknown fields/enums refuse loud — serde `deny_unknown_fields`
//! does not compose with `flatten`), [`read`] arms over borrowed model state, and
//! [`write::splice`] (W1 choke-point both hosts commit through). Read arms never own corpus
//! or disk; callers supply state. Write is stateful (disk/reparse/receipt/policy) but still
//! one shared impl. `root`/`diff` stay with each host (different cursor/history plumbing).

pub mod armed_disk;
pub mod check_write;
pub mod decode;
pub mod gate;
pub mod guard;
pub mod ladder;
pub mod plan;
pub(crate) mod positions;
pub mod reaction;
pub mod read;
pub mod rev;
pub mod write;

use wire::{ErrorBody, ErrorCode, Path, Root};

/// Protocol both hosts speak (wire §3.2, proto-1). Strict decode validates `hello`'s `proto` against this.
pub const PROTO: u32 = 1;

/// Whether `rev` is a contract rev the server serves (v3 amendment). Unknown rev refused loud at hello; never silent downgrade.
#[must_use]
pub fn is_known_rev(rev: &str) -> bool {
    matches!(rev, "v2" | "v3")
}

/// `bad_request` with a human message (wire §8: `bad_request` ⇒ recovery `fix`).
#[must_use]
pub fn bad_request(message: impl Into<String>) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::BadRequest);
    e.message = Some(message.into());
    Box::new(e)
}

/// The section-address recovery clause — the ONE place the engine says how a
/// caller finds a real selector, so no refusal invents its own spelling.
///
/// Selector-aware, because the two selector planes are published by DIFFERENT
/// faces. Heading paths and dewey ordinals ride `toc`, which the human render
/// prints. `^` anchors ride the separate `anchors[]` array, which
/// `render::toc_text` does not print at all — so telling a reader to list the
/// section paths after an anchor miss sends them to a listing their anchor was
/// never in (the dead end recorded at `results/gaps-and-issues.md` § 3.1).
///
/// `display_path` is `None` on the plan-lowering path, which holds a document
/// but no spelling of its name; the clause then names the act without a command
/// rather than inventing a path it cannot know.
#[must_use]
pub fn section_recovery(selector: &str, display_path: Option<&str>) -> String {
    match (selector.starts_with('^'), display_path) {
        (true, Some(p)) => format!(
            "Fix: run `mrd read {p} --json` and read its `anchors[]` — the section map \
             does not list `^` anchors."
        ),
        (true, None) => "Fix: read the page with --json and use its `anchors[]` — the \
             section map does not list `^` anchors."
            .to_owned(),
        (false, Some(p)) => format!(
            "Fix: run `mrd read {p}` with no --section to list this document's section \
             paths."
        ),
        (false, None) => {
            "Fix: read the page with no selector to list its section paths.".to_owned()
        }
    }
}

/// The write plane's partial-state disclosure. A refused batch lands NOTHING,
/// so the refusal says so and a reader never has to guess which edits took —
/// the write-side twin of `config::NO_PARTIAL_LOAD_CLAUSE`.
pub const NO_PARTIAL_WRITE_CLAUSE: &str = "No edit was applied; the batch is refused whole.";

/// `fs::load` with §8 error split: `file_not_found` / `invalid_utf8` / `io_error{cause}`.
/// Shared fs→wire mapper for both hosts' reads and the write path.
///
/// # Errors
/// Wire envelope for missing file, non-UTF-8, or I/O failure.
pub fn load_doc(root: &fs::WorkspaceRoot, path: &Path) -> Result<model::Document, Box<ErrorBody>> {
    fs::load(root, std::path::Path::new(&path.0)).map_err(|e| {
        Box::new(match e.kind() {
            std::io::ErrorKind::NotFound => {
                let mut err = ErrorBody::new(ErrorCode::FileNotFound);
                err.path = Some(path.clone());
                err
            }
            std::io::ErrorKind::InvalidData => ErrorBody::new(ErrorCode::InvalidUtf8),
            _ => {
                let mut err = ErrorBody::new(ErrorCode::IoError);
                err.cause = Some(e.to_string());
                err
            }
        })
    })
}

/// Ambient workspace root (v2 §4.1/§12): domain file bytes folded via `model::merkle_root`.
///
/// # Errors
/// Wire `io_error` when domain snapshot read/fold fails.
pub fn ambient_root(root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
    Ok(domain_snapshot(root)?.1)
}

/// §12 hash-domain snapshot: domain file bytes + folded root in one read/fold (no stamp drift).
///
/// # Errors
/// Wire `io_error` when domain snapshot read/fold fails.
pub fn domain_snapshot(
    root: &fs::WorkspaceRoot,
) -> Result<(fs::DomainFiles, Root), Box<ErrorBody>> {
    let (files, folded) = fs::domain_snapshot(root).map_err(|e| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    })?;
    Ok((files, Root(folded.0)))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wire::{ErrorCode, Op};

    use super::{bad_request, is_known_rev};

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        match v {
            Value::Object(map) => map,
            _ => panic!("test frame is an object"),
        }
    }

    #[test]
    fn known_rev_set_is_v2_and_v3() {
        assert!(is_known_rev("v2"));
        assert!(is_known_rev("v3"));
        assert!(!is_known_rev("v4"));
        assert!(!is_known_rev(""));
    }

    #[test]
    fn bad_request_carries_the_fix_class_and_message() {
        let e = bad_request("boom");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(e.recovery, wire::Recovery::Fix);
        assert_eq!(e.message.as_deref(), Some("boom"));
    }

    #[test]
    fn decode_accepts_a_read_op() {
        let op = super::decode::decode(
            &obj(json!({"op": "toc", "path": "a.md"})),
            super::rev::Rev::V2,
        )
        .expect("toc decodes");
        assert!(matches!(op, Op::Toc { path } if path.0 == "a.md"));
    }

    #[test]
    fn decode_rejects_an_unknown_field_by_name() {
        // serde would silently drop `bogus`; strict pass refuses loud.
        let e = super::decode::decode(
            &obj(json!({"op": "toc", "path": "a.md", "bogus": 1})),
            super::rev::Rev::V2,
        )
        .expect_err("unknown field is refused");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(
            e.message.as_deref().is_some_and(|m| m.contains("bogus")),
            "the refusal names the field: {:?}",
            e.message
        );
    }

    #[test]
    fn decode_answers_unknown_op_for_an_unarmed_name() {
        let e = super::decode::decode(&obj(json!({"op": "nope"})), super::rev::Rev::V2)
            .expect_err("unknown op refused");
        assert_eq!(e.code, ErrorCode::UnknownOp);
    }

    fn plan_splice() -> Value {
        json!({"op": "splice", "path": "a.md",
            "plan_edits": [{"append": {"hpath": "A", "body": "x"}}]})
    }

    /// Frozen v2 negative: `plan_edits` under v2 hits unknown-field wall byte-for-byte.
    #[test]
    fn v2_splice_plan_edits_refuses_on_the_frozen_wall() {
        let e = super::decode::decode(&obj(plan_splice()), super::rev::Rev::V2)
            .expect_err("v2 plan_edits refused");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(
            e.message.as_deref(),
            Some("unknown request field `plan_edits` on `splice`"),
            "the frozen check_fields refusal, verbatim"
        );
    }

    /// v3 decodes all five plan shapes into the typed union.
    #[test]
    fn v3_splice_plan_edits_decodes_all_shapes() {
        let frame = json!({"op": "splice", "path": "a.md", "plan_edits": [
            {"append": {"hpath": "A", "body": "x"}},
            {"match": {"hpath": "A", "old": "a", "new": "b", "all": true, "rev": "r"}},
            {"replace_section": {"hpath": "A", "body": "x", "rev": "r"}},
            {"create": {"parent_hpath": "A", "title": "B", "body": "x"}},
            {"set_property": {"key": "k", "value": "v"}},
        ]});
        let op = super::decode::decode(&obj(frame), super::rev::Rev::V3).expect("v3 decodes");
        let Op::Splice {
            edits, plan_edits, ..
        } = op
        else {
            panic!("splice op");
        };
        assert!(edits.is_empty(), "plan form carries no native edits");
        assert_eq!(plan_edits.len(), 5);
        assert!(matches!(&plan_edits[1],
            wire::PlanEdit::Match { all: true, rev: Some(r), .. } if r == "r"));
    }

    /// S7 pin: v3-only, strict keys, no actor key (D13); pin-only splice is a complete batch.
    #[test]
    fn v3_splice_pin_decodes_and_carries_no_actor() {
        let frame = json!({"op": "splice", "path": "plan.md",
            "pin": {"target": "guide.md", "selector": "Guide/Q3", "vibe": true}});
        let op = super::decode::decode(&obj(frame), super::rev::Rev::V3).expect("v3 decodes");
        let Op::Splice { edits, pin, .. } = op else {
            panic!("splice op");
        };
        assert!(edits.is_empty(), "a pin-only splice needs no edits");
        let pin = pin.expect("the pin decoded");
        assert_eq!(pin.target, wire::Path("guide.md".into()));
        assert_eq!(pin.selector, "Guide/Q3");
        assert_eq!(pin.vibe, Some(true));

        // D13: no pin.actor — identity is splice's daemon-derived actor only.
        let forged = json!({"op": "splice", "path": "plan.md",
            "pin": {"target": "guide.md", "selector": "Q3", "actor": "someone-else"}});
        let e = super::decode::decode(&obj(forged), super::rev::Rev::V3)
            .expect_err("a pin actor is not a field");
        assert_eq!(
            e.message.as_deref(),
            Some("unknown request field `actor` on `pin`")
        );
    }

    /// v2 wall and per-key strictness around pin.
    #[test]
    fn splice_pin_strict_negatives() {
        // v2 never heard of `pin` — frozen unknown-field refusal.
        let v2 = json!({"op": "splice", "path": "plan.md",
            "pin": {"target": "guide.md", "selector": "Q3"}});
        let e = super::decode::decode(&obj(v2.clone()), super::rev::Rev::V2)
            .expect_err("v2 refuses the pin field");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(
            e.message.as_deref(),
            Some("unknown request field `pin` on `splice`")
        );

        for (frame, want) in [
            (
                json!({"op": "splice", "path": "p.md", "pin": {"target": "g.md"}}),
                "missing `selector` on `pin`",
            ),
            (
                json!({"op": "splice", "path": "p.md", "pin": {"selector": "Q3"}}),
                "missing `target` on `pin`",
            ),
            (
                json!({"op": "splice", "path": "p.md",
                       "pin": {"target": "g.md", "selector": "  "}}),
                "`pin.selector` must name a section (a sanitized heading path or `^id`)",
            ),
            (
                json!({"op": "splice", "path": "p.md", "pin": []}),
                "`pin` must be an object on `splice`",
            ),
        ] {
            let e = super::decode::decode(&obj(frame), super::rev::Rev::V3)
                .expect_err("strict pin decode");
            assert_eq!(e.message.as_deref(), Some(want));
        }
    }

    /// Full create field set decodes into typed birth op, `if_root` included.
    #[test]
    fn create_decodes_its_whole_field_set() {
        let frame = json!({"id": 9, "op": "create", "path": "notes/newborn.md",
            "body": "# X\n", "actor": "agent:e7c3bb2e",
            "now": "2026-07-26T13:30:00-04:00", "if_root": "b3:a", "dry": true});
        let op = super::decode::decode(&obj(frame), super::rev::Rev::V3).expect("create decodes");
        let Op::Create {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        } = op
        else {
            panic!("create op");
        };
        assert_eq!(path, wire::Path("notes/newborn.md".into()));
        assert_eq!(body, "# X\n");
        assert_eq!(actor.as_deref(), Some("agent:e7c3bb2e"));
        assert_eq!(now.as_deref(), Some("2026-07-26T13:30:00-04:00"));
        assert_eq!(if_root, Some(wire::Root("b3:a".into())));
        assert_eq!(dry, Some(true));
    }

    /// Create decode is rev-agnostic; v3 gate is at dispatch (do not move refusal into decode).
    #[test]
    fn create_decode_is_rev_agnostic_the_gate_is_dispatch() {
        let frame = json!({"op": "create", "path": "a.md", "body": "x"});
        for rev in [super::rev::Rev::V2, super::rev::Rev::V3] {
            let op = super::decode::decode(&obj(frame.clone()), rev)
                .unwrap_or_else(|e| panic!("create decodes under {rev:?}: {e:?}"));
            assert!(matches!(op, Op::Create { .. }));
        }
    }

    /// No `force` / batch vocabulary on create; each unknown field refused by name.
    #[test]
    fn create_refuses_the_fields_it_deliberately_lacks() {
        for extra in ["force", "edits", "receipt", "plan_edits", "pin", "sec"] {
            let mut frame = json!({"op": "create", "path": "a.md", "body": "x"});
            frame[extra] = json!(true);
            let e = super::decode::decode(&obj(frame), super::rev::Rev::V3)
                .expect_err("the field wall refuses it");
            assert_eq!(e.code, ErrorCode::BadRequest);
            assert_eq!(
                e.message.as_deref(),
                Some(format!("unknown request field `{extra}` on `create`").as_str()),
                "the refusal must name `{extra}`"
            );
        }
    }

    /// Anti-vacuity: declared create fields are admitted.
    #[test]
    fn create_admits_exactly_its_declared_fields() {
        for field in super::decode::CREATE_FIELDS {
            let mut frame = json!({"op": "create", "path": "a.md", "body": "x"});
            frame[field] = match field {
                "dry" => json!(true),
                "now" => json!("2026-07-26T13:30:00-04:00"),
                _ => json!("x"),
            };
            let op = super::decode::decode(&obj(frame), super::rev::Rev::V3)
                .unwrap_or_else(|e| panic!("declared field `{field}` is admitted: {e:?}"));
            assert!(matches!(op, Op::Create { .. }));
        }
    }

    /// Strict-decode refusals asserted by cause message (what the client fixes).
    #[test]
    fn create_strict_negatives_name_their_cause() {
        for (frame, want) in [
            (
                json!({"op": "create", "path": "a.md"}),
                "missing `body` on `create`",
            ),
            (
                json!({"op": "create", "body": "x"}),
                "missing `path` on `create`",
            ),
            (
                json!({"op": "create", "path": "a.md", "body": 7}),
                "`body` on `create` must be a string",
            ),
            (
                json!({"op": "create", "path": "a.md", "body": "x", "dry": "yes"}),
                "`dry` on `create` must be a boolean",
            ),
            (
                json!({"op": "create", "path": "a.md", "body": "x", "bogus": 1}),
                "unknown request field `bogus` on `create`",
            ),
            (
                json!({"op": "create", "path": "a.md", "body": "x", "force": true}),
                "unknown request field `force` on `create`",
            ),
            (
                json!({"op": "create", "path": "a.md", "body": "x", "now": "yesterday"}),
                "`now` must be RFC 3339 (§9, validated never generated): `yesterday`",
            ),
        ] {
            let e = super::decode::decode(&obj(frame), super::rev::Rev::V3)
                .expect_err("strict create decode");
            assert_eq!(e.code, ErrorCode::BadRequest);
            assert_eq!(e.message.as_deref(), Some(want));
        }
    }

    /// Path-law wall first: birth cannot be spelled outside workspace (`bad_path` before engine).
    #[test]
    fn create_path_law_refuses_before_the_engine_is_reached() {
        for bad in ["/etc/passwd", "../escape.md", "", "a/../../b.md"] {
            let frame = json!({"op": "create", "path": bad, "body": "x"});
            let e = super::decode::decode(&obj(frame), super::rev::Rev::V3)
                .expect_err("path law refuses the escape");
            assert_eq!(e.code, ErrorCode::BadPath, "path `{bad}`");
            assert_eq!(e.path.as_ref().map(|p| p.0.as_str()), Some(bad));
        }
    }

    /// `plan_edits`: mutual exclusion, empty-array law, per-shape strictness.
    #[test]
    fn v3_plan_edits_strict_negatives() {
        // both edits and plan_edits → bad_request
        let both = json!({"op": "splice", "path": "a.md",
            "edits": [{"target": {"hpath": [{"h": "A"}]}, "edit": {"put": {"at": "end", "text": "x"}}}],
            "plan_edits": [{"append": {"hpath": "A", "body": "x"}}]});
        let e = super::decode::decode(&obj(both), super::rev::Rev::V3).expect_err("both refused");
        assert_eq!(
            e.message.as_deref(),
            Some("`edits` and `plan_edits` are mutually exclusive on `splice`")
        );

        // present-but-empty plan_edits → explicit empty-batch refusal
        let empty = json!({"op": "splice", "path": "a.md", "plan_edits": []});
        let e = super::decode::decode(&obj(empty), super::rev::Rev::V3).expect_err("empty refused");
        assert_eq!(
            e.message.as_deref(),
            Some("`plan_edits` must carry at least one edit")
        );

        // neither → frozen v2 missing-edits refusal
        let neither = json!({"op": "splice", "path": "a.md"});
        let e =
            super::decode::decode(&obj(neither), super::rev::Rev::V3).expect_err("neither refused");
        assert_eq!(e.message.as_deref(), Some("missing `edits` on `splice`"));

        // unknown field inside a shape → named refusal
        let bad_field = json!({"op": "splice", "path": "a.md",
            "plan_edits": [{"append": {"hpath": "A", "body": "x", "bogus": 1}}]});
        let e = super::decode::decode(&obj(bad_field), super::rev::Rev::V3)
            .expect_err("shape field wall");
        assert_eq!(
            e.message.as_deref(),
            Some("unknown field `bogus` in `append`")
        );

        // two tags on one item → exactly-one refusal
        let two_tags = json!({"op": "splice", "path": "a.md",
            "plan_edits": [{"append": {"hpath": "A", "body": "x"},
                            "match": {"hpath": "A", "old": "a", "new": "b"}}]});
        let e = super::decode::decode(&obj(two_tags), super::rev::Rev::V3)
            .expect_err("two tags refused");
        assert!(
            e.message
                .as_deref()
                .is_some_and(|m| m.contains("exactly one")),
            "{:?}",
            e.message
        );
    }
}
