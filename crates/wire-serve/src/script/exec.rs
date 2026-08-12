//! The in-process script attempt (wire-contract § A.7): entry world in,
//! [`wire::ScriptTrace`] out.
//!
//! The lane's whole shape, in order — the CLI lane's `mrd::script::cmd::run`
//! transposed onto a pinned world:
//!
//! 1. the entry fingerprint IS the pin's — the currency pass already ran at
//!    op entry, so no fingerprint round trip exists to make;
//! 2. the caller's own guard is checked pre-eval (zero reads, nothing armed);
//! 3. evaluation runs against [`EntryWorld`] through [`EngineHost`] — reads
//!    are in-process, the armed overlay serves read-your-own-writes, and the
//!    wall clock binds at every host read;
//! 4. rev threading is ENTRY-GRAIN: the recording licenses each row
//!    (write-follows-read, unchanged), the pinned world supplies the token —
//!    an overlay read's armed rev must never thread, because §4.4 resolves
//!    guards against the pre-batch state and pre-batch = entry world (or the
//!    commit refuses on `if_fingerprint` first);
//! 5. `expect_armed`, then the pre-commit clock, then ONE guarded splice
//!    through [`crate::write::splice`] — the same door, `Origin::Wire`, the
//!    commit CAS unchanged. A panic inside the splice is the in-process
//!    analog of a lost answer: `commit_unknown`, recovery `resync`
//!    (`retry` under dry).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use effects::{
    ArmedEdit, ReadFace, ReadFault, ScriptCtx, ScriptHost, ScriptLimits, ScriptRecording,
};
use serde_json::value::RawValue;
use wire::{DeltaFrame, PlanEdit, Recovery, ScriptBudgets, ScriptTrace, SecFacts, TocFacts};

use super::trace::{CommitLeg, Refusal, assemble, guard_refused};

/// The pinned entry world — what the daemon hands the executor. `Sync`
/// because the kernel evaluates on its own bounded-stack thread and the host
/// (holding this borrow) is moved onto it.
pub trait EntryWorld: Sync {
    /// The document at `path` in the entry world, when the pinned corpus
    /// serves it.
    fn doc(&self, path: &str) -> Option<&model::Document>;
    /// The unserved condition (a non-UTF-8 member), when the pinned corpus
    /// holds one at `path`.
    fn unserved(&self, path: &str) -> Option<&str>;
    /// The entry fingerprint — the pin's own, minted by the op-entry currency
    /// pass.
    fn fingerprint(&self) -> wire::Root;
}

/// One decoded `script` op, ready to execute.
#[derive(Debug)]
pub struct ScriptExec {
    /// The request id, threaded to the splice (the §6.1 receipt renders it).
    pub id: Option<u64>,
    /// The program; the module top level IS the body.
    pub source: String,
    /// The caller's arguments — the inert dict.
    pub args: BTreeMap<String, String>,
    /// Caller-enumerated paths; sorted here (the CLI lane's own posture).
    pub files: Vec<String>,
    /// §9 identity, threaded — never minted.
    pub actor: Option<String>,
    /// §9 time, format-validated at decode — never generated.
    pub now: Option<String>,
    /// The receipt address, riding the commit splice (§6.1).
    pub receipt: Option<wire::ReceiptAddr>,
    /// The caller's own pre-eval guard (v3 spelling `if_fingerprint`).
    pub if_root: Option<wire::Root>,
    /// The rehearsal flag — the splice carries it (§4.4 dry law).
    pub dry: bool,
    /// The armed-set digest gate, checked before the splice is issued.
    pub expect_armed: Option<String>,
    /// Field-by-field budget overrides; absent fields take the defaults.
    pub budgets: Option<ScriptBudgets>,
}

/// What one served attempt produced: the trace, and the committed delta frame
/// for the caller's ring (present exactly when the splice landed live).
#[derive(Debug)]
pub struct ScriptServe {
    /// The § A.7 response body.
    pub trace: ScriptTrace,
    /// The committed frame — the dispatch arm advances its ring with it, the
    /// same leg the `splice` arm drives.
    pub committed: Option<DeltaFrame>,
}

