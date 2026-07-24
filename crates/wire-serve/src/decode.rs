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
use wire::{ErrorBody, ErrorCode, HpathSeg, Op, Path, SecRef};

use crate::bad_request;

/// Envelope keys every request may carry beside the op fields.
const ENVELOPE: [&str; 2] = ["id", "op"];

/// Strict-decode one request object into a [`wire::Op`], validating its field
/// set by hand (§3.2 server law). Both hosts call this — the sidecar over stdio
/// and the resident daemon over its socket — so the strict pass is one
/// implementation.
///
/// # Errors
/// A `bad_request` for an unknown field, an unknown enum value, a mistyped
/// value, a malformed path/anchor/`now`, or an unknown declared contract rev; a
/// `bad_path` for a path-law violation; `unsupported_proto` for a `hello` whose
/// `proto` this server does not speak; `unknown_op` for an unrecognized op name.
pub fn decode(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
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
        "splice" => decode_splice(obj),
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

/// v2 §4.4: the only write op, batch-only. §9: `now` is RFC 3339,
/// format-VALIDATED never generated — a malformed `now` is the server's
/// `bad_request` (the pass W4 left to this build-out).
fn decode_splice(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "splice";
    check_fields(
        obj,
        op,
        &[
            "path", "actor", "now", "receipt", "if_root", "dry", "force", "edits",
        ],
    )?;
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    let Some(edits_v) = obj.get("edits") else {
        return Err(bad_request("missing `edits` on `splice`"));
    };
    Ok(Op::Splice {
        path: req_path(obj, op, "path")?,
        actor: opt_str(obj, op, "actor")?,
        now,
        receipt: obj.get("receipt").map(decode_receipt).transpose()?,
        if_root: opt_str(obj, op, "if_root")?.map(wire::Root),
        dry: opt_bool(obj, op, "dry")?,
        force: opt_bool(obj, op, "force")?,
        edits: decode_edits(edits_v)?,
    })
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
    // the mint-guard: one block-id charset, both planes (§2.4)
    match model::Ref::anchor(id.clone()) {
        Ok(_) => Ok(SecRef::Anchor { anchor: id.clone() }),
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
