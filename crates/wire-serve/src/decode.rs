//! Hand-rolled strict-decode pass (v2 §3.2): unknown fields/enums/mistypes refuse
//! loud. serde ignores unknown fields and `deny_unknown_fields` does not compose
//! with `flatten` — a dropped CAS guard corrupts data. Unarmed ops → `unknown_op`.

use serde_json::{Map, Value};
use wire::{ErrorBody, ErrorCode, GuardEntry, HpathSeg, Op, Path, PlanEdit, SecRef};

use crate::bad_request;
use crate::rev::Rev;

/// Envelope keys every request may carry beside the op fields.
const ENVELOPE: [&str; 2] = ["id", "op"];

/// Frozen v2 `splice` field set (wire v2 §4.4) — never grows.
pub(crate) const SPLICE_V2_FIELDS: [&str; 8] = [
    "path", "actor", "now", "receipt", "if_root", "dry", "force", "edits",
];

/// v3 `splice` fields: one owner of the set; amendments = V3 \\ V2.
/// `scope` + `guards` ride the `scoped-guards` family cap (§5.4), not
/// dotted `splice.scope` / `splice.guards` (one family, one flag).
pub(crate) const SPLICE_V3_FIELDS: [&str; 14] = [
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
    "files",
    "scope",
    "guards",
    // § A.2.1 middleware passthrough (cap `splice.fields`) — single form
    // only; the set form's walls never carry it (no middleware there in V1).
    "fields",
];

/// Guard-family fields that refuse un-negotiated on a frozen v2 session
/// (§3.2 / §5.4 / §8.2). `scope_bytes` is top-level on the mint door only;
/// on splice/script it rides a `guards[]` entry.
const FAMILY_FIELDS: [&str; 3] = ["scope", "guards", "scope_bytes"];

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

/// Strict-decode one request into [`wire::Op`] by hand (§3.2). Shared by every serve door.
///
/// `rev` gates only `splice`'s field list (`plan_edits` under v3); other ops are
/// rev-agnostic (v3-only ops gate at dispatch).
///
/// # Errors
/// `bad_request` / `bad_path` / `unsupported_proto` / `unknown_op` as appropriate.
#[expect(
    clippy::too_many_lines,
    reason = "exhaustive op decode table — one arm per wire op; splitting adds indirection, not insight"
)]
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
            // Bare = world mint (v2, no parameters). Under v3 the mint arm
            // admits `scope` / `scope_bytes` (§4.7). Un-negotiated use on a
            // frozen v2 session refuses with the family teaching, not
            // "unknown field" — the field is known law, not a typo.
            refuse_unnegotiated_family(obj, rev)?;
            if rev == Rev::V3 {
                check_fields(obj, op, &["scope", "scope_bytes"])?;
            } else {
                check_fields(obj, op, &[])?;
            }
            let scope = opt_path(obj, op, "scope")?;
            let scope_bytes = opt_str(obj, op, "scope_bytes")?;
            if scope.is_some() && scope_bytes.is_some() {
                return Err(bad_request(wire::mint_pair_teaching()));
            }
            Ok(Op::Root { scope, scope_bytes })
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
            // §4.7 push path, B-01 cursor grammar: both fields optional at
            // decode — which pairs are lawful (both, neither) is the serve's
            // anchor evaluation, not field shape.
            check_fields(obj, op, &["tree_instance", "from_seq"])?;
            Ok(Op::Sub {
                tree_instance: opt_str(obj, op, "tree_instance")?,
                from_seq: opt_u64(obj, op, "from_seq")?,
            })
        }
        "mounts" => {
            // § A.5: no parameters — the table is machine-scoped, and the
            // strict wall makes that a fact a caller can trust.
            check_fields(obj, op, &[])?;
            Ok(Op::Mounts)
        }
        "walk" => decode_walk(obj),
        "sql" => {
            // § A.11: the statement and nothing else — profile, cwd, and row
            // bounds are host concerns, never wire fields.
            check_fields(obj, "sql", &["query"])?;
            Ok(Op::Sql {
                query: req_str(obj, "sql", "query")?,
            })
        }
        "read" => decode_read(obj),
        "check_write" => decode_check_write(obj),
        "splice" => decode_splice(obj, rev),
        "create" => decode_create(obj),
        "remove" => decode_remove(obj),
        "script" => decode_script(obj),
        "run" => decode_run(obj, rev),
        // §3.2: only genuinely unknown names land here.
        _ => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// § A.10 `walk`: the page, the direction toggle, the depth bound.
fn decode_walk(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "walk";
    check_fields(obj, op, &["path", "down", "depth"])?;
    let depth = obj
        .get("depth")
        .map(|_| req_u64(obj, op, "depth"))
        .transpose()?
        .map(|d| {
            u32::try_from(d).map_err(|_| {
                bad_request(format!(
                    "`depth` on `{op}` exceeds the supported bound: {d}"
                ))
            })
        })
        .transpose()?;
    Ok(Op::Walk {
        path: req_path(obj, op, "path")?,
        down: opt_bool(obj, op, "down")?,
        depth,
    })
}