/// The in-process wall-clock default — the same 7s the CLI lane binds
/// (`run-plane.md` §5.3 host policy: existence is contract, value tunable).
const WALL_CLOCK_MS: u64 = 7_000;

/// Serve one script attempt against the pinned entry world.
#[must_use]
pub fn serve_script(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    mints: Option<&receipt::read_mint::ReadMintStore>,
    world: &dyn EntryWorld,
    exec: &ScriptExec,
) -> ScriptServe {
    let entry = world.fingerprint();
    // The caller's own guard, pre-eval: zero reads, nothing armed, no splice.
    if let Some(pinned) = &exec.if_root
        && pinned != &entry
    {
        return ScriptServe {
            trace: guard_refused(entry.0, pinned.0.clone()),
            committed: None,
        };
    }

    let limits = limits_of(exec.budgets.as_ref());
    let wall = Duration::from_millis(
        exec.budgets
            .as_ref()
            .and_then(|b| b.wall_ms)
            .unwrap_or(WALL_CLOCK_MS),
    );
    let deadline = Instant::now() + wall;

    let mut files = exec.files.clone();
    files.sort();
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: exec.args.clone(),
        files,
    };

    let mut eval = {
        let mut host = EngineHost {
            world,
            root,
            actor: exec.actor.clone().unwrap_or_default(),
            deadline,
            armed_path: None,
            armed_plan: Vec::new(),
            overlay: None,
        };
        effects::eval_script(&exec.source, &ctx, limits, &mut host)
    };

    // Failed eval OR zero armed → read-class exit, no splice.
    if eval.outcome.is_err() || eval.armed.is_empty() {
        return ScriptServe {
            trace: assemble(entry.0, &eval, CommitLeg::NotIssued),
            committed: None,
        };
    }

    // Entry-grain rev threading: the recording licenses, the pin supplies.
    eval.armed = thread_entry_tokens(&eval.armed, &eval.recording, world, root);

    // The caller's armed-set expectation — after rev threading (the threaded
    // token is part of what was armed), before anything is issued.
    if let Some(expected) = &exec.expect_armed {
        let actual = super::digest::armed_digest(&super::digest::ArmedRow::of_all(&eval.armed));
        if *expected != actual {
            let leg = CommitLeg::Refused(Refusal::minted(
                Recovery::Fix,
                format!(
                    "expect_armed_mismatch: this run armed {actual}, the caller pinned \
                     {expected}. The armed set is not the one that was authorized, so NO \
                     splice was issued — nothing was sent, nothing landed, no fingerprint \
                     advanced. re-arm: run the arm leg again and gate the set it publishes"
                ),
            ));
            return ScriptServe {
                trace: assemble(entry.0, &eval, leg),
                committed: None,
            };
        }
    }

    // The pre-commit clock (§ Where the budgets bind, layer 3): a run whose
    // clock elapsed during evaluation refuses with nothing issued.
    if Instant::now() > deadline {
        let leg = CommitLeg::Refused(Refusal::minted(
            Recovery::Retry,
            "the script entry's wall clock elapsed before the commit was issued — the armed \
             edits were never sent, nothing landed, and no fingerprint advanced. re-run: the \
             reads that ran cost the budget",
        ));
        return ScriptServe {
            trace: assemble(entry.0, &eval, leg),
            committed: None,
        };
    }

    let (leg, committed) = commit(root, seq, mints, &entry, exec, &eval);
    ScriptServe {
        trace: assemble(entry.0, &eval, leg),
        committed,
    }
}

