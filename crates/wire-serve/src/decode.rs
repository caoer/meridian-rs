//! Hand-rolled strict-decode pass (v2 §3.2): unknown fields/enums/mistypes refuse
//! loud. serde ignores unknown fields and `deny_unknown_fields` does not compose
//! with `flatten` — a dropped CAS guard corrupts data. Unarmed ops → `unknown_op`.

use serde_json::{Map, Value};
use wire::{ErrorBody, ErrorCode, HpathSeg, Op, Path, PlanEdit, SecRef};

use crate::bad_request;
use crate::rev::Rev;

/// Envelope keys every request may carry beside the op fields.
const ENVELOPE: [&str; 2] = ["id", "op"];

/// Frozen v2 `splice` field set (wire v2 §4.4) — never grows.
pub(crate) const SPLICE_V2_FIELDS: [&str; 8] = [
    "path", "actor", "now", "receipt", "if_root", "dry", "force", "edits",
];

/// v3 `splice` fields: ONE owner of the set; amendments = V3 \\ V2 (R23 caps).
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

/// Strict-decode one request into [`wire::Op`] by hand (§3.2). Shared by both hosts.
///
/// `rev` gates only `splice`'s field list (`plan_edits` under v3); other ops are
/// rev-agnostic (v3-only ops gate at DISPATCH).
///
/// # Errors
/// `bad_request` / `bad_path` / `unsupported_proto` / `unknown_op` as appropriate.
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
            // Unknown declared rev refuses loud — never silent fallback.
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
            // v2 §4.7: no parameters — root is world-grain.
            check_fields(obj, op, &[])?;
            Ok(Op::Root)
        }
        "links" => {
            // v2 §4.6: both optional; absent path = whole-corpus edges.
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
            // v2 §4.7 push path: reserved shape, live at T5-SUB.
            check_fields(obj, op, &["from_seq"])?;
            Ok(Op::Sub {
                from_seq: req_u64(obj, op, "from_seq")?,
            })
        }
        "read" => decode_read(obj),
        "check_write" => decode_check_write(obj),
        "view_path" => {
            // `cwd` is a raw host absolute path — `req_str`, not `req_path`.
            check_fields(obj, op, &["cwd", "fresh"])?;
            Ok(Op::ViewPath {
                cwd: req_str(obj, op, "cwd")?,
                fresh: opt_bool(obj, op, "fresh")?,
            })
        }
        "splice" => decode_splice(obj, rev),
        "create" => decode_create(obj),
        // §3.2: only genuinely unknown names land here.
        _ => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// Composed `read` (v3-only at DISPATCH; decode is rev-agnostic).
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
    // U14 (decision 14): `sections` and `frag` are STRUCTURED on the wire. They
    // were arrays of joined strings, and a string address on a machine surface
    // is exactly what this docket row removes — the caller states the plane it
    // means (`{"hpath":[…]}` / `{"n":"1.2"}` / `{"anchor":"id"}`) instead of
    // spelling three grammars into one field and letting match order decide.
    // A pre-U14 caller's string arrives here and is REFUSED by name: the door
    // it wants is `wire::ReadSel::parse`, on its own side.
    let sections = match obj.get("sections") {
        None => None,
        Some(v @ Value::Array(_)) => Some(
            serde_json::from_value::<Vec<wire::ReadSel>>(v.clone()).map_err(|_| {
                bad_request(
                    "`sections` must be an array of tagged section selectors on `read` — \
                     `{\"hpath\":[{\"h\":\"Notes\"},{\"h\":\"Q3\"}]}` for a heading path, \
                     `{\"n\":\"1.2\"}` for a dewey ordinal, `{\"anchor\":\"id\"}` for a \
                     block. A joined string is no longer an address on this face (U14): \
                     convert it once at your own ingress door.",
                )
            })?,
        ),
        Some(_) => {
            return Err(bad_request(
                "`sections` must be an array of tagged section selectors on `read`",
            ));
        }
    };
    let frag = match obj.get("frag") {
        None | Some(Value::Null) => None,
        Some(v @ Value::Array(_)) => Some(
            serde_json::from_value::<Vec<HpathSeg>>(v.clone()).map_err(|_| {
                bad_request(
                    "`frag` must be an array of hpath segments on `read` \
                     (`[{\"h\":\"Notes\"},{\"h\":\"Deep\"}]`) — the subtree it scopes is a \
                     tree fact, and a joined string made it a string-prefix question (U14).",
                )
            })?,
        ),
        Some(_) => {
            return Err(bad_request(
                "`frag` must be an array of hpath segments on `read`",
            ));
        }
    };
    Ok(Op::Read {
        path: req_path(obj, op, "path")?,
        mode,
        frag,
        sections,
        display_path: opt_str(obj, op, "display_path")?,
        // §9 read-provenance (D-Actor/B): wire input, never ambient.
        actor: opt_str(obj, op, "actor")?,
    })
}

/// `check_write` (v3-only at DISPATCH). `target` is a raw host path (`req_str`).
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