/// § A.7 `script` field set — the entry's own inputs and nothing else. No
/// budgets field at birth: the CLI entry exposes none either, and a future
/// override arrives as a dotted `script.<field>` cap, never by loosening this
/// wall. `effects`/`invocation` are the script-effects contract's two fields;
/// `token_count_endpoint` is the `token_count` effect's leg-B field — the
/// harness measuring endpoint, riding exactly when the `token_count` effect
/// is declared.
pub(crate) const SCRIPT_FIELDS: [&str; 14] = [
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
    "scope",
    "guards",
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
    // Paths only, never content; call order preserved — `files[i]` is the
    // i-th path the caller typed, because the program indexes the list and a
    // host-substituted order lands edits on the wrong document silently
    // (order-bind ruling; the CLI lane preserves order the same way — one
    // law, two doors).
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
    let scope = opt_path(obj, op, "scope")?;
    let guards = decode_guards(obj.get("guards"))?;
    if scope.is_some() && if_root.is_none() {
        return Err(bad_request(wire::broken_premise_pair_teaching(
            "scope without if_fingerprint",
        )));
    }
    if !effects.is_empty() {
        if dry == Some(true) {
            return Err(bad_request(
                "`dry` has no meaning in effects mode — a live program cannot \
                 rehearse; `run(dry=True)` inspects one task",
            ));
        }
        if if_root.is_some() {
            return Err(bad_request(wire::effects_door_teaching("if_fingerprint")));
        }
        if scope.is_some() {
            return Err(bad_request(wire::effects_door_teaching("scope")));
        }
        if !guards.is_empty() {
            return Err(bad_request(wire::effects_door_teaching("guards")));
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
        scope,
        guards,
    })
}

/// § A.8 `run` field set — targets plus §9 identity, and nothing else. No
/// receipt / capability / timeout / code field by design: receipts are the
/// plane's own, authority and deadline resolve from the corpus, and the wire
/// carries names, never code.
///
/// *Amended 2026-08-23 (hook-support design § 2.2): `prelude` joins the set
/// behind cap `run.mode`.* It is load-phase SOURCE, one per call, shared by
/// every mode-bearing target — the `script` op already takes caller `source`,
/// so this is the shipped precedent, not a new class of field. The set stays
/// CLOSED: that is what refuses an unnegotiated field by name instead of
/// silently ignoring it.
pub(crate) const RUN_FIELDS: [&str; 7] = [
    "targets",
    "invocation",
    "actor",
    "now",
    "fields",
    "ambient",
    "prelude",
];

/// The `run` op's fields as SHIPPED, before the hook-support amendment — the
/// set a non-v3 session is judged against, so `prelude` refuses by name there
/// instead of being accepted by a server the client never negotiated with.
pub(crate) const SHIPPED_RUN_FIELDS: [&str; 6] =
    ["targets", "invocation", "actor", "now", "fields", "ambient"];

/// The § A.8 fan-out ceiling: every face list carries one.
const RUN_MAX_TARGETS: usize = 64;

/// § A.8 page-task execution (v3-only at dispatch; decode is rev-agnostic).
fn decode_run(obj: &Map<String, Value>, rev: Rev) -> Result<Op, Box<ErrorBody>> {
    let op = "run";
    // The amendment's additions leave the closed set on a NON-v3 session, so
    // an unnegotiated field meets the by-name wall HERE — before the op-grain
    // v3 gate answers `unknown_op` for the whole request. Same shape as the
    // root op's mint arm (`refuse_unnegotiated_family` + a rev-conditional
    // `check_fields`), which is the shipped precedent.
    //
    // Without this the published mechanism was unconstructible: `prelude` and
    // the six target additions sat in the unconditional sets, so a v2 client
    // could never see the refusal the docs promise and a v3 client always has
    // the caps. The acceptance gate tests for those exact bytes.
    if rev == Rev::V3 {
        check_fields(obj, op, &RUN_FIELDS)?;
    } else {
        check_fields(obj, op, &SHIPPED_RUN_FIELDS)?;
    }
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
                out.push(decode_run_target(t, i, rev)?);
            }
            out
        }
        Some(_) => {
            return Err(bad_request("`targets` must be an array on `run`"));
        }
        None => return Err(bad_request("missing `targets` on `run`")),
    };
    // The caller's ambient directory (cap `run.ambient`): a workspace-
    // relative DIR path, never a ref — the strict wall holds the path law
    // here exactly as it holds `now`'s format and `invocation`'s charset,
    // so a malformed ambient refuses the frame before any target runs.
    let ambient = opt_str(obj, op, "ambient")?;
    if let Some(a) = &ambient {
        if addr::head_carries_root_separator(a) {
            return Err(bad_request(format!(
                "`ambient` must be a workspace-relative directory path, not a \
                 `root:` ref, on `run`: `{a}` — the run executes on the bound \
                 workspace; a bare birth path resolves under ambient inside it"
            )));
        }
        if a.is_empty() || !addr::confined(a) {
            return Err(bad_request(format!(
                "`ambient` must be a confined workspace-relative directory \
                 path on `run` (no absolute path, no `.`/`..`/empty segment): \
                 `{a}`"
            )));
        }
    }
    // Load-phase source (cap `run.mode`), one per call. Shape only at this
    // wall — a prelude that PARSES wrong is a `prelude_invalid` fault on the
    // rows, not a frame refusal, because it is code the engine evaluates,
    // not a field it validates.
    let prelude = opt_str(obj, op, "prelude")?;
    if let Some(p) = &prelude
        && p.is_empty()
    {
        return Err(bad_request(
            "`prelude` must be non-empty when present on `run` — an empty \
             prelude is the absent one spelled a second way; omit the field",
        ));
    }
    Ok(Op::Run {
        targets,
        invocation,
        prelude,
        actor: opt_str(obj, op, "actor")?,
        now,
        fields: decode_fields(obj, op)?,
        ambient,
    })
}

