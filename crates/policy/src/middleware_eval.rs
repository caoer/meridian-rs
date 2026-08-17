//! Sealed `middleware(ctx)` evaluator — the door leg's metered runner
//! (armed-plane Part A2). One evaluation per armed in-scope middleware page,
//! after CAS and batch validation, before bytes land. The program may
//! `refuse`, emit `set_field` / `create` (compiled into the caller's sealed
//! set by the mount), and emit `send` intents (never applied here).
//!
//! `policy` stays I/O-free: the world the program queries through `ctx.sql` /
//! `ctx.read` is an injected [`MwWorld`] the write door owns. Metering,
//! containment and the failure surface are the CHECK evaluator's
//! ([`crate::check_eval`]) — one limit set, one bomb story.

use std::cell::RefCell;
use std::collections::BTreeMap;

use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::AstModule;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;

use crate::change::Change;
use crate::check_eval::{
    CheckError, CheckLimits, alloc_docfacts, alloc_edit, alloc_target, budget, check_dialect,
    check_nesting_depth, check_source_size, heap_bytes, is_depth_overflow, on_eval_stack, opt_str,
    runtime,
};
use crate::declaration::Refusal;

/// One cell of a `ctx.sql` row — the closed scalar surface the world serves.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    /// SQL NULL.
    Null,
    /// Any textual value.
    Text(String),
    /// Any integral value.
    Int(i64),
    /// Any floating value.
    Float(f64),
    /// A boolean.
    Bool(bool),
}

/// One `ctx.sql` row: `(column name, value)` in projection order.
pub type SqlRow = Vec<(String, SqlValue)>;

/// The overlay world a middleware queries — injected by the write door, which
/// owns the snapshot and the pending-set overlay. `policy` performs no I/O.
/// `Sync` because evaluation runs on the dedicated large-stack thread
/// ([`on_eval_stack`]) and the reference crosses into it.
pub trait MwWorld: Sync {
    /// The bytes at `path` in the current overlay world, or `None` when the
    /// path does not exist there.
    fn read(&self, path: &str) -> Option<String>;

    /// One read-only SELECT against the current overlay world's projection.
    ///
    /// # Errors
    /// The backend's own message — a missing backend, a projection failure, or
    /// a SQL error. The evaluator surfaces it as a runtime fault, which the
    /// gate fails CLOSED on.
    fn sql(&self, query: &str) -> Result<Vec<SqlRow>, String>;
}

/// One disk/wire emission from a middleware evaluation, in emission order.
/// The mount compiles these into the caller's sealed set (`set_field`,
/// `create`) or onto the response (`send`); this crate only records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MwEmit {
    /// `set_field(path=, key=, value=)` — a frontmatter upsert on `path`
    /// (this file or another), joining the sealed set.
    SetField {
        /// The workspace-relative file the field lands on.
        path: String,
        /// The top-level frontmatter key.
        key: String,
        /// The value, landed verbatim.
        value: String,
    },
    /// `create(path=, body=)` — a birth in the same sealed set.
    Create {
        /// The workspace-relative path born.
        path: String,
        /// The full body bytes.
        body: String,
    },
    /// `send(to=, body=)` — a V1 intent on the write response. The engine
    /// never delivers it and never marks it delivered.
    Send {
        /// Seat or channel names, verbatim.
        to: Vec<String>,
        /// The message body, verbatim.
        body: String,
    },
}

/// One middleware evaluation's outcome: the refusals it emitted (any ⇒ the
/// write refuses whole) and its emissions in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MwOutcome {
    /// Refusals, in emission order. Non-empty refuses the whole sealed set.
    pub refusals: Vec<Refusal>,
    /// Emissions, in emission order.
    pub emits: Vec<MwEmit>,
}

/// What one evaluation injects as `ctx` — the change surface plus the opaque
/// `fields` passthrough. `change.before` is this file before the put;
/// `change.doc` is this file after the pending set SO FAR (caller put plus
/// earlier middleware transforms — the mount re-derives it between rows).
pub struct MwCtxInput<'a> {
    /// The `rulepack-api@2` change surface for this write.
    pub change: &'a Change,
    /// The put frame's opaque `fields` map (wire-contract § A.2.1).
    pub fields: &'a BTreeMap<String, String>,
}