/// Birth-op fields. No `force` — guarded door has no forced-birth escape.
pub(crate) const CREATE_FIELDS: [&str; 6] = ["path", "body", "actor", "now", "if_root", "dry"];

/// Strict-decode `create`. Rev-agnostic; v3 gate at DISPATCH. `now` is RFC 3339.
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

/// `splice`: only write-existing under v2; v3 also admits `plan_edits`/`pin`.
/// `now` RFC 3339 validated, never generated (§9).
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
    // S7: pin is a write; pin-only batch is complete. Decode before edits gate.
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
            // Frozen v2 refusal, verbatim (C note 6).
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

/// `splice.pin` (S7): `{target, selector, vibe?}`. No `actor` — identity is D13.
fn decode_pin(v: &Value) -> Result<wire::PinSpec, Box<ErrorBody>> {
    let Some(obj) = v.as_object() else {
        return Err(bad_request("`pin` must be an object on `splice`"));
    };
    check_fields(obj, "pin", &["target", "selector", "vibe"])?;
    // U14: the pin selector is TAGGED on the wire (decision 14 — `PinSpec` is a
    // machine surface). A joined string here re-created the `/` delimiter
    // collision one layer up from the lock-ref grammar it was removed from, so
    // the string form is refused by name and pointed at its own door.
    let Some(raw_sel) = obj.get("selector") else {
        return Err(bad_request("missing `selector` on `pin`"));
    };
    let selector: wire::ReadSel = serde_json::from_value(raw_sel.clone()).map_err(|_| {
        bad_request(
            "`pin.selector` must be a tagged section selector object on `splice` \
             (U14, decision 14): `{\"hpath\":[{\"h\":\"Guide\"},{\"h\":\"A/B\"}]}` for a \
             heading path, `{\"anchor\":\"id\"}` for a block, `{\"n\":\"1.2\"}` for a \
             dewey ordinal. A joined string cannot address a heading whose own \
             text contains the `/` it would be split on: convert once at your \
             ingress door, off the read face's published `hpath` array.",
        )
    })?;
    // An empty selector names nothing, in whichever plane it was spelled.
    let empty = match &selector {
        wire::ReadSel::Hpath { hpath } => {
            hpath.is_empty() || hpath.iter().any(|s| s.h.trim().is_empty())
        }
        wire::ReadSel::Dewey { n } => n.trim().is_empty(),
        wire::ReadSel::Anchor { anchor } => anchor.trim().is_empty(),
    };
    if empty {
        return Err(bad_request(
            "`pin.selector` must name a section (a heading path, a `^id`, or a dewey \
             ordinal) — no segment may be blank",
        ));
    }
    Ok(wire::PinSpec {
        target: req_path(obj, "pin", "target")?,
        selector,
        vibe: opt_bool(obj, "pin", "vibe")?,
    })
}

/// `plan_edits`: externally tagged, one tag each, strict field wall per shape.
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
                plan_fields(b, "set_property", &["key", "value", "rev"])?;
                PlanEdit::SetProperty {
                    key: req_str(b, "set_property", "key")?,
                    value: req_str(b, "set_property", "value")?,
                    // U10/P3: the FILE-grain doc-root token. Optional at the
                    // wall (the shape stays decodable without it); the U10
                    // guard is what DEMANDS it on a wire-origin write.
                    rev: opt_str(b, "set_property", "rev")?,
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

/// Closed field set for one plan-edit shape body (no envelope keys).
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

/// §6.1 receipt address: `{path, anchor}` — path law + mint-guard charset.
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

/// §4.4 batch edits: each `{target, edit, if_node_rev?}`; empty array refuses.
fn decode_edits(v: &Value) -> Result<Vec<wire::Edit>, Box<ErrorBody>> {
    let Value::Array(items) = v else {
        return Err(bad_request("`edits` must be an array"));
    };
    if items.is_empty() {
        // Empty batch unrepresentable under §7.1 (one Delta = one root advance).
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

/// Exactly two edit shapes (§4.4): externally tagged `match` / `put`.
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

/// Strict field wall: any key outside the op's set (+ envelope) refuses by name.
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

/// Optional twin of [`req_path`]: absent → `None`, present → path-law.
fn opt_path(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Option<Path>, Box<ErrorBody>> {
    obj.get(key).map(|_| req_path(obj, op, key)).transpose()
}

/// v2 §1 path law: workspace-relative, `/`-separated, never absolute, no `.`/`..`.
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

/// §2.1 mint ref: exactly one of `hpath`/`anchor`/`fm_key`; anchor mint-guarded.
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
    // S10: strip `@fp` before mint-guard (first of two sites; makes decorated
    // addresses agent-plane). Unrecognized `@` shapes still refuse below.
    let id = syntax::split_fp(id).0;
    // mint-guard: one block-id charset, both planes (§2.4)
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

/// One hpath segment: `{"h":…,"n"?}` or v1 bare string; `n` is 1-based `u32`.
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

/// `extract.kinds` vs closed enum: unknown → `bad_request{unknown_kinds}` (D-C5).
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