/// One § A.8 target: `page` required; `task`/`args`/`env`/`dry` optional.
/// `args` is POSITIONAL here (the run plane's contract shape), unlike the
/// script entry's inert dict — two entries, each speaking its own plane's
/// grammar verbatim.
///
/// *Amended 2026-08-23 (hook-support design § 2.2).* Six optional fields
/// join the closed set for the load/fire modes — `mode`, `block`, `input`,
/// `timeout_ms`, `budget`, `source`. The set stays closed and the wall stays
/// loud: an unnegotiated `mode` is refused BY NAME here (`` unknown field
/// `mode` on `targets[0]` of `run` ``), which is the shipped refusal, not a
/// new mechanism. Every EXCLUSION § 2.2 states is enforced below, because a
/// field that is meaningless on a target must refuse rather than be ignored
/// — silently dropping `env` on an evaluated fire would be the
/// guard-you-believe-is-armed trap in a second costume.
fn decode_run_target(
    t: &Map<String, Value>,
    i: usize,
    rev: Rev,
) -> Result<wire::RunTarget, Box<ErrorBody>> {
    /// The set as SHIPPED — what a non-v3 session is judged against.
    const SHIPPED_TARGET_FIELDS: [&str; 5] = ["page", "task", "args", "env", "dry"];
    /// The amended set: the shipped five plus the six this design adds.
    const TARGET_FIELDS: [&str; 11] = [
        "page",
        "task",
        "args",
        "env",
        "dry",
        "mode",
        "block",
        "input",
        "timeout_ms",
        "budget",
        "source",
    ];
    let admitted: &[&str] = if rev == Rev::V3 {
        &TARGET_FIELDS
    } else {
        &SHIPPED_TARGET_FIELDS
    };
    for key in t.keys() {
        if !admitted.contains(&key.as_str()) {
            return Err(bad_request(format!(
                "unknown field `{key}` on `targets[{i}]` of `run`"
            )));
        }
    }
    // The mode is read FIRST: it decides which of the two contracts the rest
    // of this row is judged against.
    let mode = decode_run_mode(t, i)?;
    // A draft's bytes stand in for `page` — the ONE place this amendment
    // relaxes a shipped requirement, and only on a mode-bearing target.
    let source = match t.get("source") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].source` must be a string on `run`"
            )));
        }
    };
    if source.is_some() && mode.is_none() {
        return Err(bad_request(format!(
            "`targets[{i}].source` needs `mode` on `run` — draft bytes are a \
             load/fire rehearsal; a task target runs corpus-declared blocks \
             only (§ A.8's named absence: the wire carries names, never code)"
        )));
    }
    let page = decode_run_page(t, i, source.is_some())?;
    let task = decode_run_task(t, i)?;
    let args = decode_run_args(t, i)?;
    let env = decode_run_env(t, i)?;
    let dry = match t.get("dry") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].dry` must be a boolean on `run`"
            )));
        }
    };
    let block = decode_run_block(t, i)?;
    // `input` is arbitrary JSON by design — the fire binds it as a REAL
    // starlark value, so there is no shape to check here beyond presence.
    // `null` is a VALUE (starlark `None`), not an absence, so it is kept.
    let input = t.get("input").cloned();
    let timeout_ms = decode_run_timeout(t, i)?;
    let budget = decode_run_budget(t, i)?;

    exclusions(
        i,
        &Target {
            mode,
            task: &task,
            args: &args,
            env: &env,
            block: &block,
            input: &input,
            timeout_ms,
            budget,
            source: &source,
        },
    )?;

    Ok(wire::RunTarget {
        page,
        task,
        args,
        env,
        // A `source` target FORCES `dry`: nothing runs live from wire bytes,
        // so no write door is reachable and § A.8's "the wire carries names,
        // never code" survives as "never code THAT RUNS". Forced here, at the
        // decode wall, so no downstream arm can forget it.
        dry: if source.is_some() { Some(true) } else { dry },
        mode,
        block,
        input,
        timeout_ms,
        budget,
        source,
    })
}