/// Run one middleware program over `ctx` under the FULL limits.
///
/// Same containment story as [`crate::check_eval::run_check_change`]: a
/// dedicated large stack, tick/heap/call-depth guards, source-size and
/// nesting caps, and `catch_unwind` for allocation bombs — a hostile program
/// terminates with a typed [`CheckError`], never hangs, never crashes.
///
/// # Errors
/// [`CheckError`] — the door fails CLOSED on every variant (a middleware that
/// cannot complete never reads as a pass).
pub fn run_middleware(
    source: &str,
    input: &MwCtxInput<'_>,
    world: &dyn MwWorld,
    limits: CheckLimits,
) -> Result<MwOutcome, CheckError> {
    on_eval_stack(|| eval_middleware(source, input, world, limits))
}

/// The middleware ceiling globals: the Starlark standard library plus exactly
/// the door vocabulary — `refuse`, `set_field`, `create`, `send`, `sql`,
/// `read`. `sql` and `read` are the same function values bound at `ctx.sql` /
/// `ctx.read`; both spellings reach one implementation.
fn middleware_globals() -> Globals {
    GlobalsBuilder::standard().with(middleware_api).build()
}

/// The metered evaluation body (runs on the large stack).
fn eval_middleware(
    source: &str,
    input: &MwCtxInput<'_>,
    world: &dyn MwWorld,
    limits: CheckLimits,
) -> Result<MwOutcome, CheckError> {
    check_source_size(source, limits)?;
    check_nesting_depth(source)?;

    let globals = middleware_globals();
    let store = MwStore {
        refusals: RefCell::new(Vec::new()),
        emits: RefCell::new(Vec::new()),
        world,
    };
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    // AssertUnwindSafe: on panic the store is discarded unread and a budget
    // fault returned (the check evaluator's exact discipline).
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            // ctx carries function values allocated in the globals' frozen
            // heap; keep that heap alive from the module's.
            module.frozen_heap().add_reference(globals.heap());
            let heap = module.heap();
            let ctx_value = alloc_ctx(heap, &globals, input);

            let mut eval = Evaluator::new(&module);
            if let Err(e) = eval.set_max_tick_count(step_guard) {
                return Err(runtime(e));
            }
            if let Err(e) = eval.set_max_heap_size(mem_guard) {
                return Err(runtime(e));
            }
            if let Err(e) = eval.set_max_callstack_size(limits.max_call_depth.max(1)) {
                return Err(runtime(e));
            }
            eval.extra = Some(&store);

            let ast = match AstModule::parse("MIDDLEWARE.md", source.to_owned(), &check_dialect()) {
                Ok(ast) => ast,
                Err(e) => {
                    return Err(CheckError::Parse {
                        reason: e.to_string(),
                    });
                }
            };

            let mut aborted = false;
            let mut depth_overflow = false;
            let mut fault: Option<String> = None;
            let mut missing = false;
            match eval.eval_module(ast, &globals) {
                Ok(_) => match module.get("middleware") {
                    Some(entry) => {
                        if let Err(e) = eval.eval_function(entry, &[ctx_value], &[]) {
                            aborted = true;
                            depth_overflow = is_depth_overflow(&e);
                            fault = Some(e.to_string());
                        }
                    }
                    None => missing = true,
                },
                Err(e) => {
                    aborted = true;
                    depth_overflow = is_depth_overflow(&e);
                    fault = Some(e.to_string());
                }
            }

            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);
            let refusals = store.refusals.take();
            let emits = store.emits.take();

            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            if missing {
                Err(CheckError::MissingEntry)
            } else if aborted {
                if over_budget || depth_overflow {
                    Err(budget(limits))
                } else {
                    Err(CheckError::Runtime {
                        reason: fault.unwrap_or_default(),
                    })
                }
            } else if over_budget {
                Err(budget(limits))
            } else {
                Ok(MwOutcome { refusals, emits })
            }
        })
    }));

    match evaluated {
        Ok(inner) => inner,
        Err(_panic) => Err(budget(limits)),
    }
}

// ── ctx allocation ────────────────────────────────────────────────────────────

