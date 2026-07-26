//! The hand-rolled strict-decode pass (v2 §3.2 server law, the repo's own
//! recorded obligation — risk R5: fixture-driven because manual).
//!
//! serde's default silently IGNORES unknown request fields, and
//! `deny_unknown_fields` does not compose with `flatten` — a silently dropped
//! CAS guard corrupts data. So every armed op's field set is validated here by
//! hand, loudly: unknown field → `bad_request` naming it; unknown enum value →
//! `bad_request` (with `unknown_kinds` echoed for `extract`, D-C5); mistyped
//! value → `bad_request`. Ops the wire doesn't arm at this rung answer
//! `unknown_op` (§3.2: an op is in `caps` or answers `unknown_op`).

use serde_json::{Map, Value};
use wire::{ErrorBody, ErrorCode, HpathSeg, Op, Path, PlanEdit, SecRef};

use crate::bad_request;
use crate::rev::Rev;

/// Envelope keys every request may carry beside the op fields.
const ENVELOPE: [&str; 2] = ["id", "op"];

/// `splice`'s FROZEN v2 field set (wire v2 §4.4). Never grows — a v2 session's
/// field wall is byte-identical for the life of the contract.
pub(crate) const SPLICE_V2_FIELDS: [&str; 8] = [
    "path", "actor", "now", "receipt", "if_root", "dry", "force", "edits",
];

/// `splice`'s v3 field set: the v2 list plus the v3-era amendments.
///
/// This array is the ONE owner of "which splice fields exist under v3", and the
/// v3-era amendments are exactly `SPLICE_V3_FIELDS \ SPLICE_V2_FIELDS`. R23:
/// each of those amendments MUST be advertised by the v3 caps projection as
/// `splice.<field>` — enforced by the enumeration test
/// [`crate::rev::tests::v3_splice_amendments_are_all_advertised`], which derives
/// its expected set from these two arrays rather than from a hand-copied list.
pub(crate) const SPLICE_V3_FIELDS: [&str; 10] = [
    "path",
    "actor",
    "now",
    "receipt",
    "if_root",
    "dry",
    "force",
    "edits",
    "plan_edits",
    "pin",
];

