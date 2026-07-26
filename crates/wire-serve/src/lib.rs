//! The shared typed edge — strict decode + the read-op arms — lifted out of the
//! sidecar so the resident registry daemon and the per-workspace sidecar answer
//! the wire contract through ONE implementation (arch map A6: "lift, don't
//! duplicate").
//!
//! # Charter
//! **Owns:** the hand-rolled strict-decode pass ([`decode`] — wire §3.2 server
//! law: unknown request fields and unknown enum values are rejected loudly,
//! because serde's `deny_unknown_fields` does not compose with `flatten`), the
//! read-op arms ([`read`] — `toc`/`cat`/`extract`/`links`/`resolve`) served over
//! BORROWED parsed state, and the WRITE choke-point ([`write::splice`] — the
//! `splice → commit` seam both hosts commit through, W1).
//!
//! **The read arms never** own the corpus, read disk, parse, or hold a serve
//! loop: each takes already-built `model` state (`&Document`, or the
//! `&CorpusIndex` + document map) and the ambient root as data; the CALLER
//! obtains that state — the sidecar builds it per request, the daemon reuses its
//! warm engine. That split is the whole point: one projection, two state
//! sources, no drift.
//!
//! **The write choke-point is inherently stateful** — a commit IS disk I/O — so
//! [`write`] necessarily reads + writes disk (`fs`), reparses the after-state
//! (`syntax`), renders receipts (`receipt`), and evaluates verdicts (`policy`).
//! It is still ONE implementation both hosts share; the delta ring stays with the
//! caller (see [`write`]). The disk-load + fold helpers the read arms and the
//! write path both use — [`load_doc`], [`ambient_root`], [`domain_snapshot`] —
//! live here too, so there is one fs→wire mapper, not two.
//!
//! # Root / diff live with the caller
//! `root` and `diff` are cursor/history plumbing whose source genuinely differs
//! by host (the sidecar owns a per-epoch ring; the resident daemon has none
//! until the watcher lands), so they are NOT arms here — each host builds those
//! two responses from its own `seq`/history. This crate holds only the corpus
//! reads whose logic is identical across hosts.

pub mod check_write;
pub mod decode;
pub mod gate;
pub mod plan;
pub(crate) mod positions;
pub mod read;
pub mod rev;
pub mod write;

use wire::{ErrorBody, ErrorCode, Path, Root};

/// The one protocol both hosts speak (wire §3.2, proto-1). The strict decode
/// validates a `hello`'s `proto` against this.
pub const PROTO: u32 = 1;

/// Is `rev` a contract rev the server serves (wire v3 amendment)? An unknown
/// declared rev is refused LOUD at `hello` decode, never silently downgraded.
/// The negotiation itself (which rev a session runs) is the caller's; this is
/// only the decode-time known-set check.
#[must_use]
pub fn is_known_rev(rev: &str) -> bool {
    matches!(rev, "v2" | "v3")
}

/// A `bad_request` carrying a human message — the strict-decode workhorse
/// (wire §8: `bad_request` ⇒ recovery `fix`).
#[must_use]
pub fn bad_request(message: impl Into<String>) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::BadRequest);
    e.message = Some(message.into());
    Box::new(e)
}

/// `fs::load` with the §8 error split: `file_not_found` (env — the file is gone,
/// path echoed), `invalid_utf8` (refused, never lossy-decoded), `io_error{cause}`
/// otherwise. Shared by both hosts' single-file reads and the write path (which
/// loads fresh disk before validating) — one fs→wire mapper, not two.
///
/// # Errors
/// The wire envelope for a missing file, a non-UTF-8 file, or an I/O failure.
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

/// The ambient workspace root (v2 §4.1/§12): the §12 hash domain's file bytes
/// folded through `model::merkle_root` — the one blake3 home. A fresh disk fold
/// shared by the sidecar read arms and the write path's `root_before`/`root_after`
/// (the resident daemon's warm engine carries the same fold as `at_fingerprint`).
///
/// # Errors
/// The wire `io_error` envelope when the domain snapshot read/fold fails.
pub fn ambient_root(root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
    Ok(domain_snapshot(root)?.1)
}

/// The §12 hash-domain snapshot: every domain file's bytes + the root folded over
/// exactly those bytes — one read, one fold, so a consumer parses the same bytes
/// its `as_of_root` describes and the answer cannot drift from its stamp. The
/// shared `fs::domain_snapshot` primitive re-homed into the wire `Root` token and
/// the `io::Error` into the wire `io_error` frame.
///
/// # Errors
/// The wire `io_error` envelope when the domain snapshot read/fold fails.
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
        // serde would silently drop `bogus`; the strict pass refuses it loud.
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

    // ------------------------------------------------------------------
    // M1 U8b rider-1 fixtures: the rev-threaded splice field wall.
    // ------------------------------------------------------------------

    fn plan_splice() -> Value {
        json!({"op": "splice", "path": "a.md",
            "plan_edits": [{"append": {"hpath": "A", "body": "x"}}]})
    }

    /// The FROZEN v2 negative (rider 1): `plan_edits` under a v2 session hits
    /// the existing unknown-field wall, byte-for-byte.
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

    /// S7 `splice.pin`: v3-only at decode, strict on its own three keys, and —
    /// the security shape — carrying NO actor key (D13). A pin IS a write, so a
    /// pin-only splice is a complete batch and the frozen "missing `edits`"
    /// refusal must not fire on it.
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

        // The actor wall (D13): there is no `pin.actor` key to set, so a caller
        // cannot forge a pin as another actor — the pin's identity is the
        // splice's own daemon-derived `actor` and nothing else.
        let forged = json!({"op": "splice", "path": "plan.md",
            "pin": {"target": "guide.md", "selector": "Q3", "actor": "someone-else"}});
        let e = super::decode::decode(&obj(forged), super::rev::Rev::V3)
            .expect_err("a pin actor is not a field");
        assert_eq!(
            e.message.as_deref(),
            Some("unknown request field `actor` on `pin`")
        );
    }

    /// The v2 wall and the per-key strictness around the pin.
    #[test]
    fn splice_pin_strict_negatives() {
        // v2 never heard of `pin` — the frozen unknown-field refusal, verbatim.
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

    /// Mutual exclusion + the explicit empty-array law (C note 6) + the
    /// strict per-shape wall.
    #[test]
    fn v3_plan_edits_strict_negatives() {
        // both edits and plan_edits → bad_request
        let both = json!({"op": "splice", "path": "a.md",
            "edits": [{"target": {"hpath": ["A"]}, "edit": {"put": {"at": "end", "text": "x"}}}],
            "plan_edits": [{"append": {"hpath": "A", "body": "x"}}]});
        let e = super::decode::decode(&obj(both), super::rev::Rev::V3).expect_err("both refused");
        assert_eq!(
            e.message.as_deref(),
            Some("`edits` and `plan_edits` are mutually exclusive on `splice`")
        );

        // present-but-empty plan_edits → the explicit empty-batch refusal
        let empty = json!({"op": "splice", "path": "a.md", "plan_edits": []});
        let e = super::decode::decode(&obj(empty), super::rev::Rev::V3).expect_err("empty refused");
        assert_eq!(
            e.message.as_deref(),
            Some("`plan_edits` must carry at least one edit")
        );

        // neither edits nor plan_edits → the frozen v2 missing-edits refusal
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
