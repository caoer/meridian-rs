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

/// v3 `splice` fields: one owner of the set; amendments = V3 \\ V2.
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

/// Which laws one decode pass enforces.
///
/// The strict FIELD wall (§3.2) is unconditional at every grain under both — a
/// door that leaks an unknown field turns a guarded write into an unguarded one.
/// What varies is the §2.4 block-id charset, a judgment on a VALUE inside a
/// shape that is already legal:
///
/// - [`Laws::Full`] — the wire door. Every refusal leaves through one error
///   frame, so there is nothing to distinguish and the decoder judges both.
/// - [`Laws::ShapeOnly`] — the CLI seam (`mrd put` reading the `edits` value off
///   stdin). Here the refusal legs are different exits: `docs/status.md`'s triad
///   makes exit 2 the CLI's OWN refusal of a malformed invocation and exit 1 the
///   ENGINE refusing a well-formed one. A bad block id is the second kind, and
///   the engine judges it anyway on the resolve walk — so the seam leaves that
///   value alone rather than converting an engine refusal into its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Laws {
    /// Shape and value laws both.
    Full,
    /// The §4.4 shape alone; values stay the engine's judgment.
    ShapeOnly,
}

/// Strict-decode one request into [`wire::Op`] by hand (§3.2). Shared by both hosts.
///
/// `rev` gates only `splice`'s field list (`plan_edits` under v3); other ops are
/// rev-agnostic (v3-only ops gate at dispatch).
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
            let sec = obj
                .get("sec")
                .map(|v| decode_sec(v, Laws::Full))
                .transpose()?;
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
            // v2 §4.7 push path.
            check_fields(obj, op, &["from_seq"])?;
            Ok(Op::Sub {
                from_seq: req_u64(obj, op, "from_seq")?,
            })
        }
        "mounts" => {
            // § A.5: no parameters — the table is machine-scoped, and the
            // strict wall makes that a fact a caller can trust.
            check_fields(obj, op, &[])?;
            Ok(Op::Mounts)
        }
        "read" => decode_read(obj),
        "check_write" => decode_check_write(obj),
        "splice" => decode_splice(obj, rev),
        "create" => decode_create(obj),
        "script" => decode_script(obj),
        "run" => decode_run(obj),
        // §3.2: only genuinely unknown names land here.
        _ => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// § A.7 `script` field set — the entry's own inputs and nothing else. No
/// budgets field at birth: the CLI entry exposes none either, and a future
/// override arrives as a dotted `script.<field>` cap, never by loosening this
/// wall. `effects`/`invocation` are the script-effects ruling's two fields
/// (2026-08-13); `token_count_endpoint` is the `token_count` ruling's leg-B
/// field (same day) — the harness measuring endpoint, riding exactly when
/// the `token_count` effect is declared.
pub(crate) const SCRIPT_FIELDS: [&str; 12] = [
    "source",
    "args",
    "files",
    "actor",
    "now",
    "receipt",
    "if_root",
    "dry",
    "expect_armed",
    "effects",
    "invocation",
    "token_count_endpoint",
];

/// The closed effect-builtin set (§ A.7 effects paragraph). `mutex` is
/// recorded DO-NOT-BUILD and deliberately not here.
const KNOWN_EFFECTS: [&str; 2] = ["run", "token_count"];