/// Strict-decode one request object into a [`wire::Op`], validating its field
/// set by hand (§3.2 server law). Both hosts call this — the sidecar over stdio
/// and the resident daemon over its socket — so the strict pass is one
/// implementation.
///
/// `rev` is the session's negotiated contract rev, threaded in by BOTH hosts
/// (M1 U8b rider 1): the ONE rev-dependent decode surface is `splice`'s field
/// list — `plan_edits` decodes under v3 and hits the frozen unknown-field wall
/// under v2 (fixture-pinned). Every other op decodes rev-agnostically (v3-only
/// ops like `read`/`check_write` gate at DISPATCH, unchanged).
///
/// # Errors
/// A `bad_request` for an unknown field, an unknown enum value, a mistyped
/// value, a malformed path/anchor/`now`, or an unknown declared contract rev; a
/// `bad_path` for a path-law violation; `unsupported_proto` for a `hello` whose
/// `proto` this server does not speak; `unknown_op` for an unrecognized op name.
pub fn decode(obj: &Map<String, Value>, rev: Rev) -> Result<Op, Box<ErrorBody>> {
    let Some(op) = obj.get("op").and_then(Value::as_str) else {
        return Err(bad_request("`op` must be a string"));
    };
    match op {
        "hello" => {
            check_fields(obj, op, &["proto", "client", "contract", "workspace"])?;
            let proto = req_u64(obj, op, "proto")?;
            let client = opt_str(obj, op, "client")?;
            let contract = opt_str(obj, op, "contract")?;
            let workspace = opt_str(obj, op, "workspace")?;
            if proto != u64::from(crate::PROTO) {
                let mut e = ErrorBody::new(ErrorCode::UnsupportedProto);
                e.supported = Some(vec![crate::PROTO]);
                return Err(Box::new(e));
            }
            // v3-amendment negotiation: an unknown DECLARED rev is refused LOUD,
            // never a silent fallback (docs/wire-contract-v3-amendment.md).
            if let Some(rev) = &contract
                && !crate::is_known_rev(rev)
            {
                return Err(bad_request(format!(
                    "unknown contract rev `{rev}`: this server speaks v2, v3"
                )));
            }
            Ok(Op::Hello {
                proto: crate::PROTO,
                client,
                contract,
                workspace,
            })
        }
        "toc" => {
            check_fields(obj, op, &["path"])?;
            Ok(Op::Toc {
                path: req_path(obj, op, "path")?,
            })
        }
        "cat" => {
            check_fields(obj, op, &["path", "sec"])?;
            let sec = obj.get("sec").map(decode_sec).transpose()?;
            Ok(Op::Cat {
                path: req_path(obj, op, "path")?,
                sec,
            })
        }
        "extract" => {
            check_fields(obj, op, &["path", "kinds"])?;
            let kinds = obj.get("kinds").map(decode_kinds).transpose()?;
            Ok(Op::Extract {
                path: req_path(obj, op, "path")?,
                kinds,
            })
        }
        "resolve" => {
            check_fields(obj, op, &["from", "ref", "content"])?;
            Ok(Op::Resolve {
                from: req_path(obj, op, "from")?,
                r#ref: req_str(obj, op, "ref")?,
                content: opt_bool(obj, op, "content")?,
            })
        }
        "root" => {
            // v2 §4.7: no parameters — the root is world-grain.
            check_fields(obj, op, &[])?;
            Ok(Op::Root)
        }
        "links" => {
            // v2 §4.6: both fields optional — `path` absent is the
            // whole-corpus edge map; `require_root` is the §10.2 opt-in.
            check_fields(obj, op, &["path", "require_root"])?;
            Ok(Op::Links {
                path: opt_path(obj, op, "path")?,
                require_root: opt_str(obj, op, "require_root")?.map(wire::Root),
            })
        }
        "diff" => {
            check_fields(obj, op, &["from_root", "to_root"])?;
            Ok(Op::Diff {
                from_root: wire::Root(req_str(obj, op, "from_root")?),
                to_root: wire::Root(req_str(obj, op, "to_root")?),
            })
        }
        "sub" => {
            // v2 §4.7 push path: the reserved shape, live at T5-SUB.
            check_fields(obj, op, &["from_seq"])?;
            Ok(Op::Sub {
                from_seq: req_u64(obj, op, "from_seq")?,
            })
        }
        "read" => decode_read(obj),
        "check_write" => decode_check_write(obj),
        "view_path" => {
            // V2 §Q2 the view-organ path forwarder. `cwd` is a RAW host path
            // (absolute) the daemon resolves to a workspace — NOT a
            // workspace-relative wire path, so it takes `req_str`, never
            // `req_path` (path-law would reject a leading `/`). `fresh` is the
            // optional bounded-rebuild knob (§Q3).
            check_fields(obj, op, &["cwd", "fresh"])?;
            Ok(Op::ViewPath {
                cwd: req_str(obj, op, "cwd")?,
                fresh: opt_bool(obj, op, "fresh")?,
            })
        }
        "splice" => decode_splice(obj, rev),
        "create" => decode_create(obj),
        // §3.2 discovery honesty: every op is armed as of T5-SUB — only
        // genuinely unknown names land here.
        _ => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// M1 U4a2 the composed read op (v3-only at DISPATCH — decode is
/// rev-agnostic; a v2 session's dispatch answers `unknown_op`, §3.2
/// discovery honesty against the frozen v2 caps).
fn decode_read(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "read";
    check_fields(
        obj,
        op,
        &["path", "mode", "frag", "sections", "display_path", "actor"],
    )?;
    let mode = opt_str(obj, op, "mode")?;
    if let Some(m) = &mode
        && m != "toc"
        && m != "sections"
    {
        return Err(bad_request(format!(
            "`mode` must be `toc` or `sections` on `read`: `{m}`"
        )));
    }
    let sections = match obj.get("sections") {
        None => None,
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(s) = item.as_str() else {
                    return Err(bad_request(
                        "`sections` must be an array of strings on `read`",
                    ));
                };
                out.push(s.to_owned());
            }
            Some(out)
        }
        Some(_) => {
            return Err(bad_request(
                "`sections` must be an array of strings on `read`",
            ));
        }
    };
    Ok(Op::Read {
        path: req_path(obj, op, "path")?,
        mode,
        frag: opt_str(obj, op, "frag")?,
        sections,
        display_path: opt_str(obj, op, "display_path")?,
        // §9 read-provenance slot (D-Actor/B): opaque string, same law as
        // splice's actor — a wire input, never ambient.
        actor: opt_str(obj, op, "actor")?,
    })
}