/// Build the kernel limits from the request's overrides.
fn limits_of(budgets: Option<&ScriptBudgets>) -> ScriptLimits {
    let mut limits = ScriptLimits::default();
    let Some(b) = budgets else { return limits };
    if let Some(fuel) = b.fuel {
        limits.eval.fuel = fuel;
    }
    if let Some(mem) = b.mem {
        limits.eval.mem = mem;
    }
    if let Some(depth) = b.call_depth {
        limits.eval.max_call_depth = usize::try_from(depth).unwrap_or(usize::MAX);
    }
    if let Some(bytes) = b.source_bytes {
        limits.eval.max_source_bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
    }
    if let Some(reads) = b.reads {
        limits.max_reads = usize::try_from(reads).unwrap_or(usize::MAX);
    }
    if let Some(armed) = b.armed {
        limits.max_armed_edits = usize::try_from(armed).unwrap_or(usize::MAX);
    }
    limits
}

// ---------------------------------------------------------------------------
// The entry-world host
// ---------------------------------------------------------------------------

/// The in-process [`ScriptHost`]: reads serve from the pinned entry world at
/// memory speed, the armed overlay serves the program its own writes, and the
/// wall clock binds at every read.
struct EngineHost<'a> {
    world: &'a dyn EntryWorld,
    root: &'a fs::WorkspaceRoot,
    actor: String,
    deadline: Instant,
    /// The one armed content path, set by the first `armed()` observation.
    armed_path: Option<String>,
    /// The armed plan so far, in arm order — the overlay's input.
    armed_plan: Vec<PlanEdit>,
    /// The composed overlay, cached with the plan length it was composed at.
    overlay: Option<(usize, model::Document)>,
}