/// Allocate the injected `ctx` struct (armed-plane Part A2 § The ctx surface):
/// `op`, `before`, `after`, `put`, `fields`, and the bound `sql` / `read`
/// function values pulled from the ceiling globals.
fn alloc_ctx<'v>(heap: Heap<'v>, globals: &Globals, input: &MwCtxInput<'_>) -> Value<'v> {
    let change = input.change;
    let edits: Vec<Value<'v>> = change.edits.iter().map(|e| alloc_edit(heap, e)).collect();
    let targets: Vec<Value<'v>> = change
        .targets
        .iter()
        .map(|t| alloc_target(heap, t))
        .collect();
    let fields_changed: Vec<Value<'v>> = change
        .fields_changed
        .iter()
        .map(|s| heap.alloc(s.as_str()))
        .collect();
    let sections_changed: Vec<Value<'v>> = change
        .sections_changed
        .iter()
        .map(|hpath| {
            let segs: Vec<Value<'v>> = hpath.iter().map(|s| heap.alloc(s.as_str())).collect();
            heap.alloc(segs)
        })
        .collect();
    let put = heap.alloc(AllocStruct([
        ("op", heap.alloc(change.op.as_str())),
        ("actor", opt_str(heap, change.actor.as_deref())),
        ("force", heap.alloc(change.force)),
        ("edits", heap.alloc(edits)),
        ("targets", heap.alloc(targets)),
        ("fields_changed", heap.alloc(fields_changed)),
        ("sections_changed", heap.alloc(sections_changed)),
    ]));
    let fields = heap.alloc(AllocDict(
        input
            .fields
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
    ));
    // The bound accessors: the SAME frozen function values the globals carry,
    // so `ctx.sql(...)` and bare `sql(...)` are one implementation.
    let bound = |name: &str| -> Value<'v> {
        globals
            .iter()
            .find(|(n, _)| *n == name)
            .map_or_else(Value::new_none, |(_, v)| v.to_value())
    };
    heap.alloc(AllocStruct([
        ("op", heap.alloc(change.op.as_str())),
        ("before", alloc_docfacts(heap, &change.before)),
        ("after", alloc_docfacts(heap, &change.doc)),
        ("put", put),
        ("fields", fields),
        ("sql", bound("sql")),
        ("read", bound("read")),
    ]))
}

/// Allocate one SQL row as a Starlark dict `{column: value}`.
fn alloc_sql_row<'v>(heap: Heap<'v>, row: &SqlRow) -> Value<'v> {
    heap.alloc(AllocDict(row.iter().map(|(name, value)| {
        let v: Value<'v> = match value {
            SqlValue::Null => Value::new_none(),
            SqlValue::Text(s) => heap.alloc(s.as_str()),
            SqlValue::Int(i) => heap.alloc(*i),
            SqlValue::Float(f) => heap.alloc(*f),
            SqlValue::Bool(b) => Value::new_bool(*b),
        };
        (heap.alloc(name.as_str()), v)
    })))
}

// ── The door builtins ─────────────────────────────────────────────────────────

/// What `Evaluator::extra` carries on the door plane: the emit store plus the
/// injected world.
#[derive(starlark::any::ProvidesStaticType)]
struct MwStore<'h> {
    refusals: RefCell<Vec<Refusal>>,
    emits: RefCell<Vec<MwEmit>>,
    world: &'h dyn MwWorld,
}