/// M1 U8c the I4 def-conformance verdict op (v3-only at DISPATCH, like
/// `read`). `target` is a RAW host path string (the caller's absolute
/// spelling — labels refusal strings + anchors def-layer discovery), so it
/// takes `req_str`, never `req_path`. `edits` is the put-plan vocabulary.
fn decode_check_write(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "check_write";
    check_fields(obj, op, &["path", "target", "actor", "now", "edits"])?;
    let Some(Value::Array(items)) = obj.get("edits") else {
        return Err(bad_request(
            "`edits` must be an array of edit objects on `check_write`",
        ));
    };
    let mut edits = Vec::with_capacity(items.len());
    for item in items {
        let Some(e) = item.as_object() else {
            return Err(bad_request(
                "`edits` must be an array of edit objects on `check_write`",
            ));
        };
        check_fields(e, op, &["op", "at", "find", "body", "rev", "all"])?;
        edits.push(wire::CheckWriteEdit {
            op: req_str(e, op, "op")?,
            at: req_str(e, op, "at")?,
            find: opt_str(e, op, "find")?.unwrap_or_default(),
            body: opt_str(e, op, "body")?.unwrap_or_default(),
            rev: opt_str(e, op, "rev")?.unwrap_or_default(),
            all: opt_bool(e, op, "all")?.unwrap_or(false),
        });
    }
    Ok(Op::CheckWrite {
        path: req_path(obj, op, "path")?,
        target: req_str(obj, op, "target")?,
        actor: req_str(obj, op, "actor")?,
        now: req_str(obj, op, "now")?,
        edits,
    })
}

/// The BIRTH op's field set. A v3-era op, so unlike `splice` there is no
/// frozen v2 twin: the whole op ships as ONE cap (`create`), exactly like
/// `read` and `check_write`, and no dotted `create.<field>` cap exists. This
/// array is the one owner of "which create fields exist"; the negative
/// [`crate::decode::tests`] rows derive their expectations from it.
///
/// `force` is deliberately ABSENT: the guarded door has no forced-birth escape
/// (`write::create`), so admitting the key would advertise a bypass that does
/// not exist.
pub(crate) const CREATE_FIELDS: [&str; 6] = ["path", "body", "actor", "now", "if_root", "dry"];

/// Strict-decode the birth op. Rev-agnostic here (like `read`/`check_write`) —
/// the v3 gate is at DISPATCH, so a v2 session answers `unknown_op` against the
/// frozen v2 caps (§3.2 discovery honesty).
///
/// `now` takes the SAME RFC 3339 validation `splice` applies, and for the same
/// reason: this op writes a journal row stamped with the caller's clock (§9 —
/// the engine validates time, never generates it), so a malformed `now` must
/// refuse at the wire rather than land an undatable row.
fn decode_create(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "create";
    check_fields(obj, op, &CREATE_FIELDS)?;
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    Ok(Op::Create {
        path: req_path(obj, op, "path")?,
        body: req_str(obj, op, "body")?,
        actor: opt_str(obj, op, "actor")?,
        now,
        if_root: opt_str(obj, op, "if_root")?.map(wire::Root),
        dry: opt_bool(obj, op, "dry")?,
    })
}