impl EngineHost<'_> {
    /// The wall clock, checked before every host read (the in-process
    /// collapse of the CLI lane's layers 1–2 — wire-contract § A.7).
    fn within_deadline(&self, fault: &dyn Fn(String) -> ReadFault) -> Result<(), ReadFault> {
        if Instant::now() > self.deadline {
            return Err(fault(
                "the script entry's wall clock elapsed before this read — the attempt aborts \
                 whole: any edits the script armed are never sent, and nothing commits"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Resolve the ENTRY document for `path` — the pinned corpus first, the
    /// unserved condition as a fault, then the §12.1 addressability fallback
    /// (a real file under the root but outside the hash domain serves from a
    /// single-file disk load — outside the pin and outside the stand-still
    /// guarantee exactly as the fingerprint never covered it).
    fn entry_doc(
        &self,
        path: &str,
        fault: &dyn Fn(String) -> ReadFault,
    ) -> Result<std::borrow::Cow<'_, model::Document>, ReadFault> {
        if let Some(doc) = self.world.doc(path) {
            return Ok(std::borrow::Cow::Borrowed(doc));
        }
        if let Some(condition) = self.world.unserved(path) {
            return Err(fault(format!(
                "the corpus cannot serve this member: {condition}"
            )));
        }
        let wire_path = wire::Path(path.to_owned());
        crate::load_doc(self.root, &wire_path)
            .map(std::borrow::Cow::Owned)
            .map_err(|e| fault(super::faces::refusal_text(&e)))
    }

    /// Serve `f` the document `path` reads as — the armed overlay for the one
    /// armed content path (read-your-own-writes), the entry document
    /// otherwise. The overlay is the SPLICE'S OWN candidate: the armed plan
    /// lowered and validated against the entry document through the same
    /// intake the commit uses, so the preview is the commit's bytes.
    fn with_doc<R>(
        &mut self,
        path: &str,
        fault: &dyn Fn(String) -> ReadFault,
        f: impl FnOnce(&model::Document) -> Result<R, ReadFault>,
    ) -> Result<R, ReadFault> {
        let overlaid = self.armed_path.as_deref() == Some(path) && !self.armed_plan.is_empty();
        if !overlaid {
            let doc = self.entry_doc(path, fault)?;
            return f(&doc);
        }
        if self
            .overlay
            .as_ref()
            .is_none_or(|(at, _)| *at != self.armed_plan.len())
        {
            let base = self.entry_doc(path, fault)?;
            let wire_path = wire::Path(path.to_owned());
            let candidate = crate::write::overlay_candidate(&base, &wire_path, &self.armed_plan)
                .map_err(|e| {
                    fault(format!(
                        "the armed overlay cannot compose — the program's own armed edits do \
                         not apply to the entry world, and the commit would refuse the same \
                         way: {}",
                        super::faces::refusal_text(&e)
                    ))
                })?;
            self.overlay = Some((self.armed_plan.len(), candidate));
        }
        let (_, doc) = self.overlay.as_ref().expect("composed just above");
        f(doc)
    }
}

impl ScriptHost for EngineHost<'_> {
    fn toc(&mut self, path: &str) -> Result<TocFacts, ReadFault> {
        let owned = path.to_owned();
        let fault = move |reason: String| ReadFault {
            path: owned.clone(),
            section: None,
            reason,
        };
        self.within_deadline(&fault)?;
        let ambient = self.world.fingerprint();
        self.with_doc(path, &fault, |doc| {
            super::faces::toc_facts(doc, &wire::Path(path.to_owned()), &ambient).map_err(&fault)
        })
    }

    fn cat(&mut self, path: &str, section: &str) -> Result<SecFacts, ReadFault> {
        let owned_path = path.to_owned();
        let owned_section = section.to_owned();
        let fault = move |reason: String| ReadFault {
            path: owned_path.clone(),
            section: Some(owned_section.clone()),
            reason,
        };
        self.within_deadline(&fault)?;
        self.with_doc(path, &fault, |doc| {
            super::faces::sec_facts(doc, section).map_err(&fault)
        })
    }

    fn actor(&self) -> &str {
        &self.actor
    }

    fn armed(&mut self, edit: &ArmedEdit) {
        // First arm names the one content path (the kernel's own
        // single-write-file law guarantees every later arm agrees).
        if self.armed_path.is_none() {
            self.armed_path = Some(edit.path.clone());
        }
        self.armed_plan.push(edit.edit.clone());
    }
}

// ---------------------------------------------------------------------------
// Entry-grain rev threading
// ---------------------------------------------------------------------------

/// Thread each armed row's CAS token: the RECORDING licenses the row
/// (write-follows-read, the CLI lane's own grain — a `props` write needs the
/// file read, an `append` needs its section read or the toc row naming it),
/// and the ENTRY WORLD supplies the token. An already-set `rev` is never
/// overwritten; an unlicensed row keeps `rev: None` and meets the engine's
/// own `guard_required` refusal.
fn thread_entry_tokens(
    armed: &[ArmedEdit],
    recording: &ScriptRecording,
    world: &dyn EntryWorld,
    root: &fs::WorkspaceRoot,
) -> Vec<ArmedEdit> {
    armed
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            match &mut arm.edit {
                PlanEdit::SetProperty {
                    rev: rev @ None, ..
                } => {
                    if file_read_licensed(recording, &arm.path) {
                        *rev = entry_file_rev(world, root, &arm.path);
                    }
                }
                PlanEdit::Append {
                    hpath,
                    rev: rev @ None,
                    ..
                } if section_read_licensed(recording, &arm.path, hpath) => {
                    *rev = entry_section_rev(world, root, &arm.path, hpath);
                }
                _ => {}
            }
            arm
        })
        .collect()
}

/// Did the attempt read `path`'s toc face? (The file-grain license a
/// `set_property` row needs.)
fn file_read_licensed(recording: &ScriptRecording, path: &str) -> bool {
    recording
        .reads
        .iter()
        .any(|read| read.path == path && matches!(read.face, ReadFace::Toc(_)))
}

/// Did the attempt read the section an `append` lands in? A section read
/// addressing it, or a toc read whose rows name it — the CLI lane's own scan.
fn section_read_licensed(
    recording: &ScriptRecording,
    path: &str,
    hpath: &[wire::HpathSeg],
) -> bool {
    recording.reads.iter().any(|read| {
        if read.path != path {
            return false;
        }
        match &read.face {
            ReadFace::Section(_) => read.section.as_deref().is_some_and(|s| addresses(s, hpath)),
            ReadFace::Toc(facts) => facts
                .toc
                .iter()
                .any(|entry| addresses(&entry.section, hpath)),
        }
    })
}