/// § A.7 in-process script submission (v3-only at dispatch; decode is
/// rev-agnostic, the `read` precedent).
#[allow(clippy::too_many_lines)] // the § A.7 field wall is one sequential decode pass by design
fn decode_script(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "script";
    check_fields(obj, op, &SCRIPT_FIELDS)?;
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    // The inert dict: string keys, string values, no callables, no host reach.
    let args = match obj.get("args") {
        None => std::collections::BTreeMap::new(),
        Some(Value::Object(map)) => {
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in map {
                let Some(v) = v.as_str() else {
                    return Err(bad_request(format!(
                        "`args` values must be strings on `script` (the inert dict, \
                         run-plane § The script entry): `{k}` is not"
                    )));
                };
                out.insert(k.clone(), v.to_owned());
            }
            out
        }
        Some(_) => {
            return Err(bad_request(
                "`args` must be an object of string values on `script`",
            ));
        }
    };
    // Paths only, never content; sorted here so order on the wire is not
    // meaning (the CLI lane sorts client-side — one law, two doors).
    let files = match obj.get("files") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(path) = item.as_str() else {
                    return Err(bad_request(
                        "`files` must be an array of path strings on `script` — paths \
                         only, never content (all content enters through `read()`)",
                    ));
                };
                out.push(path.to_owned());
            }
            out.sort();
            out
        }
        Some(_) => {
            return Err(bad_request(
                "`files` must be an array of path strings on `script`",
            ));
        }
    };
    // Effects mode (§ A.7 effects paragraph): the combination walls live at
    // decode so a malformed submission teaches its wall before any entry.
    let effects = match obj.get("effects") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Err(bad_request(format!(
                    "`effects: []` names no effect builtin — name one ({}) or \
                     omit the field for a pure script",
                    KNOWN_EFFECTS.join(", ")
                )));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(bad_request(
                        "`effects` must be an array of effect-builtin names on `script`",
                    ));
                };
                if !KNOWN_EFFECTS.contains(&name) {
                    return Err(bad_request(format!(
                        "unknown effect `{name}` on `script` — the closed set is: {}",
                        KNOWN_EFFECTS.join(", ")
                    )));
                }
                if out.contains(&name.to_owned()) {
                    return Err(bad_request(format!(
                        "effect `{name}` named twice on `script`"
                    )));
                }
                out.push(name.to_owned());
            }
            out
        }
        Some(_) => {
            return Err(bad_request(
                "`effects` must be an array of effect-builtin names on `script`",
            ));
        }
    };
    let invocation = opt_str(obj, op, "invocation")?;
    let dry = opt_bool(obj, op, "dry")?;
    let if_root = opt_str(obj, op, "if_root")?.map(wire::Root);
    let expect_armed = opt_str(obj, op, "expect_armed")?;
    if !effects.is_empty() {
        if dry == Some(true) {
            return Err(bad_request(
                "`dry` has no meaning in effects mode — a live program cannot \
                 rehearse; `run(dry=True)` inspects one task",
            ));
        }
        if if_root.is_some() {
            return Err(bad_request(
                "`if_fingerprint` has no meaning in effects mode — a live \
                 program reads its own present; there is no premise to guard",
            ));
        }
        if expect_armed.is_some() {
            return Err(bad_request(
                "`expect_armed` has no meaning in effects mode — a live \
                 program holds no armed set",
            ));
        }
        match &invocation {
            None => {
                return Err(bad_request(
                    "effects mode requires `invocation` — run identity derives \
                     host-minted (`<invocation>-r<K>`, §9)",
                ));
            }
            Some(inv) => {
                if inv.is_empty()
                    || !inv
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                {
                    return Err(bad_request(format!(
                        "`invocation` must be a non-empty path-safe token \
                         ([A-Za-z0-9._-]) on `script`: `{inv}`"
                    )));
                }
            }
        }
    } else if invocation.is_some() {
        return Err(bad_request(
            "`invocation` rides effects mode only — a pure script mints no \
             run identity",
        ));
    }
    // The measuring endpoint rides the `token_count` effect EXACTLY: orphan
    // on a pure script, orphan beside other effects, and an explicit empty
    // value are all claims the wall refuses (absent stays absent).
    let token_count_endpoint = opt_str(obj, op, "token_count_endpoint")?;
    if let Some(endpoint) = &token_count_endpoint {
        if !effects.iter().any(|e| e == "token_count") {
            return Err(bad_request(
                "`token_count_endpoint` rides the `token_count` effect only — \
                 declare `effects: [\"token_count\"]` or drop the field",
            ));
        }
        if endpoint.is_empty() {
            return Err(bad_request(
                "`token_count_endpoint` must be a non-empty unix-socket path \
                 on `script` — an explicit empty endpoint binds nothing",
            ));
        }
    }
    Ok(Op::Script {
        source: req_str(obj, op, "source")?,
        args,
        files,
        actor: opt_str(obj, op, "actor")?,
        now,
        receipt: obj.get("receipt").map(decode_receipt).transpose()?,
        if_root,
        dry,
        expect_armed,
        effects,
        invocation,
        token_count_endpoint,
    })
}