/// v2 §4.4: the only write op UNDER V2, batch-only (v3 adds `create`, the birth
/// door; `splice` stays the only op that EDITS an existing file). §9: `now` is
/// RFC 3339,
/// format-VALIDATED never generated — a malformed `now` is the server's
/// `bad_request` (the pass W4 left to this build-out).
///
/// M1 U8b (rider 1): under a v3 session the field list additionally admits
/// `plan_edits` — the plan-level batch, mutually exclusive with `edits`. Under
/// v2 the list is FROZEN, so a v2 `plan_edits` refuses on the existing
/// unknown-field wall byte-for-byte (fixture-pinned negative).
fn decode_splice(obj: &Map<String, Value>, rev: Rev) -> Result<Op, Box<ErrorBody>> {
    let op = "splice";
    if rev == Rev::V3 {
        check_fields(obj, op, &SPLICE_V3_FIELDS)?;
    } else {
        check_fields(obj, op, &SPLICE_V2_FIELDS)?;
    }
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    let plan_edits = match obj.get("plan_edits") {
        None => Vec::new(),
        Some(v) => decode_plan_edits(v)?,
    };
    // S7: a `pin` is itself a write, so a pin-only splice is a complete batch.
    // Decoded BEFORE the edits gate because that gate reads its presence.
    let pin = match obj.get("pin") {
        None => None,
        Some(v) => Some(decode_pin(v)?),
    };
    let edits = match obj.get("edits") {
        Some(edits_v) => {
            if !plan_edits.is_empty() {
                return Err(bad_request(
                    "`edits` and `plan_edits` are mutually exclusive on `splice`",
                ));
            }
            decode_edits(edits_v)?
        }
        None if plan_edits.is_empty() && pin.is_none() => {
            // The frozen v2 refusal, verbatim — a plan-less, edit-less splice
            // reads exactly as before (C note 6: never a serde accident).
            return Err(bad_request("missing `edits` on `splice`"));
        }
        None => Vec::new(),
    };
    Ok(Op::Splice {
        path: req_path(obj, op, "path")?,
        actor: opt_str(obj, op, "actor")?,
        now,
        receipt: obj.get("receipt").map(decode_receipt).transpose()?,
        if_root: opt_str(obj, op, "if_root")?.map(wire::Root),
        dry: opt_bool(obj, op, "dry")?,
        force: opt_bool(obj, op, "force")?,
        edits,
        plan_edits,
        pin,
    })
}

/// Strict-decode `splice.pin` (S7): `{target, selector, vibe?}` and nothing
/// else. There is deliberately no `actor` key — a pin's mint identity IS the
/// splice's own daemon-derived actor (D13), so admitting one here would let a
/// caller forge a pin as another actor.
fn decode_pin(v: &Value) -> Result<wire::PinSpec, Box<ErrorBody>> {
    let Some(obj) = v.as_object() else {
        return Err(bad_request("`pin` must be an object on `splice`"));
    };
    check_fields(obj, "pin", &["target", "selector", "vibe"])?;
    let selector = req_str(obj, "pin", "selector")?;
    if selector.trim().is_empty() {
        return Err(bad_request(
            "`pin.selector` must name a section (a sanitized heading path or `^id`)",
        ));
    }
    Ok(wire::PinSpec {
        target: req_path(obj, "pin", "target")?,
        selector,
        vibe: opt_bool(obj, "pin", "vibe")?,
    })
}