/// The target's file rev in the ENTRY world (pinned corpus, or the §12.1
/// disk fallback for an out-of-domain real file).
fn entry_file_rev(world: &dyn EntryWorld, root: &fs::WorkspaceRoot, path: &str) -> Option<String> {
    if let Some(doc) = world.doc(path) {
        return Some(doc.root.node_rev.0.clone());
    }
    crate::load_doc(root, &wire::Path(path.to_owned()))
        .ok()
        .map(|doc| doc.root.node_rev.0.clone())
}

/// The addressed section's node rev in the ENTRY world — the toc rows scanned
/// by the same address matcher the license used.
fn entry_section_rev(
    world: &dyn EntryWorld,
    root: &fs::WorkspaceRoot,
    path: &str,
    hpath: &[wire::HpathSeg],
) -> Option<String> {
    let row_rev = |doc: &model::Document| {
        let wire::ResponseBody::Toc { nodes, .. } =
            crate::read::toc(doc, &wire::Path(path.to_owned()), &world.fingerprint())
        else {
            return None;
        };
        nodes.iter().find_map(|node| {
            let node_hpath = node.hpath.as_ref()?;
            hpaths_agree(node_hpath, hpath).then(|| node.node_rev.0.clone())
        })
    };
    if let Some(doc) = world.doc(path) {
        return row_rev(doc);
    }
    crate::load_doc(root, &wire::Path(path.to_owned()))
        .ok()
        .and_then(|doc| row_rev(&doc))
}

/// Does the recorded section spelling address the same node as the armed
/// row's `hpath`? Both sides go through [`wire::ReadSel::parse`] — the one
/// human-string→selector door (the CLI lane's own matcher, ported verbatim).
fn addresses(recorded: &str, hpath: &[wire::HpathSeg]) -> bool {
    let wire::ReadSel::Hpath { hpath: parsed } = wire::ReadSel::parse(recorded) else {
        return false;
    };
    hpaths_agree(&parsed, hpath)
}

/// Segment-text agreement with empty segments filtered on both sides —
/// `ReadSel::parse("/A")` yields `["", "A"]` and the arm filters them, so the
/// comparison filters them on both sides or one leading slash decides a CAS
/// token. Occurrence indexes are deliberately not compared (the CLI lane's
/// own posture).
fn hpaths_agree(a: &[wire::HpathSeg], b: &[wire::HpathSeg]) -> bool {
    let mut a = a.iter().map(|seg| seg.h.as_str()).filter(|h| !h.is_empty());
    let mut b = b.iter().map(|seg| seg.h.as_str()).filter(|h| !h.is_empty());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (x, y) if x != y => return false,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The commit leg
// ---------------------------------------------------------------------------

/// ONE guarded splice through the write choke-point, `Origin::Wire` — the
/// § A.7 lane is a wire-origin write, so the fingerprint-or-force demand and
/// the write-follows-read refusal bind exactly as they do over the socket.
fn commit(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    mints: Option<&receipt::read_mint::ReadMintStore>,
    entry: &wire::Root,
    exec: &ScriptExec,
    eval: &effects::ScriptEval,
) -> (CommitLeg, Option<DeltaFrame>) {
    let paths = eval.content_paths();
    let [path] = paths.as_slice() else {
        // Unreachable by construction: a second content path refuses at arm
        // time (`multi_file_write_set`). It still SPEAKS: nothing was sent,
        // so the class is `fix` and the workspace guarantee is a fact.
        return (
            CommitLeg::Refused(Refusal::minted(
                Recovery::Fix,
                format!(
                    "the armed set writes {} content paths; one script commits to ONE file. NO \
                     splice was issued — nothing was sent, nothing landed, no fingerprint \
                     advanced. fix: arm one content path",
                    paths.len()
                ),
            )),
            None,
        );
    };

    let args = crate::write::SpliceArgs {
        id: exec.id,
        origin: crate::guard::Origin::Wire,
        path: wire::Path(path.clone()),
        actor: exec.actor.clone(),
        now: exec.now.clone(),
        receipt: exec.receipt.clone(),
        if_root: Some(entry.clone()),
        dry: exec.dry,
        force: false,
        edits: Vec::new(),
        plan_edits: eval.armed.iter().map(|armed| armed.edit.clone()).collect(),
        pin: None,
    };

    // The splice is a function call here, so a lost answer cannot happen —
    // except as a panic mid-splice, which is the same indeterminacy: caught,
    // spoken as `commit_unknown`, never an unwind through the daemon.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::write::splice(root, seq, &args, &[], mints)
    }));
    match outcome {
        Ok(Ok(out)) => {
            let bytes = v3_body(&out.body);
            if exec.dry {
                (CommitLeg::Rehearsal(bytes), None)
            } else {
                (CommitLeg::Response(bytes), out.committed)
            }
        }
        Ok(Err(error)) => (classify_refusal(&error), None),
        Err(_panic) => (CommitLeg::Unknown(lost_commit(exec.dry)), None),
    }
}