/// One target's `task` — the shipped addressing, unchanged.
fn decode_run_task(t: &Map<String, Value>, i: usize) -> Result<Option<String>, Box<ErrorBody>> {
    match t.get("task") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(bad_request(format!(
            "`targets[{i}].task` must be a string on `run`"
        ))),
    }
}

/// One mode-bearing target's `timeout_ms` CEILING (effective limit =
/// min(declared, ceiling)).
fn decode_run_timeout(t: &Map<String, Value>, i: usize) -> Result<Option<u64>, Box<ErrorBody>> {
    match t.get("timeout_ms") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(ms) if ms > 0 => Ok(Some(ms)),
            _ => Err(bad_request(format!(
                "`targets[{i}].timeout_ms` must be a positive integer on \
                 `run` — it is a CEILING (effective limit = min(declared, \
                 ceiling)); zero would name a target that cannot run"
            ))),
        },
    }
}

/// One target's `page`. Required everywhere except on a `source` target,
/// which carries the bytes instead — the ONE place this amendment relaxes a
/// shipped requirement.
fn decode_run_page(
    t: &Map<String, Value>,
    i: usize,
    has_source: bool,
) -> Result<String, Box<ErrorBody>> {
    match t.get("page") {
        Some(Value::String(p)) if !p.is_empty() => Ok(p.clone()),
        // `source` replaces `page`, and only there.
        None if has_source => Ok(String::new()),
        // Absent and present-but-wrong are ONE refusal on purpose: the
        // caller's next move is the same either way — put a non-empty string
        // there — and two spellings of it would be two strings to keep in
        // step for no reader's benefit.
        _ => Err(bad_request(format!(
            "`targets[{i}].page` must be a non-empty string on `run`"
        ))),
    }
}