/// M1 U8b `plan_edits` (v3-only; the caller gated on rev before calling):
/// externally tagged items, exactly one tag each, every shape's field set
/// validated by hand — the same strict wall as the native edit union. An
/// empty array refuses like the native empty batch (a batch IS its edits).
fn decode_plan_edits(v: &Value) -> Result<Vec<PlanEdit>, Box<ErrorBody>> {
    let Value::Array(items) = v else {
        return Err(bad_request("`plan_edits` must be an array"));
    };
    if items.is_empty() {
        return Err(bad_request("`plan_edits` must carry at least one edit"));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(e) = item else {
            return Err(bad_request("each plan edit must be an object"));
        };
        if e.len() != 1 {
            return Err(bad_request(
                "a plan edit must carry exactly one of `append`/`match`/`replace_section`/`create`/`set_property`",
            ));
        }
        let (tag, body_v) = e.iter().next().expect("len checked");
        let Value::Object(b) = body_v else {
            return Err(bad_request(format!("`{tag}` must be an object")));
        };
        out.push(match tag.as_str() {
            "append" => {
                plan_fields(b, "append", &["hpath", "body"])?;
                PlanEdit::Append {
                    hpath: req_str(b, "append", "hpath")?,
                    body: req_str(b, "append", "body")?,
                }
            }
            "match" => {
                plan_fields(b, "match", &["hpath", "old", "new", "all", "rev"])?;
                PlanEdit::Match {
                    hpath: req_str(b, "match", "hpath")?,
                    old: req_str(b, "match", "old")?,
                    new: req_str(b, "match", "new")?,
                    all: opt_bool(b, "match", "all")?.unwrap_or(false),
                    rev: opt_str(b, "match", "rev")?,
                }
            }
            "replace_section" => {
                plan_fields(b, "replace_section", &["hpath", "body", "rev"])?;
                PlanEdit::ReplaceSection {
                    hpath: req_str(b, "replace_section", "hpath")?,
                    body: req_str(b, "replace_section", "body")?,
                    rev: opt_str(b, "replace_section", "rev")?,
                }
            }
            "create" => {
                plan_fields(b, "create", &["parent_hpath", "title", "body"])?;
                PlanEdit::Create {
                    parent_hpath: req_str(b, "create", "parent_hpath")?,
                    title: req_str(b, "create", "title")?,
                    body: req_str(b, "create", "body")?,
                }
            }
            "set_property" => {
                plan_fields(b, "set_property", &["key", "value"])?;
                PlanEdit::SetProperty {
                    key: req_str(b, "set_property", "key")?,
                    value: req_str(b, "set_property", "value")?,
                }
            }
            other => {
                return Err(bad_request(format!(
                    "unknown plan edit shape `{other}` — one of append/match/replace_section/create/set_property"
                )));
            }
        });
    }
    Ok(out)
}

/// The strict field wall for one plan-edit shape body (no envelope keys ride
/// inside a shape, so this is a plain closed-set check).
fn plan_fields(
    obj: &Map<String, Value>,
    shape: &str,
    allowed: &[&str],
) -> Result<(), Box<ErrorBody>> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(bad_request(format!("unknown field `{key}` in `{shape}`")));
        }
    }
    Ok(())
}

/// §6.1 receipt address: `{path, anchor}` exactly — path law on `path`, the
/// mint-guard charset on `anchor` (a receipt anchor is a mint position).
fn decode_receipt(v: &Value) -> Result<wire::ReceiptAddr, Box<ErrorBody>> {
    let Value::Object(r) = v else {
        return Err(bad_request("`receipt` must be an object"));
    };
    for key in r.keys() {
        if !["path", "anchor"].contains(&key.as_str()) {
            return Err(bad_request(format!("unknown field `{key}` in `receipt`")));
        }
    }
    let path = req_path(r, "receipt", "path")?;
    let anchor = match r.get("anchor") {
        Some(Value::String(id)) => match model::Ref::anchor(id.clone()) {
            Ok(_) => id.clone(),
            Err(bad) => {
                return Err(bad_request(format!(
                    "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
                    id = bad.id
                )));
            }
        },
        Some(_) => return Err(bad_request("`anchor` must be a string")),
        None => return Err(bad_request("missing `anchor` on `receipt`")),
    };
    Ok(wire::ReceiptAddr { path, anchor })
}