/// § A.8 `run` field set — targets plus §9 identity, and nothing else. No
/// receipt / capability / timeout / code field by design: receipts are the
/// plane's own, authority and deadline resolve from the corpus, and the wire
/// carries names, never code.
pub(crate) const RUN_FIELDS: [&str; 4] = ["targets", "invocation", "actor", "now"];

/// The § A.8 fan-out ceiling: every face list carries one.
const RUN_MAX_TARGETS: usize = 64;

/// § A.8 page-task execution (v3-only at dispatch; decode is rev-agnostic).
fn decode_run(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "run";
    check_fields(obj, op, &RUN_FIELDS)?;
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    // The host-minted identity base (§9): per-target ids derive from it and
    // land in receipt anchors and scratch paths, so it must be path-safe.
    let invocation = req_str(obj, op, "invocation")?;
    if invocation.is_empty()
        || !invocation
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(bad_request(format!(
            "`invocation` must be a non-empty path-safe token \
             ([A-Za-z0-9._-]) on `run`: `{invocation}`"
        )));
    }
    let targets = match obj.get("targets") {
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Err(bad_request("`targets` must not be empty on `run`"));
            }
            if items.len() > RUN_MAX_TARGETS {
                return Err(bad_request(format!(
                    "`targets` carries {} entries on `run` — the ceiling is \
                     {RUN_MAX_TARGETS} (§ A.8); split the call",
                    items.len()
                )));
            }
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let Value::Object(t) = item else {
                    return Err(bad_request(format!(
                        "`targets[{i}]` must be an object on `run`"
                    )));
                };
                out.push(decode_run_target(t, i)?);
            }
            out
        }
        Some(_) => {
            return Err(bad_request("`targets` must be an array on `run`"));
        }
        None => return Err(bad_request("missing `targets` on `run`")),
    };
    Ok(Op::Run {
        targets,
        invocation,
        actor: opt_str(obj, op, "actor")?,
        now,
    })
}

/// One § A.8 target: `page` required; `task`/`args`/`env`/`dry` optional.
/// `args` is POSITIONAL here (the run plane's contract shape), unlike the
/// script entry's inert dict — two entries, each speaking its own plane's
/// grammar verbatim.
fn decode_run_target(t: &Map<String, Value>, i: usize) -> Result<wire::RunTarget, Box<ErrorBody>> {
    const TARGET_FIELDS: [&str; 5] = ["page", "task", "args", "env", "dry"];
    for key in t.keys() {
        if !TARGET_FIELDS.contains(&key.as_str()) {
            return Err(bad_request(format!(
                "unknown field `{key}` on `targets[{i}]` of `run`"
            )));
        }
    }
    let Some(Value::String(page)) = t.get("page") else {
        return Err(bad_request(format!(
            "`targets[{i}].page` must be a non-empty string on `run`"
        )));
    };
    if page.is_empty() {
        return Err(bad_request(format!(
            "`targets[{i}].page` must be a non-empty string on `run`"
        )));
    }
    let task = match t.get("task") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].task` must be a string on `run`"
            )));
        }
    };
    let args = match t.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for a in items {
                let Some(s) = a.as_str() else {
                    return Err(bad_request(format!(
                        "`targets[{i}].args` must be an array of strings on `run` — \
                         positional, contract-validated by the plane"
                    )));
                };
                out.push(s.to_owned());
            }
            out
        }
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].args` must be an array of strings on `run`"
            )));
        }
    };
    let env = match t.get("env") {
        None => std::collections::BTreeMap::new(),
        Some(Value::Object(map)) => {
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in map {
                let Some(v) = v.as_str() else {
                    return Err(bad_request(format!(
                        "`targets[{i}].env` values must be strings on `run`: `{k}` is not"
                    )));
                };
                out.insert(k.clone(), v.to_owned());
            }
            out
        }
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].env` must be an object of string values on `run`"
            )));
        }
    };
    let dry = match t.get("dry") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].dry` must be a boolean on `run`"
            )));
        }
    };
    Ok(wire::RunTarget {
        page: page.clone(),
        task,
        args,
        env,
        dry,
    })
}

/// Composed `read` (v3-only at dispatch; decode is rev-agnostic).
fn decode_read(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "read";
    check_fields(
        obj,
        op,
        &["path", "frag", "sections", "display_path", "actor"],
    )?;
    // `sections` and `frag` are structured on the wire: the caller states the
    // plane it means, so match order never decides. A joined-string address is
    // refused by name; its door is `wire::ReadSel::parse`, on the caller's side.
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
        frag,
        sections,
        display_path: opt_str(obj, op, "display_path")?,
        // §9 read-provenance: wire input, never ambient.
        actor: opt_str(obj, op, "actor")?,
    })
}