/// One target's `mode`. Read FIRST at the call site: it decides which of the
/// two contracts the rest of the row is judged against.
fn decode_run_mode(
    t: &Map<String, Value>,
    i: usize,
) -> Result<Option<wire::RunMode>, Box<ErrorBody>> {
    Ok(match t.get("mode") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => match s.as_str() {
            "load" => Some(wire::RunMode::Load),
            "fire" => Some(wire::RunMode::Fire),
            other => {
                return Err(bad_request(format!(
                    "`targets[{i}].mode` must be `load` or `fire` on `run`: `{other}`"
                )));
            }
        },
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].mode` must be a string on `run`"
            )));
        }
    })
}

/// One target's positional `args` — the run plane's contract shape (a LIST
/// of strings), unlike the script entry's inert dict.
fn decode_run_args(t: &Map<String, Value>, i: usize) -> Result<Vec<String>, Box<ErrorBody>> {
    Ok(match t.get("args") {
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
    })
}

/// One target's declared `env` pairs, contract-validated by the plane.
fn decode_run_env(
    t: &Map<String, Value>,
    i: usize,
) -> Result<std::collections::BTreeMap<String, String>, Box<ErrorBody>> {
    Ok(match t.get("env") {
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
    })
}

/// One target's `block` — the `^id` anchor of a declared block.
fn decode_run_block(t: &Map<String, Value>, i: usize) -> Result<Option<String>, Box<ErrorBody>> {
    Ok(match t.get("block") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            // §2.4's ONE charset, both planes. The `^` is the address
            // grammar's sigil, not part of the id — a caller that sends it
            // is told so rather than silently trimmed.
            if let Some(rest) = s.strip_prefix('^') {
                return Err(bad_request(format!(
                    "`targets[{i}].block` carries the `^` sigil on `run`: \
                     `{s}` — send the bare block id (`{rest}`); the anchor \
                     grammar's `^` belongs to the address, not the field"
                )));
            }
            if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(bad_request(format!(
                    "`targets[{i}].block` must be a non-empty block id in the \
                     [A-Za-z0-9-] charset on `run` (§2.4, one charset both \
                     planes): `{s}`"
                )));
            }
            Some(s.clone())
        }
        Some(_) => {
            return Err(bad_request(format!(
                "`targets[{i}].block` must be a string on `run`"
            )));
        }
    })
}

/// One decoded target's fields, for the exclusion pass. A struct rather than
/// nine positional arguments: most are `Option`s of the same shape, and two
/// swapped at a call site would compile and refuse the wrong field.
struct Target<'a> {
    mode: Option<wire::RunMode>,
    task: &'a Option<String>,
    args: &'a [String],
    env: &'a std::collections::BTreeMap<String, String>,
    block: &'a Option<String>,
    input: &'a Option<Value>,
    timeout_ms: Option<u64>,
    budget: Option<wire::EvalBudget>,
    source: &'a Option<String>,
}

/// § 2.2's exclusion rules, all of them, in one place.
///
/// Each refuses BY NAME; none is ignored. A field that is meaningless on a
/// target must refuse rather than be silently dropped — quietly ignoring
/// `env` on an evaluated fire would be the guard-you-believe-is-armed trap
/// (§3.2's strict-wall rationale) wearing a second costume: the caller
/// believes it set something, and nothing says otherwise.
fn exclusions(i: usize, t: &Target<'_>) -> Result<(), Box<ErrorBody>> {
    match t.mode {
        None => {
            // A task target keeps § A.8's named absences verbatim.
            if t.block.is_some() || t.input.is_some() {
                let named = if t.block.is_some() { "block" } else { "input" };
                return Err(bad_request(format!(
                    "`targets[{i}].{named}` needs `mode` on `run` — it \
                     addresses a declared BLOCK, and a target with no `mode` \
                     is the shipped task path, which addresses a task NAME"
                )));
            }
            if t.timeout_ms.is_some() || t.budget.is_some() {
                let named = if t.timeout_ms.is_some() {
                    "timeout_ms"
                } else {
                    "budget"
                };
                return Err(bad_request(format!(
                    "`targets[{i}].{named}` is refused on a task target of \
                     `run` (§ A.8's named absence): a task's limits come from \
                     the declaring root's config — the page side declares, \
                     the caller does not tune. Caller ceilings ride \
                     mode-bearing targets only"
                )));
            }
        }
        Some(m) => {
            if t.task.is_some() {
                return Err(bad_request(format!(
                    "`targets[{i}]` carries both `task` and `mode` on `run` — \
                     the two addressings are exclusive: `task` names a \
                     frontmatter-declared task, `mode` addresses a \
                     `declare()` block"
                )));
            }
            if !t.args.is_empty() {
                return Err(bad_request(format!(
                    "`targets[{i}].args` is refused on a `mode: {}` target of \
                     `run` — argv is the task contract's channel; a fire's one \
                     input channel is `input`",
                    m.as_str()
                )));
            }
            if m == wire::RunMode::Load {
                if t.block.is_some() || t.input.is_some() {
                    let named = if t.block.is_some() { "block" } else { "input" };
                    return Err(bad_request(format!(
                        "`targets[{i}].{named}` is refused on a `mode: load` \
                         target of `run` — a load evaluates EVERY block's top \
                         level on the page and answers their declarations; it \
                         addresses no single block and calls no entry"
                    )));
                }
                if !t.env.is_empty() {
                    return Err(bad_request(format!(
                        "`targets[{i}].env` is refused on a `mode: load` \
                         target of `run` — a load starts no process and calls \
                         no entry, so nothing exists to receive it"
                    )));
                }
            }
            if m == wire::RunMode::Fire && t.block.is_none() && t.source.is_none() {
                return Err(bad_request(format!(
                    "`targets[{i}].block` is required with `mode: fire` on \
                     `run` — a fire calls ONE declared block's entry, named by \
                     its `^id` anchor"
                )));
            }
        }
    }
    Ok(())
}

/// One mode-bearing target's caller ceilings (cap `run.mode`). A closed
/// two-field object: a ceiling that names a field the kernel has no knob for
/// is a caller believing it bounded something, so it refuses by name.
fn decode_run_budget(
    t: &Map<String, Value>,
    i: usize,
) -> Result<Option<wire::EvalBudget>, Box<ErrorBody>> {
    let Some(v) = t.get("budget") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let Value::Object(map) = v else {
        return Err(bad_request(format!(
            "`targets[{i}].budget` must be an object of `steps`/`mem` on `run`"
        )));
    };
    for key in map.keys() {
        if !matches!(key.as_str(), "steps" | "mem") {
            return Err(bad_request(format!(
                "unknown field `{key}` on `targets[{i}].budget` of `run` — \
                 the ceilings are `steps` and `mem`"
            )));
        }
    }
    let field = |name: &str| -> Result<Option<u64>, Box<ErrorBody>> {
        match map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_u64() {
                Some(n) if n > 0 => Ok(Some(n)),
                _ => Err(bad_request(format!(
                    "`targets[{i}].budget.{name}` must be a positive integer \
                     on `run` — it is a CEILING, and zero would name a target \
                     that cannot run"
                ))),
            },
        }
    };
    let budget = wire::EvalBudget {
        steps: field("steps")?,
        mem: field("mem")?,
    };
    if budget.steps.is_none() && budget.mem.is_none() {
        return Err(bad_request(format!(
            "`targets[{i}].budget` names no ceiling on `run` — give `steps`, \
             `mem`, or both; an empty budget is the absent one spelled a \
             second way"
        )));
    }
    Ok(Some(budget))
}

/// Composed `read` (v3-only at dispatch; decode is rev-agnostic).
fn decode_read(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "read";
    // No `actor`: a read is identity-free (§ A.3 proof law) — the retired
    // field refuses at this wall like any unknown field.
    check_fields(obj, op, &["path", "toc", "sections", "display_path"])?;
    // `sections` and `toc` are structured on the wire: the caller states the
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
    let toc = match obj.get("toc") {
        None | Some(Value::Null) => None,
        Some(v @ Value::Object(_)) => Some(
            serde_json::from_value::<wire::ReadSel>(v.clone()).map_err(|_| {
                bad_request(
                    "`toc` must be ONE tagged section selector on `read` — \
                     `{\"hpath\":[{\"h\":\"Notes\"},{\"h\":\"Deep\"}]}` for a heading path or \
                     `{\"n\":\"1.2\"}` for a dewey ordinal. A joined string is not an \
                     address on this face (U14): convert it once at your own ingress door.",
                )
            })?,
        ),
        Some(_) => {
            return Err(bad_request(
                "`toc` must be one tagged section selector on `read`",
            ));
        }
    };
    Ok(Op::Read {
        path: req_path(obj, op, "path")?,
        toc,
        sections,
        display_path: opt_str(obj, op, "display_path")?,
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
pub(crate) const CREATE_FIELDS: [&str; 7] =
    ["path", "body", "actor", "now", "if_root", "dry", "fields"];

/// § A.2.1 `fields`: an optional object of STRING values, opaque — decoded
/// shape only, no key interpreted. Absent decodes as the empty map.
fn decode_fields(
    obj: &Map<String, Value>,
    op: &str,
) -> Result<std::collections::BTreeMap<String, String>, Box<ErrorBody>> {
    match obj.get("fields") {
        None | Some(Value::Null) => Ok(std::collections::BTreeMap::new()),
        Some(Value::Object(map)) => {
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in map {
                let Some(s) = v.as_str() else {
                    return Err(bad_request(format!(
                        "`fields` values must be strings on `{op}` (§ A.2.1 — the passthrough \
                         is a flat string map): key `{k}` is not a string"
                    )));
                };
                out.insert(k.clone(), s.to_owned());
            }
            Ok(out)
        }
        Some(_) => Err(bad_request(format!(
            "`fields` must be an object of string values on `{op}` (§ A.2.1)"
        ))),
    }
}

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
        fields: decode_fields(obj, op)?,
    })
}