/// The store out of `Evaluator::extra` — absent only outside a metered run.
fn mw_store<'a, 'e>(eval: &'a Evaluator<'_, '_, 'e>) -> anyhow::Result<&'a MwStore<'e>> {
    eval.extra
        .and_then(|e| e.downcast_ref::<MwStore<'e>>())
        .ok_or_else(|| anyhow::anyhow!("middleware-api: builtin invoked without an emit store"))
}

/// `send(to=…)`'s target: one seat string or a list of them.
fn to_list(to: Value<'_>) -> anyhow::Result<Vec<String>> {
    if let Some(s) = to.unpack_str() {
        return Ok(vec![s.to_owned()]);
    }
    if let Some(items) = starlark::values::list::ListRef::from_value(to) {
        let mut out = Vec::with_capacity(items.len());
        for item in items.iter() {
            let Some(s) = item.unpack_str() else {
                return Err(anyhow::anyhow!(
                    "send: `to` must be a string or a list of strings"
                ));
            };
            out.push(s.to_owned());
        }
        return Ok(out);
    }
    Err(anyhow::anyhow!(
        "send: `to` must be a string or a list of strings"
    ))
}

/// The door builtins — the ENTIRE middleware surface beyond the Starlark
/// standard library. Every argument is named (except `sql`/`read`'s single
/// positional); nothing here writes — emits are recorded for the mount to
/// compile, and `sql`/`read` consult the injected world read-only.
#[starlark_module]
fn middleware_api(builder: &mut GlobalsBuilder) {
    /// `refuse(message=, passing=)` — refuse the WHOLE write. `passing` cites
    /// the passing scenario, required exactly as on the CHECK plane.
    fn refuse(
        #[starlark(require = named)] message: String,
        #[starlark(require = named)] passing: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        mw_store(eval)?.refusals.borrow_mut().push(Refusal {
            message,
            passing_scenario: passing,
        });
        Ok(NoneType)
    }

    /// `set_field(path=, key=, value=)` — record one frontmatter upsert for
    /// the sealed set (this file or another).
    fn set_field(
        #[starlark(require = named)] path: String,
        #[starlark(require = named)] key: String,
        #[starlark(require = named)] value: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        mw_store(eval)?
            .emits
            .borrow_mut()
            .push(MwEmit::SetField { path, key, value });
        Ok(NoneType)
    }

    /// `create(path=, body=)` — record one birth for the sealed set.
    fn create(
        #[starlark(require = named)] path: String,
        #[starlark(require = named)] body: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        mw_store(eval)?
            .emits
            .borrow_mut()
            .push(MwEmit::Create { path, body });
        Ok(NoneType)
    }

    /// `send(to=, body=)` — record one V1 `send` intent. Never delivered by
    /// the engine; the host realizes it and answers on the put.
    fn send(
        #[starlark(require = named)] to: Value<'_>,
        #[starlark(require = named)] body: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let to = to_list(to)?;
        mw_store(eval)?
            .emits
            .borrow_mut()
            .push(MwEmit::Send { to, body });
        Ok(NoneType)
    }

    /// `sql(query)` — one read-only SELECT against the current overlay world;
    /// returns a list of `{column: value}` dicts. Also bound at `ctx.sql`.
    fn sql<'v>(
        #[starlark(require = pos)] query: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let rows = mw_store(eval)?
            .world
            .sql(&query)
            .map_err(|reason| anyhow::anyhow!("ctx.sql: {reason}"))?;
        let values: Vec<Value<'v>> = rows.iter().map(|row| alloc_sql_row(heap, row)).collect();
        Ok(heap.alloc(AllocList(values)))
    }

    /// `read(path)` — the path's bytes in the current overlay world, or
    /// `None`. Also bound at `ctx.read`.
    fn read<'v>(
        #[starlark(require = pos)] path: String,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        Ok(match mw_store(eval)?.world.read(&path) {
            Some(bytes) => heap.alloc(bytes),
            None => Value::new_none(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{ChangeOp, Invocation, derive_change};
    use model::{Document, NodeKind};

    fn doc_of(path: &str, md: &str) -> Document {
        let nodes = syntax::parse(md);
        let mut doc = model::build(md.to_string(), nodes);
        if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    fn status_change() -> Change {
        let before = doc_of(
            "tasks/t.md",
            "---\nowner: alice\nstatus: open\n---\n# T\n\nx\n",
        );
        let after = doc_of(
            "tasks/t.md",
            "---\nowner: alice\nstatus: review\n---\n# T\n\nx\n",
        );
        derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some("agent:bob"),
                force: false,
            },
            &[],
            &|_| None,
        )
    }

    /// A world of two fixed pages and a canned SQL answer.
    struct FakeWorld;

    impl MwWorld for FakeWorld {
        fn read(&self, path: &str) -> Option<String> {
            match path {
                "agents/a.md" => Some("---\nreports-to: old\n---\n# A\n".to_string()),
                _ => None,
            }
        }

        fn sql(&self, query: &str) -> Result<Vec<SqlRow>, String> {
            if query.contains("boom") {
                return Err("no backend at this door".to_string());
            }
            Ok(vec![vec![
                ("path".to_string(), SqlValue::Text("agents/a.md".into())),
                ("n".to_string(), SqlValue::Int(2)),
                ("gone".to_string(), SqlValue::Null),
            ]])
        }
    }

    fn run(source: &str) -> Result<MwOutcome, CheckError> {
        let change = status_change();
        let fields: BTreeMap<String, String> =
            [("session".to_string(), "0ecc9d6a".to_string())].into();
        run_middleware(
            source,
            &MwCtxInput {
                change: &change,
                fields: &fields,
            },
            &FakeWorld,
            CheckLimits::default(),
        )
    }

    #[test]
    fn the_full_ctx_surface_is_readable_and_emits_are_recorded_in_order() {
        let out = run("\
def middleware(ctx):
    seen = ctx.op + ctx.before.path + ctx.after.path + ctx.put.op
    if ctx.after.frontmatter.get(\"status\") == \"review\":
        set_field(path = ctx.after.path, key = \"session\", value = ctx.fields.get(\"session\"))
        create(path = \"tasks/followup.md\", body = \"# next\\n\")
        send(to = [\"reviewer\"], body = \"t -> review\")
")
        .expect("evaluates");
        assert!(out.refusals.is_empty());
        assert_eq!(
            out.emits,
            vec![
                MwEmit::SetField {
                    path: "tasks/t.md".into(),
                    key: "session".into(),
                    value: "0ecc9d6a".into(),
                },
                MwEmit::Create {
                    path: "tasks/followup.md".into(),
                    body: "# next\n".into(),
                },
                MwEmit::Send {
                    to: vec!["reviewer".into()],
                    body: "t -> review".into(),
                },
            ]
        );
    }

    #[test]
    fn refuse_records_and_requires_the_passing_citation() {
        let out = run("\
def middleware(ctx):
    refuse(message = \"no\", passing = \"rules/mw.md#legal\")
")
        .expect("evaluates");
        assert_eq!(out.refusals.len(), 1);
        assert_eq!(out.refusals[0].passing_scenario, "rules/mw.md#legal");

        // Missing `passing` faults — same law as CHECK's refuse.
        assert!(matches!(
            run("def middleware(ctx):\n    refuse(message = \"no\")\n"),
            Err(CheckError::Runtime { .. })
        ));
    }

    #[test]
    fn ctx_sql_and_ctx_read_reach_the_world_and_bare_spellings_match() {
        let out = run("\
def middleware(ctx):
    rows = ctx.sql(\"SELECT path FROM frontmatter\")
    for row in rows:
        if row[\"n\"] == 2 and row[\"gone\"] == None:
            set_field(path = row[\"path\"], key = \"seen\", value = ctx.read(row[\"path\"])[0:3])
    if sql(\"SELECT 1\") == rows or read(\"agents/a.md\") != None:
        send(to = \"leader\", body = \"both spellings work\")
")
        .expect("evaluates");
        assert_eq!(
            out.emits,
            vec![
                MwEmit::SetField {
                    path: "agents/a.md".into(),
                    key: "seen".into(),
                    value: "---".into(),
                },
                MwEmit::Send {
                    to: vec!["leader".into()],
                    body: "both spellings work".into(),
                },
            ]
        );
    }

    #[test]
    fn a_world_fault_is_a_runtime_fault_never_a_silent_pass() {
        let err = run("def middleware(ctx):\n    ctx.sql(\"boom\")\n").unwrap_err();
        let CheckError::Runtime { reason } = err else {
            panic!("a sql fault is a runtime fault: {err:?}");
        };
        assert!(reason.contains("no backend at this door"), "{reason}");
    }

    #[test]
    fn missing_entry_and_bombs_are_typed() {
        assert!(matches!(run("x = 1\n"), Err(CheckError::MissingEntry)));
        let change = status_change();
        let fields = BTreeMap::new();
        let limits = CheckLimits {
            fuel: 10_000,
            ..CheckLimits::default()
        };
        assert!(matches!(
            run_middleware(
                "def middleware(ctx):\n    for _ in range(10000000):\n        pass\n",
                &MwCtxInput {
                    change: &change,
                    fields: &fields,
                },
                &FakeWorld,
                limits,
            ),
            Err(CheckError::Budget { .. })
        ));
    }

    #[test]
    fn the_ceiling_excludes_check_and_hook_vocabulary() {
        // `intent` is hook vocabulary; reaching it faults at eval.
        assert!(matches!(
            run("def middleware(ctx):\n    intent(action = \"notify\")\n"),
            Err(CheckError::Runtime { .. })
        ));
        // The ceiling globals are exactly standard + the six door names.
        let names: std::collections::HashSet<String> = middleware_globals()
            .names()
            .map(|n| n.as_str().to_owned())
            .collect();
        for granted in ["refuse", "set_field", "create", "send", "sql", "read"] {
            assert!(names.contains(granted), "grants `{granted}`");
        }
        for denied in ["intent", "receipt_addr", "append_section", "open", "now"] {
            assert!(!names.contains(denied), "must not expose `{denied}`");
        }
    }
}