/// The engine-minted refusal for a commit whose outcome is NOT KNOWN — the
/// in-process analog of a lost answer (the splice panicked mid-flight). The
/// class splits on `dry` exactly as the CLI lane's `lost_answer` does.
fn lost_commit(dry: bool) -> Refusal {
    if dry {
        return Refusal::minted(
            Recovery::Retry,
            "the commit leg panicked mid-splice. This was a DRY run — it rehearses everything \
             except disk, so nothing could have been committed and the workspace is unchanged. \
             retry: re-run the same script, a rehearsal writes nothing",
        );
    }
    Refusal::minted(
        Recovery::Resync,
        "the commit leg panicked mid-splice. The splice was ISSUED, so whether the workspace \
         carries this run is UNKNOWN. resync: re-read and re-plan, never resend, because a \
         resend writes twice",
    )
}

/// Classify a typed splice refusal into the CLI lane's legs: the world-grain
/// guard failing is the conflict leg; everything else is a refusal carrying
/// the engine's own words. Both embed/render in v3 vocabulary through the ONE
/// projection door (`crate::rev`), so the trace's bytes match what a v3
/// socket client would have seen.
fn classify_refusal(error: &wire::ErrorBody) -> CommitLeg {
    let projected = v3_error(error);
    let code = projected
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if code.as_deref() == Some("fingerprint_mismatch") {
        let bytes = RawValue::from_string(projected.to_string())
            .expect("a serde_json Value renders as valid JSON");
        return CommitLeg::Conflict(bytes);
    }
    let recovery = projected
        .get("recovery")
        .and_then(serde_json::Value::as_str)
        .and_then(|r| {
            serde_json::from_value::<Recovery>(serde_json::Value::String(r.to_owned())).ok()
        });
    let message = projected
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let reason = match &code {
        Some(code) if message.is_empty() => code.clone(),
        Some(code) => format!("{code}: {message}"),
        None => message.to_owned(),
    };
    CommitLeg::Refused(Refusal {
        code,
        recovery,
        reason,
    })
}

/// A splice response body in v3 vocabulary, as raw bytes — the ONE projection
/// door applied to a constructed frame, so no second rename table exists.
fn v3_body(body: &wire::ResponseBody) -> Box<RawValue> {
    let mut frame = serde_json::json!({
        "ok": true,
        "body": serde_json::to_value(body).expect("wire bodies serialize"),
    });
    crate::rev::project_response(&mut frame);
    RawValue::from_string(frame["body"].to_string()).expect("a projected frame renders as JSON")
}

/// A splice error body in v3 vocabulary, as a JSON value.
fn v3_error(error: &wire::ErrorBody) -> serde_json::Value {
    let mut frame = serde_json::json!({
        "error": serde_json::to_value(error).expect("wire errors serialize"),
    });
    crate::rev::project_response(&mut frame);
    frame["error"].take()
}
