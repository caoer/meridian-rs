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

pub(crate) fn decode(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let Some(op) = obj.get("op").and_then(Value::as_str) else {
        return Err(bad_request("`op` must be a string"));
    };
    match op {
        "hello" => {
            check_fields(obj, op, &["proto", "client"])?;
            let proto = req_u64(obj, op, "proto")?;
            let client = opt_str(obj, op, "client")?;
            if proto != u64::from(crate::PROTO) {
                let mut e = ErrorBody::new(ErrorCode::UnsupportedProto);
                e.supported = Some(vec![crate::PROTO]);
                return Err(Box::new(e));
            }
            Ok(Op::Hello {
                proto: crate::PROTO,
                client,
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
        // §3.2 discovery honesty: everything else — including ops the wire
        // vocabulary knows but this rung has not armed (splice/root/diff) —
        // answers `unknown_op`.
        _ => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// The strict-server field wall: any key outside the op's declared set (and
/// the envelope) is rejected loudly, by name.
fn check_fields(obj: &Map<String, Value>, op: &str, allowed: &[&str]) -> Result<(), Box<ErrorBody>> {
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

fn opt_str(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Option<String>, Box<ErrorBody>> {
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

/// v2 §1 path law: workspace-relative, `/`-separated, never absolute, no
/// `.`/`..` segments. Violations are the server's `bad_path`, echoed.
fn req_path(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Path, Box<ErrorBody>> {
    let s = req_str(obj, op, key)?;
    let violates = s.is_empty()
        || s.starts_with('/')
        || s.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..");
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