/// Death-op fields. No `force` — the one irreversible op has no escape hatch
/// (§ A.3 remove door; the forced-birth precedent applied to death).
pub(crate) const REMOVE_FIELDS: [&str; 6] =
    ["path", "if_file_rev", "actor", "now", "if_root", "dry"];

/// Strict-decode `remove`. Rev-agnostic; v3 gate at dispatch. `now` is RFC
/// 3339. `if_file_rev` decodes as optional (§ A.1 schema-optional guard law);
/// the door's own `guard_required` demand is semantic, after decode.
fn decode_remove(obj: &Map<String, Value>) -> Result<Op, Box<ErrorBody>> {
    let op = "remove";
    check_fields(obj, op, &REMOVE_FIELDS)?;
    let now = opt_str(obj, op, "now")?;
    if let Some(n) = &now
        && !wire::now_is_rfc3339(n)
    {
        return Err(bad_request(format!(
            "`now` must be RFC 3339 (§9, validated never generated): `{n}`"
        )));
    }
    Ok(Op::Remove {
        path: req_path(obj, op, "path")?,
        if_file_rev: opt_str(obj, op, "if_file_rev")?.map(wire::NodeRev),
        actor: opt_str(obj, op, "actor")?,
        now,
        if_root: opt_str(obj, op, "if_root")?.map(wire::Root),
        dry: opt_bool(obj, op, "dry")?,
    })
}