/// `check_write` (v3-only at dispatch). `target` is a raw host path (`req_str`).
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
            at: req_segs(e, op, "at")?,
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

/// Strict-decode `create`. Rev-agnostic; v3 gate at dispatch. `now` is RFC 3339.
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
    // Pin is a write; a pin-only batch is complete. Decode before the edits gate.
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
            decode_edits(edits_v, Laws::Full)?
        }
        None if plan_edits.is_empty() && pin.is_none() => {
            // Frozen v2 refusal, verbatim.
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

/// `splice.pin`: `{target, selector, vibe?}`. No `actor` — identity is
/// splice's daemon-derived actor only.
fn decode_pin(v: &Value) -> Result<wire::PinSpec, Box<ErrorBody>> {
    let Some(obj) = v.as_object() else {
        return Err(bad_request("`pin` must be an object on `splice`"));
    };
    check_fields(obj, "pin", &["target", "selector", "vibe"])?;
    // The pin selector is tagged on the wire: a joined string re-creates the
    // `/` delimiter collision, so the string form is refused by name.
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
                plan_fields(b, "append", &["hpath", "body", "rev"])?;
                PlanEdit::Append {
                    hpath: req_segs(b, "append", "hpath")?,
                    body: req_str(b, "append", "body")?,
                    rev: opt_str(b, "append", "rev")?,
                }
            }
            "match" => {
                plan_fields(b, "match", &["hpath", "old", "new", "all", "rev"])?;
                PlanEdit::Match {
                    hpath: req_segs(b, "match", "hpath")?,
                    old: req_str(b, "match", "old")?,
                    new: req_str(b, "match", "new")?,
                    all: opt_bool(b, "match", "all")?.unwrap_or(false),
                    rev: opt_str(b, "match", "rev")?,
                }
            }
            "replace_section" => {
                plan_fields(b, "replace_section", &["hpath", "body", "rev"])?;
                PlanEdit::ReplaceSection {
                    hpath: req_segs(b, "replace_section", "hpath")?,
                    body: req_str(b, "replace_section", "body")?,
                    rev: opt_str(b, "replace_section", "rev")?,
                }
            }
            "create" => {
                plan_fields(b, "create", &["parent_hpath", "title", "body", "rev"])?;
                PlanEdit::Create {
                    parent_hpath: req_segs(b, "create", "parent_hpath")?,
                    title: req_str(b, "create", "title")?,
                    body: req_str(b, "create", "body")?,
                    // Parent-section token, optional at the wall (schema-
                    // optional like every guard field); the guard demands it
                    // at occurrence-addressed parents.
                    rev: opt_str(b, "create", "rev")?,
                }
            }
            "set_property" => {
                plan_fields(b, "set_property", &["key", "value", "rev"])?;
                PlanEdit::SetProperty {
                    key: req_str(b, "set_property", "key")?,
                    value: req_str(b, "set_property", "value")?,
                    // File-grain doc-root token: optional at the wall; the guard
                    // demands it on a wire-origin write.
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

/// A plan edit's required address field, as §2.1 segments — the same `{h, n?}`
/// grammar `sec.hpath` takes, decoded through the same [`decode_seg`] door. An
/// empty array decodes; the lowering arms own what it means.
fn req_segs(
    obj: &Map<String, Value>,
    shape: &str,
    field: &str,
) -> Result<Vec<HpathSeg>, Box<ErrorBody>> {
    let Some(Value::Array(items)) = obj.get(field) else {
        return Err(bad_request(format!(
            "`{field}` in `{shape}` must be an array of hpath segments — \
             `[{{\"h\":\"Goals\"}},{{\"h\":\"Q3\"}}]`, the grammar the read face publishes"
        )));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(decode_seg(item)?);
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
///
/// Public because the `edits` value has a SECOND door: `mrd put` reads the bare
/// array off stdin (§4.4 "the CLI seam reads the `edits` VALUE"). One decoder
/// serves both, so the strict wall (§3.2 grain law) cannot hold at one door and
/// leak at the other — a `deny_unknown_fields`-less serde decode there dropped
/// `if_rev` silently and turned a guarded write into an unguarded one.
///
/// `laws` says which grain this door enforces — see [`Laws`]. The field wall is
/// unconditional; only the §2.4 block-id charset rides the flag.
///
/// # Errors
/// `bad_request` naming the offending field/value and the closed set it checked.
pub fn decode_edits(v: &Value, laws: Laws) -> Result<Vec<wire::Edit>, Box<ErrorBody>> {
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
                // The legal set is named because THIS field carries the CAS
                // guard: a caller who typed `if_rev` needs the spelling back,
                // not just the news that the key was wrong (§3.2 grain law).
                // Phrasing follows the config door's refusal exemplar
                // (`config::UNKNOWN_FIELD_REFUSAL_EXEMPLAR`) — one house style
                // for "you typed a field that does not exist".
                return Err(bad_request(format!(
                    "unknown field `{key}` in edit — legal fields are `target`, `edit`, \
                     `if_node_rev`"
                )));
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
            target: decode_sec(target_v, laws)?,
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
            return Err(bad_request(format!(
                "unknown field `{key}` in `edit` — legal fields are `match`, `put`"
            )));
        }
    }
    match (shape.get("match"), shape.get("put")) {
        (Some(Value::Object(m)), None) => {
            for key in m.keys() {
                if !["old", "new"].contains(&key.as_str()) {
                    return Err(bad_request(format!(
                        "unknown field `{key}` in `match` — legal fields are `old`, `new`"
                    )));
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
                    return Err(bad_request(format!(
                        "unknown field `{key}` in `put` — legal fields are `at`, `text`"
                    )));
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
fn decode_sec(v: &Value, laws: Laws) -> Result<SecRef, Box<ErrorBody>> {
    let Value::Object(sec) = v else {
        return Err(bad_request("`sec` must be an object"));
    };
    for key in sec.keys() {
        if !["hpath", "anchor", "fm_key"].contains(&key.as_str()) {
            return Err(bad_request(format!(
                "unknown field `{key}` in the section ref (`sec` on a read, `target` on \
                 an edit) — legal fields are `hpath`, `anchor`, `fm_key`"
            )));
        }
    }
    match (sec.get("hpath"), sec.get("anchor"), sec.get("fm_key")) {
        (Some(h), None, None) => decode_hpath(h),
        (None, Some(a), None) => decode_anchor(a, laws),
        (None, None, Some(k)) => match k {
            Value::String(s) => Ok(SecRef::FmKey { fm_key: s.clone() }),
            _ => Err(bad_request("`fm_key` must be a string")),
        },
        _ => Err(bad_request(
            "`sec` must carry exactly one of `hpath`/`anchor`/`fm_key`",
        )),
    }
}

fn decode_anchor(v: &Value, laws: Laws) -> Result<SecRef, Box<ErrorBody>> {
    let Value::String(id) = v else {
        return Err(bad_request("`anchor` must be a string"));
    };
    // Strip `@fp` before mint-guard; unrecognized `@` shapes still refuse below.
    let id = syntax::split_fp(id).0;
    if laws == Laws::ShapeOnly {
        // The shape is legal and the charset is a VALUE law — the engine's to
        // judge, and it does, on the resolve walk. See [`Laws::ShapeOnly`].
        return Ok(SecRef::Anchor {
            anchor: id.to_owned(),
        });
    }
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

/// One hpath segment: `{"h":…,"n"?}`, the only form; `n` is 1-based `u32`.
/// The v1 bare string is refused (v2 §2.1 as amended, decision 20).
fn decode_seg(v: &Value) -> Result<HpathSeg, Box<ErrorBody>> {
    match v {
        Value::String(h) => Err(bad_request(format!(
            "{refusal}: `{h}`",
            refusal = wire::HPATH_SEG_V1_REFUSAL
        ))),
        Value::Object(seg) => {
            for key in seg.keys() {
                if !["h", "n"].contains(&key.as_str()) {
                    return Err(bad_request(format!(
                        "unknown field `{key}` in hpath segment — legal fields are `h`, `n`"
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
        _ => Err(bad_request("hpath segment must be a `{h, n?}` object")),
    }
}

/// `extract.kinds` vs closed enum: unknown → `bad_request{unknown_kinds}`.
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