/// §4.4 batch edits: each `{target, edit, if_node_rev?}`, the target in THE
/// §2.1 grammar (mint-guarded), the edit exactly one of the two shapes.
fn decode_edits(v: &Value) -> Result<Vec<wire::Edit>, Box<ErrorBody>> {
    let Value::Array(items) = v else {
        return Err(bad_request("`edits` must be an array"));
    };
    if items.is_empty() {
        // Derived reading (no frozen empty-batch form exists): a batch IS
        // its edits, and an edit-less commit would mint a Delta with no
        // root advance — unrepresentable under §7.1 "one Delta = one batch
        // = one root advance". Refused loud, recorded as derived-data.
        return Err(bad_request("`edits` must carry at least one edit"));
    }
    let mut edits = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(e) = item else {
            return Err(bad_request("each edit must be an object"));
        };
        for key in e.keys() {
            if !["target", "edit", "if_node_rev"].contains(&key.as_str()) {
                return Err(bad_request(format!("unknown field `{key}` in edit")));
            }
        }
        let Some(target_v) = e.get("target") else {
            return Err(bad_request("missing `target` in edit"));
        };
        let Some(shape_v) = e.get("edit") else {
            return Err(bad_request("missing `edit` in edit"));
        };
        let if_node_rev = match e.get("if_node_rev") {
            None => None,
            Some(Value::String(s)) => Some(wire::NodeRev(s.clone())),
            Some(_) => return Err(bad_request("`if_node_rev` must be a string")),
        };
        edits.push(wire::Edit {
            target: decode_sec(target_v)?,
            edit: decode_edit_shape(shape_v)?,
            if_node_rev,
        });
    }
    Ok(edits)
}

/// Exactly two edit shapes (§4.4): externally tagged `{"match":{…}}` /
/// `{"put":{…}}`, exactly one tag, each shape's fields validated by hand.
fn decode_edit_shape(v: &Value) -> Result<wire::EditShape, Box<ErrorBody>> {
    let Value::Object(shape) = v else {
        return Err(bad_request("`edit` must be an object"));
    };
    for key in shape.keys() {
        if !["match", "put"].contains(&key.as_str()) {
            return Err(bad_request(format!("unknown field `{key}` in `edit`")));
        }
    }
    match (shape.get("match"), shape.get("put")) {
        (Some(Value::Object(m)), None) => {
            for key in m.keys() {
                if !["old", "new"].contains(&key.as_str()) {
                    return Err(bad_request(format!("unknown field `{key}` in `match`")));
                }
            }
            Ok(wire::EditShape::Match {
                old: req_str(m, "match", "old")?,
                new: req_str(m, "match", "new")?,
            })
        }
        (None, Some(Value::Object(p))) => {
            for key in p.keys() {
                if !["at", "text"].contains(&key.as_str()) {
                    return Err(bad_request(format!("unknown field `{key}` in `put`")));
                }
            }
            let at = match p.get("at") {
                Some(Value::String(s)) => match s.as_str() {
                    "all" => wire::PutAt::All,
                    "content" => wire::PutAt::Content,
                    "end" => wire::PutAt::End,
                    "upsert" => wire::PutAt::Upsert,
                    other => {
                        return Err(bad_request(format!(
                            "`at` must be one of all/content/end/upsert: `{other}`"
                        )));
                    }
                },
                _ => return Err(bad_request("`at` must be a string")),
            };
            Ok(wire::EditShape::Put {
                at,
                text: req_str(p, "put", "text")?,
            })
        }
        _ => Err(bad_request(
            "`edit` must carry exactly one of `match`/`put` as an object",
        )),
    }
}