/// `splice`: only write-existing under v2; v3 also admits `plan_edits`/`pin`
/// and the `files[]` set form (dotted cap `splice.set`).
/// `now` RFC 3339 validated, never generated (§9).
fn decode_splice(obj: &Map<String, Value>, rev: Rev) -> Result<Op, Box<ErrorBody>> {
    let op = "splice";
    refuse_unnegotiated_family(obj, rev)?;
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
    // The §4.4 set form: strictly one form or the other — `files[]` XOR the
    // single form's `path`+`edits`/`plan_edits`/`pin` (`bad_request` when both
    // or neither appear).
    if let Some(files_v) = obj.get("files") {
        for single in ["path", "edits", "plan_edits", "pin", "fields"] {
            if obj.contains_key(single) {
                return Err(bad_request(format!(
                    "`files` and `{single}` are mutually exclusive on `splice` — the set form \
                     carries each member's path and batch inside `files[]` (§4.4 set form)"
                )));
            }
        }
        return decode_splice_set(obj, files_v, now);
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
    let if_root = opt_str(obj, op, "if_root")?.map(wire::Root);
    let scope = opt_path(obj, op, "scope")?;
    let guards = decode_guards(obj.get("guards"))?;
    if scope.is_some() && if_root.is_none() {
        return Err(bad_request(wire::broken_premise_pair_teaching(
            "scope without if_fingerprint",
        )));
    }
    Ok(Op::Splice {
        path: req_path(obj, op, "path")?,
        actor: opt_str(obj, op, "actor")?,
        now,
        receipt: obj.get("receipt").map(decode_receipt).transpose()?,
        if_root,
        dry: opt_bool(obj, op, "dry")?,
        force: opt_bool(obj, op, "force")?,
        edits,
        plan_edits,
        pin,
        scope,
        guards,
        fields: decode_fields(obj, op)?,
    })
}

/// The §4.4 set form's members: two or more `{path, edits|plan_edits}`
/// entries, paths pairwise distinct, per-entry field wall and per-entry
/// edits-vs-plan_edits exclusion (the single form's own walls, per member).
fn decode_splice_set(
    obj: &Map<String, Value>,
    files_v: &Value,
    now: Option<String>,
) -> Result<Op, Box<ErrorBody>> {
    let Value::Array(items) = files_v else {
        return Err(bad_request(
            "`files` must be an array of `{path, edits|plan_edits}` members on `splice`",
        ));
    };
    if items.len() < 2 {
        return Err(bad_request(
            "`files` must carry two or more members (§4.4 set form) — a one-file write is \
             the single `path` form",
        ));
    }
    let mut files = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Value::Object(entry) = item else {
            return Err(bad_request(format!(
                "`files[{i}]` must be an object with `path` and `edits` or `plan_edits`"
            )));
        };
        check_fields(entry, "files[]", &["path", "edits", "plan_edits"])?;
        let path = req_path(entry, "files[]", "path")?;
        let plan_edits = match entry.get("plan_edits") {
            None => Vec::new(),
            Some(v) => decode_plan_edits(v)?,
        };
        let edits = match entry.get("edits") {
            Some(edits_v) => {
                if !plan_edits.is_empty() {
                    return Err(bad_request(format!(
                        "`edits` and `plan_edits` are mutually exclusive on `files[{i}]`"
                    )));
                }
                decode_edits(edits_v, Laws::Full)?
            }
            None if plan_edits.is_empty() => {
                return Err(bad_request(format!(
                    "missing `edits` on `files[{i}]` — every set member carries its batch"
                )));
            }
            None => Vec::new(),
        };
        if files.iter().any(|f: &wire::SpliceFile| f.path == path) {
            return Err(bad_request(format!(
                "set member paths must be pairwise distinct: `{}` appears twice — merge its \
                 edits into one member",
                path.0
            )));
        }
        files.push(wire::SpliceFile {
            path,
            edits,
            plan_edits,
        });
    }
    let if_root = opt_str(obj, "splice", "if_root")?.map(wire::Root);
    let scope = opt_path(obj, "splice", "scope")?;
    let guards = decode_guards(obj.get("guards"))?;
    if scope.is_some() && if_root.is_none() {
        return Err(bad_request(wire::broken_premise_pair_teaching(
            "scope without if_fingerprint",
        )));
    }
    Ok(Op::SpliceSet {
        files,
        actor: opt_str(obj, "splice", "actor")?,
        now,
        receipt: obj.get("receipt").map(decode_receipt).transpose()?,
        if_root,
        dry: opt_bool(obj, "splice", "dry")?,
        force: opt_bool(obj, "splice", "force")?,
        scope,
        guards,
    })
}