/// The strict-server field wall: any key outside the op's declared set (and
/// the envelope) is rejected loudly, by name.
fn check_fields(
    obj: &Map<String, Value>,
    op: &str,
    allowed: &[&str],
) -> Result<(), Box<ErrorBody>> {
    for key in obj.keys() {
        if !ENVELOPE.contains(&key.as_str()) && !allowed.contains(&key.as_str()) {
            return Err(bad_request(format!(
                "unknown request field `{key}` on `{op}`"
            )));
        }
    }
    Ok(())
}

fn req_str(obj: &Map<String, Value>, op: &str, key: &str) -> Result<String, Box<ErrorBody>> {
    match obj.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(bad_request(format!("`{key}` on `{op}` must be a string"))),
        None => Err(bad_request(format!("missing `{key}` on `{op}`"))),
    }
}

fn opt_str(
    obj: &Map<String, Value>,
    op: &str,
    key: &str,
) -> Result<Option<String>, Box<ErrorBody>> {
    obj.get(key)
        .map(|v| match v {
            Value::String(s) => Ok(s.clone()),
            _ => Err(bad_request(format!("`{key}` on `{op}` must be a string"))),
        })
        .transpose()
}

fn opt_bool(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Option<bool>, Box<ErrorBody>> {
    obj.get(key)
        .map(|v| match v {
            Value::Bool(b) => Ok(*b),
            _ => Err(bad_request(format!("`{key}` on `{op}` must be a boolean"))),
        })
        .transpose()
}

fn req_u64(obj: &Map<String, Value>, op: &str, key: &str) -> Result<u64, Box<ErrorBody>> {
    match obj.get(key) {
        Some(Value::Number(n)) if n.as_u64().is_some() => Ok(n.as_u64().unwrap_or_default()),
        Some(_) => Err(bad_request(format!(
            "`{key}` on `{op}` must be a non-negative integer"
        ))),
        None => Err(bad_request(format!("missing `{key}` on `{op}`"))),
    }
}

/// [`req_path`]'s optional twin: absent is `None`, present is path-law
/// validated.
fn opt_path(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Option<Path>, Box<ErrorBody>> {
    obj.get(key).map(|_| req_path(obj, op, key)).transpose()
}

/// v2 §1 path law: workspace-relative, `/`-separated, never absolute, no
/// `.`/`..` segments. Violations are the server's `bad_path`, echoed.
fn req_path(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Path, Box<ErrorBody>> {
    let s = req_str(obj, op, key)?;
    let violates = s.is_empty()
        || s.starts_with('/')
        || s.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if violates {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(Path(s));
        return Err(Box::new(e));
    }
    Ok(Path(s))
}

/// A §2.1 mint ref, strictly: exactly one of `hpath`/`anchor`/`fm_key`, no
/// other key, each form's shape validated by hand. The anchor form passes the
/// mint-guard (`model::Ref::anchor`, decision 011): a block id outside
/// `[A-Za-z0-9-]+` — e.g. `_`-bearing — is `bad_request` at this plane.
fn decode_sec(v: &Value) -> Result<SecRef, Box<ErrorBody>> {
    let Value::Object(sec) = v else {
        return Err(bad_request("`sec` must be an object"));
    };
    for key in sec.keys() {
        if !["hpath", "anchor", "fm_key"].contains(&key.as_str()) {
            return Err(bad_request(format!("unknown field `{key}` in `sec`")));
        }
    }
    match (sec.get("hpath"), sec.get("anchor"), sec.get("fm_key")) {
        (Some(h), None, None) => decode_hpath(h),
        (None, Some(a), None) => decode_anchor(a),
        (None, None, Some(k)) => match k {
            Value::String(s) => Ok(SecRef::FmKey { fm_key: s.clone() }),
            _ => Err(bad_request("`fm_key` must be a string")),
        },
        _ => Err(bad_request(
            "`sec` must carry exactly one of `hpath`/`anchor`/`fm_key`",
        )),
    }
}

fn decode_anchor(v: &Value) -> Result<SecRef, Box<ErrorBody>> {
    let Value::String(id) = v else {
        return Err(bad_request("`anchor` must be a string"));
    };
    // Stage-2 S10: the `@fp` strip is ordered BEFORE the mint-guard here, and
    // this is the FIRST of the two guard sites a wire address meets (the other
    // is `read::to_model_ref`, the belt for in-process callers that build a
    // `SecRef` directly). Decoding to the STORED spelling is what makes the
    // decorated address agent-plane rather than display-only: an agent that
    // read `[[guide#^goal@green.b3af12cd]]` can address `^goal@green.b3af12cd`
    // and reach exactly the node `^goal` names.
    //
    // Additive on the frozen v2 plane by construction: the only inputs whose
    // outcome moves are shaped tokens, which every pre-S10 build refused and no
    // v2 client can produce (the grammar did not exist). An `@` the shape does
    // NOT recognize still refuses, verbatim, below.
    let id = syntax::split_fp(id).0;
    // the mint-guard: one block-id charset, both planes (§2.4)
    match model::Ref::anchor(id.to_owned()) {
        Ok(_) => Ok(SecRef::Anchor {
            anchor: id.to_owned(),
        }),
        Err(bad) => Err(bad_request(format!(
            "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
            id = bad.id
        ))),
    }
}

fn decode_hpath(v: &Value) -> Result<SecRef, Box<ErrorBody>> {
    let Value::Array(items) = v else {
        return Err(bad_request("`hpath` must be an array of segments"));
    };
    let mut hpath = Vec::with_capacity(items.len());
    for item in items {
        hpath.push(decode_seg(item)?);
    }
    Ok(SecRef::Hpath { hpath })
}

/// One hpath segment: the object form `{"h":…}`/`{"h":…,"n":…}` or the v1
/// bare string (dual-deserialization bridge, §2.1). `n` is a 1-based `u32`.
fn decode_seg(v: &Value) -> Result<HpathSeg, Box<ErrorBody>> {
    match v {
        Value::String(h) => Ok(HpathSeg {
            h: h.clone(),
            n: None,
        }),
        Value::Object(seg) => {
            for key in seg.keys() {
                if !["h", "n"].contains(&key.as_str()) {
                    return Err(bad_request(format!(
                        "unknown field `{key}` in hpath segment"
                    )));
                }
            }
            let Some(Value::String(h)) = seg.get("h") else {
                return Err(bad_request("hpath segment `h` must be a string"));
            };
            let n = match seg.get("n") {
                None => None,
                Some(Value::Number(n)) => match n.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    Some(n) if n >= 1 => Some(n),
                    _ => return Err(bad_request("hpath segment `n` must be a 1-based integer")),
                },
                Some(_) => return Err(bad_request("hpath segment `n` must be a 1-based integer")),
            };
            Ok(HpathSeg { h: h.clone(), n })
        }
        _ => Err(bad_request(
            "hpath segment must be a string or `{h, n?}` object",
        )),
    }
}

/// `extract.kinds` values against the closed 11-kind enum: any unknown value
/// is `bad_request{unknown_kinds}`, loud (D-C5 — the typo-silently-returns-
/// nothing trap, killed). The valid names are the wire enum's own serde
/// spellings — never a duplicated list.
fn decode_kinds(v: &Value) -> Result<Vec<String>, Box<ErrorBody>> {
    let Value::Array(items) = v else {
        return Err(bad_request("`kinds` must be an array of strings"));
    };
    let mut kinds = Vec::with_capacity(items.len());
    let mut unknown = Vec::new();
    for item in items {
        let Value::String(s) = item else {
            return Err(bad_request("`kinds` must be an array of strings"));
        };
        if serde_json::from_value::<wire::NodeKind>(item.clone()).is_ok() {
            kinds.push(s.clone());
        } else {
            unknown.push(s.clone());
        }
    }
    if unknown.is_empty() {
        Ok(kinds)
    } else {
        let mut e = ErrorBody::new(ErrorCode::BadRequest);
        e.unknown_kinds = Some(unknown);
        Err(Box::new(e))
    }
}