/// `splice.pin`: `{target, selector, vibe?}`. No `actor` — identity is
/// splice's daemon-derived actor only.
fn decode_pin(v: &Value) -> Result<wire::PinSpec, Box<ErrorBody>> {
    let Some(obj) = v.as_object() else {
        return Err(bad_request("`pin` must be an object on `splice`"));
    };
    check_fields(
        obj,
        "pin",
        &["target", "selector", "vibe", "fingerprint", "sec_rev"],
    )?;
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
        fingerprint: opt_str(obj, "pin", "fingerprint")?,
        sec_rev: opt_str(obj, "pin", "sec_rev")?,
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
                "a plan edit must carry exactly one of `append`/`match`/`replace_section`/`create`/`set_property`/`remove_property`",
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
            // The retire row (§ A.6.6). No `value` field — and the wall says so
            // by name, because a caller sending one means `set_property` and
            // silently ignoring it would remove a key where a set was intended.
            "remove_property" => {
                plan_fields(b, "remove_property", &["key", "rev"])?;
                PlanEdit::RemoveProperty {
                    key: req_str(b, "remove_property", "key")?,
                    // The same file-grain doc-root token `set_property` takes:
                    // one plane, one grain, whichever direction the write runs.
                    rev: opt_str(b, "remove_property", "rev")?,
                }
            }
            other => {
                return Err(bad_request(format!(
                    "unknown plan edit shape `{other}` — one of append/match/replace_section/create/set_property/remove_property"
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

/// Exactly three edit shapes (§4.4): externally tagged `match` / `put` /
/// `remove`.
fn decode_edit_shape(v: &Value) -> Result<wire::EditShape, Box<ErrorBody>> {
    let Value::Object(shape) = v else {
        return Err(bad_request("`edit` must be an object"));
    };
    for key in shape.keys() {
        if !["match", "put", "remove"].contains(&key.as_str()) {
            return Err(bad_request(format!(
                "unknown field `{key}` in `edit` — legal fields are `match`, `put`, `remove`"
            )));
        }
    }
    // The identity shape (§ A.6.6) carries no fields — it writes no value, so
    // there is nothing to encode and nothing to spell. The wall is loud here
    // for the same reason it is loud everywhere: a `text` inside `remove` is a
    // caller who means `put`, and silently dropping it would write a removal
    // where a set was intended.
    if let Some(r) = shape.get("remove") {
        if shape.len() > 1 {
            return Err(bad_request(
                "`edit` must carry exactly one of `match`/`put`/`remove`",
            ));
        }
        let Value::Object(fields) = r else {
            return Err(bad_request("`remove` must be an object — send `{}`"));
        };
        if let Some(key) = fields.keys().next() {
            return Err(bad_request(format!(
                "unknown field `{key}` in `remove` — it takes no fields: `remove` strikes the \
                 frontmatter key line, it writes no value. To SET a value use \
                 `put{{at:\"upsert\"}}`."
            )));
        }
        return Ok(wire::EditShape::Remove {});
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

/// Frozen-v2 sessions never negotiate `scoped-guards`. A family field on
/// that session is un-negotiated use, not an unknown typo — the teaching
/// names the cap, not the field-wall list.
fn refuse_unnegotiated_family(obj: &Map<String, Value>, rev: Rev) -> Result<(), Box<ErrorBody>> {
    if rev == Rev::V3 {
        return Ok(());
    }
    for field in FAMILY_FIELDS {
        if obj.contains_key(field) {
            return Err(bad_request(wire::unnegotiated_family_teaching(field)));
        }
    }
    Ok(())
}

/// Decode `guards[]`: each entry `{scope?, scope_bytes?, fingerprint}`.
/// `fingerprint` is required; exactly one scope spelling, or neither for
/// the root premise. Both spellings refuse the broken-pair teaching.
fn decode_guards(v: Option<&Value>) -> Result<Vec<GuardEntry>, Box<ErrorBody>> {
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = v else {
        return Err(bad_request(
            "`guards` must be an array of `{fingerprint, scope?, scope_bytes?}` entries",
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Value::Object(entry) = item else {
            return Err(bad_request(format!(
                "`guards[{i}]` must be an object with `fingerprint` and at most one scope spelling"
            )));
        };
        check_fields(entry, "guards[]", &["scope", "scope_bytes", "fingerprint"])?;
        let fingerprint = req_str(entry, "guards[]", "fingerprint")?;
        if fingerprint.is_empty() {
            return Err(bad_request(wire::broken_premise_pair_teaching(
                format!("guards[{i}] carries no fingerprint").as_str(),
            )));
        }
        let scope = opt_path(entry, "guards[]", "scope")?;
        let scope_bytes = opt_str(entry, "guards[]", "scope_bytes")?;
        if scope.is_some() && scope_bytes.is_some() {
            return Err(bad_request(wire::broken_premise_pair_teaching(
                "both scope and scope_bytes in one premise",
            )));
        }
        out.push(GuardEntry {
            scope,
            scope_bytes,
            fingerprint,
        });
    }
    Ok(out)
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

/// Optional twin of [`req_u64`]: absent → `None`, present → integer-law.
fn opt_u64(obj: &Map<String, Value>, op: &str, key: &str) -> Result<Option<u64>, Box<ErrorBody>> {
    obj.get(key)
        .map(|v| match v {
            Value::Number(n) if n.as_u64().is_some() => Ok(n.as_u64().unwrap_or_default()),
            _ => Err(bad_request(format!(
                "`{key}` on `{op}` must be a non-negative integer"
            ))),
        })
        .transpose()
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
