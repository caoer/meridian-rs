//! § A.7 `script` — the in-process lane of the run plane's script entry
//! (phase-2 script-plane ruling, 2026-08-12; `docs/wire-contract.md` § A.7,
//! `docs/run-plane.md` § The script entry, the entry-world amendment).
//!
//! One frame carries the whole program. The daemon runs the currency pass
//! ONCE at entry, pins the entry world (an [`Arc`] clone of the resident
//! engine — a rebuild swaps the map entry while the held clone keeps the
//! entry generation alive for exactly this attempt), evaluates the program
//! in-process under the kernel's own containment, threads each armed row's
//! CAS token from the ENTRY world, and commits through the one write
//! choke-point with `if_fingerprint` = the entry fingerprint. The trace is
//! the response body.
//!
//! Laws held here, each pinned by a test in this module or in
//! `tests/script_op.rs`:
//!
//! - **Entry world**: reads serve the pinned entry state — foreign mid-program
//!   disk changes are invisible; the commit's §5.1 guard runs against the
//!   LIVE world unchanged, so a moved world refuses and nothing lands.
//! - **Read-your-own-writes**: a read of a target the program itself armed
//!   serves the ARMED content and that content's own rev.
//! - **Entry-rev threading**: the license is the recording's, the value is
//!   the entry world's — an overlay rev is never a CAS token.
//! - **Containment**: the kernel's fuel/mem/depth/source ceilings and
//!   `catch_unwind` (both inside `effects::eval_script`); the wall clock
//!   binds at entry, at every read builtin, and pre-commit.
//! - **Not the banned snapshot**: the pin is attempt-scoped; nothing survives
//!   the answer (the engines map still holds ONE generation).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use effects::trace::{CommitLeg, Refusal, ScriptTrace};
use effects::{
    ArmedEdit, ReadFault, ScriptCtx, ScriptHost, ScriptLimits, ScriptRecording, SecFacts,
    TocEntry, TocFacts, hpath_addresses,
};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use wire::{ErrorBody, ErrorCode, PlanEdit, ReadSel, Recovery, ResponseBody};

use crate::engine::WorkspaceEngine;
use crate::registry::Registry;
use wire_serve::rev::Rev;

/// Serve one `script` frame end to end, answering the full NDJSON line.
///
/// Routed from `handle_line` BEFORE the generic `serve_wire` path because the
/// success body is the run-plane `ScriptTrace`, not a [`ResponseBody`]
/// variant — the trace embeds the §4.4 splice response verbatim, and one
/// commit-fact shape means the wire types stay untouched. Error frames stay
/// typed and leave through the ordinary v2/v3 renderer.
pub(crate) fn serve_line(
    registry: &Registry,
    attached: Option<&Path>,
    obj: &Map<String, Value>,
    rev: Rev,
) -> String {
    let id = obj.get("id").and_then(Value::as_u64);
    let started = Instant::now();
    // Strict decode first (rev-agnostic), then the v3 gate — the read/create
    // dispatch order, so a malformed frame teaches its field wall on any rev.
    let op = match wire_serve::decode::decode(obj, rev) {
        Ok(op) => op,
        Err(error) => return error_line(id, *error, rev),
    };
    if rev != Rev::V3 {
        return error_line(id, ErrorBody::new(ErrorCode::UnknownOp), rev);
    }
    let Some(ws) = attached else {
        return error_line(
            id,
            *wire_serve::bad_request("no workspace bound — send `hello` with a `workspace` first"),
            rev,
        );
    };
    let wire::Op::Script {
        source,
        args,
        files,
        actor,
        now,
        receipt,
        if_root,
        dry,
        expect_armed,
    } = op
    else {
        // decode() maps the "script" tag to Op::Script only; any other arm
        // here is a routing defect, answered loud rather than misdispatched.
        return error_line(id, ErrorBody::new(ErrorCode::Internal), rev);
    };
    let request = ScriptArgs {
        id,
        source,
        args,
        files,
        actor,
        now,
        receipt,
        if_root,
        dry: dry.unwrap_or(false),
        expect_armed,
    };
    match serve(registry, ws, request) {
        Ok(trace) => {
            let body = serde_json::to_value(&trace).expect("a ScriptTrace serializes");
            let mut frame = json!({"id": id, "ok": true, "body": body});
            let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            wire_serve::rev::attach_meta(&mut frame, duration_us);
            let mut line = serde_json::to_string(&frame).expect("a script frame serializes");
            line.push('\n');
            line
        }
        Err(error) => error_line(id, *error, rev),
    }
}

/// One typed error frame, rendered per negotiated rev (the `wire_line` path).
fn error_line(id: Option<u64>, error: ErrorBody, rev: Rev) -> String {
    crate::server::wire_line(
        &wire::Response {
            id,
            ok: false,
            payload: wire::ResponsePayload::Error { error },
        },
        rev,
        None,
    )
}

/// The decoded request, owned.
struct ScriptArgs {
    id: Option<u64>,
    source: String,
    args: std::collections::BTreeMap<String, String>,
    files: Vec<String>,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<wire::ReceiptAddr>,
    if_root: Option<wire::Root>,
    dry: bool,
    expect_armed: Option<String>,
}

/// The attempt: entry pass → pin → eval → thread → gate → commit → trace.
///
/// # Errors
/// A §8 frame ONLY for what never reached the entry: the entry pass itself
/// failing (env class), or the reaper winning the warm→pin race (retry).
/// Once an entry fingerprint exists, every exit is a trace.
fn serve(
    registry: &Registry,
    ws: &Path,
    request: ScriptArgs,
) -> Result<ScriptTrace, Box<ErrorBody>> {
    // ONE currency pass, at entry (Law A-3c scope unchanged, moved in time).
    registry.warm_or_build(ws).map_err(|e| {
        let mut error = ErrorBody::new(ErrorCode::IoError);
        error.message = Some(format!("the entry pass could not prove the corpus: {e}"));
        Box::new(error)
    })?;
    // Pin the entry world. A reaper that won the warm→pin race is the same
    // transient the read path names (`corpus_race`, retry).
    let Some(world) = registry.engine_snapshot(ws) else {
        let mut error = ErrorBody::new(ErrorCode::CorpusRace);
        error.message = Some(
            "the warm engine was reaped between the entry pass and the pin — transient; retry"
                .to_owned(),
        );
        return Err(Box::new(error));
    };
    let entry = world.at_fingerprint.0.clone();

    // The caller's own pre-eval guard: zero evaluation, zero reads.
    if let Some(pinned) = &request.if_root
        && pinned.0 != entry
    {
        return Ok(ScriptTrace::guard_refused(entry, pinned.0.clone()));
    }

    let deadline = Instant::now() + effects::DEFAULT_WALL_CLOCK;
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: request.args.clone(),
        files: request.files.clone(),
    };
    let mut eval = {
        let mut host = EntryWorldHost {
            world: Arc::clone(&world),
            root: wire::Root(entry.clone()),
            deadline,
            actor: request.actor.clone().unwrap_or_default(),
            overlay: None,
        };
        effects::eval_script(&request.source, &ctx, ScriptLimits::default(), &mut host)
    };

    // A failed evaluation never commits; zero armed is the read-class exit.
    if eval.outcome.is_err() || eval.armed.is_empty() {
        return Ok(ScriptTrace::assemble(entry, &eval, CommitLeg::NotIssued));
    }

    // Entry-rev threading: the license is the recording's, the value is the
    // entry world's (run-plane § the entry-rev law).
    eval.armed = thread_entry(&eval.armed, &eval.recording, &world, &wire::Root(entry.clone()));

    // The pre-splice armed-set gate — after threading, before anything is
    // issued; same wording as the CLI lane, one law two doors.
    if let Some(expected) = &request.expect_armed {
        let actual = effects::digest::armed_digest(&effects::digest::ArmedRow::of_all(&eval.armed));
        if *expected != actual {
            return Ok(ScriptTrace::assemble(
                entry,
                &eval,
                CommitLeg::Refused(Refusal::minted(
                    Recovery::Fix,
                    format!(
                        "expect_armed_mismatch: this run armed {actual}, the caller pinned \
                         {expected}. The armed set is not the one that was authorized, so NO \
                         splice was issued — nothing was sent, nothing landed, no fingerprint \
                         advanced. re-arm: run the arm leg again and gate the set it publishes"
                    ),
                )),
            ));
        }
    }

    // Wall site 3: pre-commit. Nothing issued on a lapsed clock.
    let leg = if Instant::now() > deadline {
        CommitLeg::Refused(Refusal::minted(
            Recovery::Retry,
            "the script entry's wall clock elapsed before the commit was issued — the armed \
             edits were never sent, nothing landed, and no fingerprint advanced. re-run: the \
             reads that ran cost the budget",
        ))
    } else {
        commit(registry, ws, &request, &eval, &entry)
    };
    Ok(ScriptTrace::assemble(entry, &eval, leg))
}

/// The one guarded splice, issued daemon-side through the same choke-point
/// every wire splice takes — `Origin::Wire`, the ring advanced on a real
/// commit (this lane mints Deltas; the CLI put lane's row-12 gap does not
/// extend here).
fn commit(
    registry: &Registry,
    ws: &Path,
    request: &ScriptArgs,
    eval: &effects::ScriptEval,
    entry: &str,
) -> CommitLeg {
    let paths = eval.content_paths();
    let [path] = paths.as_slice() else {
        // Unreachable by construction — the arm-time law refuses first
        // (`multi_file_write_set`); it still SPEAKS, the CLI lane's own words.
        return CommitLeg::Refused(Refusal::minted(
            Recovery::Fix,
            format!(
                "the armed set writes {} content paths; one script commits to ONE file. NO \
                 splice was issued — nothing was sent, nothing landed, no fingerprint advanced. \
                 fix: arm one content path",
                paths.len()
            ),
        ));
    };
    let args = wire_serve::write::SpliceArgs {
        id: request.id,
        path: wire::Path(path.clone()),
        origin: wire_serve::guard::Origin::Wire,
        actor: request.actor.clone(),
        now: request.now.clone(),
        receipt: request.receipt.clone(),
        if_root: Some(wire::Root(entry.to_owned())),
        dry: request.dry,
        force: false,
        edits: Vec::new(),
        plan_edits: eval.armed.iter().map(|armed| armed.edit.clone()).collect(),
        pin: None,
    };
    // H1 order: the mint store and ring handles are taken outside any engine
    // borrow (none is held here — the entry world is an Arc, not a lock).
    let mints = registry.read_mints(ws);
    let ring = registry.ring(ws);
    let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
    match wire_serve::write::splice(&ws_root, Some(&*ring), &args, &[], Some(&mints)) {
        Ok(out) => {
            if let Some(frame) = out.committed {
                ring.advance(frame);
            }
            let raw = v3_body_raw(&out.body);
            if request.dry {
                CommitLeg::Rehearsal(raw)
            } else {
                CommitLeg::Response(raw)
            }
        }
        Err(error) => {
            let raw = v3_error_raw(&error);
            if matches!(error.code, ErrorCode::RootMismatch) {
                CommitLeg::Conflict(raw)
            } else {
                CommitLeg::Refused(refusal_of(&raw))
            }
        }
    }
}

/// The §4.4 splice response body, serialized in the v3 vocabulary — the SAME
/// bytes a v3 wire client would have received, so the two lanes' traces embed
/// one spelling. The projection is the one implementation (`rev.rs`), reached
/// through a throwaway frame.
fn v3_body_raw(body: &ResponseBody) -> Box<RawValue> {
    let mut frame = json!({
        "id": Value::Null,
        "ok": true,
        "body": serde_json::to_value(body).expect("a splice body serializes"),
    });
    wire_serve::rev::project_response(&mut frame);
    RawValue::from_string(frame["body"].to_string()).expect("projected body is JSON")
}

/// A §8 error body in the v3 vocabulary, as raw bytes for the conflict leg.
fn v3_error_raw(error: &ErrorBody) -> Box<RawValue> {
    let mut frame = json!({
        "id": Value::Null,
        "ok": false,
        "error": serde_json::to_value(error).expect("an error body serializes"),
    });
    wire_serve::rev::project_response(&mut frame);
    RawValue::from_string(frame["error"].to_string()).expect("projected error is JSON")
}

/// The wire's refusal triple off the projected error bytes — the same read
/// the CLI lane performs on the frame it receives, so the two lanes classify
/// one refusal one way.
fn refusal_of(error: &RawValue) -> Refusal {
    let parsed: Option<Value> = serde_json::from_str(error.get()).ok();
    let field = |name: &str| -> Option<String> {
        parsed
            .as_ref()
            .and_then(|e| e.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let code = field("code");
    let recovery = field("recovery")
        .and_then(|class| serde_json::from_value::<Recovery>(Value::String(class)).ok())
        .or_else(|| {
            code.as_ref().and_then(|code| {
                serde_json::from_value::<ErrorCode>(Value::String(code.clone()))
                    .ok()
                    .map(ErrorCode::recovery)
            })
        });
    let reason = field("message").unwrap_or_else(|| error.get().to_owned());
    Refusal {
        code,
        recovery,
        reason,
    }
}

/// Thread each armed row's CAS token: LICENSE from the recording (any read of
/// the row's grain this attempt, overlay-served included — the
/// write-follows-read law binds per attempt), VALUE from the entry world (the
/// pre-batch state the §4.4 guards resolve against; an overlay rev names a
/// state no disk ever carried and is never a token). An unlicensed row keeps
/// `rev: None` and meets the engine's own `guard_required` — the same refusal
/// the CLI lane's unread target meets.
fn thread_entry(
    armed: &[ArmedEdit],
    recording: &ScriptRecording,
    world: &WorkspaceEngine,
    root: &wire::Root,
) -> Vec<ArmedEdit> {
    armed
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            match &mut arm.edit {
                PlanEdit::SetProperty { rev: rev @ None, .. } => {
                    let licensed = recording
                        .reads
                        .iter()
                        .any(|r| r.path == arm.path && r.section.is_none());
                    if licensed {
                        *rev = entry_toc(world, root, &arm.path).map(|facts| facts.rev);
                    }
                }
                PlanEdit::Append {
                    hpath,
                    rev: rev @ None,
                    ..
                } => {
                    let licensed = recording.reads.iter().any(|r| {
                        if r.path != arm.path {
                            return false;
                        }
                        match (&r.section, &r.face) {
                            (Some(recorded), effects::ReadFace::Section(_)) => {
                                hpath_addresses(recorded, hpath)
                            }
                            (None, effects::ReadFace::Toc(facts)) => facts
                                .toc
                                .iter()
                                .any(|entry| hpath_addresses(&entry.section, hpath)),
                            _ => false,
                        }
                    });
                    if licensed {
                        *rev = entry_toc(world, root, &arm.path).and_then(|facts| {
                            facts
                                .toc
                                .iter()
                                .find(|entry| hpath_addresses(&entry.section, hpath))
                                .map(|entry| entry.rev.clone())
                        });
                    }
                }
                _ => {}
            }
            arm
        })
        .collect()
}

/// The §4.1 toc face of `path` at the ENTRY state — the one builder threading
/// and the host share, so a threaded token equals what an entry read served,
/// by construction.
fn entry_toc(world: &WorkspaceEngine, root: &wire::Root, path: &str) -> Option<TocFacts> {
    world
        .docs
        .get(path)
        .map(|doc| toc_facts_of(doc, path, root))
}

/// Build the script-face [`TocFacts`] from one document — the daemon-side twin
/// of the wire client's composition, built from the same serve arms
/// (`wire_serve::read::{toc, read_props}` equivalents) so the face bytes agree
/// across lanes: rev = the served `file_rev`, `fm` decoded per § A.6, toc rows
/// as `/`-joined hpaths (what `ReadSel::parse` splits again) or `^anchor`,
/// words = the composed read's own count.
fn toc_facts_of(doc: &model::Document, path: &str, root: &wire::Root) -> TocFacts {
    let body = wire_serve::read::toc(doc, &wire::Path(path.to_owned()), root);
    let ResponseBody::Toc {
        file_rev, nodes, ..
    } = body
    else {
        unreachable!("wire_serve::read::toc answers a Toc body");
    };
    let toc = nodes
        .iter()
        .filter_map(|node| {
            let anchor = node.anchor.as_ref().map(|id| format!("^{id}"));
            let section = match &node.hpath {
                Some(hpath) => hpath
                    .iter()
                    .map(|seg| seg.h.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
                None => anchor.clone()?,
            };
            Some(TocEntry {
                section,
                anchor,
                rev: node.node_rev.0.clone(),
            })
        })
        .collect();
    let fm = wire_serve::read::props_of(doc)
        .into_iter()
        .map(|prop| (prop.key, prop.value))
        .collect();
    TocFacts {
        rev: file_rev.0,
        fm,
        toc,
        words: wire_serve::read::words_of(doc),
    }
}

/// The entry world plus the program's own armed overlay — the § A.7 read
/// seam. Serves at memory speed: no locks, no passes, no disk.
struct EntryWorldHost {
    world: Arc<WorkspaceEngine>,
    root: wire::Root,
    deadline: Instant,
    actor: String,
    /// The overlay document for the ONE armed content path, cached by armed
    /// count (the armed list is append-only and single-path by the arm law).
    overlay: Option<(String, usize, model::Document)>,
}

impl EntryWorldHost {
    /// Wall site 2: every read builtin checks the clock before serving.
    fn within_deadline(&self, path: &str, section: Option<&str>) -> Result<(), ReadFault> {
        if Instant::now() > self.deadline {
            return Err(ReadFault {
                path: path.to_owned(),
                section: section.map(ToOwned::to_owned),
                reason: "the script entry's wall clock elapsed — the attempt is over budget; \
                         nothing was committed"
                    .to_owned(),
            });
        }
        Ok(())
    }

    /// The document a read of `path` serves: the entry doc, or — when the
    /// program itself armed edits on `path` — the entry doc with those edits
    /// applied, in arm order (read-your-own-writes). What you read is what is
    /// hashed: the overlay document's revs are minted from the overlay bytes.
    fn doc_for(
        &mut self,
        path: &str,
        section: Option<&str>,
        armed: &[ArmedEdit],
    ) -> Result<&model::Document, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: section.map(ToOwned::to_owned),
            reason,
        };
        let rows: Vec<&ArmedEdit> = armed.iter().filter(|a| a.path == path).collect();
        if rows.is_empty() {
            return self.world.docs.get(path).ok_or_else(|| {
                fault(format!(
                    "no such file in the entry world: {path} — absent at entry, or outside \
                     the hash domain the entry pass proved"
                ))
            });
        }
        let cached = self
            .overlay
            .as_ref()
            .is_some_and(|(p, count, _)| p == path && *count == rows.len());
        if !cached {
            let base = self.world.docs.get(path).ok_or_else(|| {
                fault(format!(
                    "put() armed edits on {path}, but the entry world holds no such file — \
                     the commit would refuse file_not_found"
                ))
            })?;
            let doc = overlay_doc(base, path, &rows).map_err(|reason| fault(reason))?;
            self.overlay = Some((path.to_owned(), rows.len(), doc));
        }
        Ok(&self.overlay.as_ref().expect("just cached").2)
    }
}

/// Apply the program's own armed rows to the entry document — the dry
/// pipeline in memory: lower → validate → candidate, one reparse, no disk.
/// A row set that cannot apply to the entry state has no overlay content to
/// serve, so the read refuses naming the first refusing law (the commit
/// would meet the same refusal from the engine's own mouth).
fn overlay_doc(
    base: &model::Document,
    path: &str,
    rows: &[&ArmedEdit],
) -> Result<model::Document, String> {
    let plan: Vec<PlanEdit> = rows.iter().map(|row| row.edit.clone()).collect();
    wire_serve::write::overlay_candidate(base, &wire::Path(path.to_owned()), &plan).map_err(
        |error| {
            format!(
                "the armed edits cannot apply to the entry state — the commit would meet                  this same refusal: {}",
                error
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", error.code))
            )
        },
    )
}

impl ScriptHost for EntryWorldHost {
    fn toc(&mut self, path: &str, armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        self.within_deadline(path, None)?;
        let root = self.root.clone();
        let doc = self.doc_for(path, None, armed)?;
        Ok(toc_facts_of(doc, path, &root))
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        self.within_deadline(path, Some(section))?;
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.to_owned()),
            reason,
        };
        // The one human-string→selector door, the CLI lane's own words for the
        // dewey arm.
        let sec = match ReadSel::parse(section) {
            ReadSel::Hpath { hpath } => wire::SecRef::Hpath { hpath },
            ReadSel::Anchor { anchor } => wire::SecRef::Anchor { anchor },
            ReadSel::Dewey { .. } => {
                return Err(fault(
                    "a dewey ordinal addresses a row of a table you are holding, not a \
                     document — pass the heading path or a ^anchor"
                        .to_owned(),
                ));
            }
        };
        let doc = self.doc_for(path, Some(section), armed)?;
        match wire_serve::read::cat(doc, Some(sec)) {
            Ok(ResponseBody::Cat {
                content, node_rev, ..
            }) => Ok(SecFacts {
                text: content,
                rev: node_rev.0,
            }),
            Ok(_) => unreachable!("wire_serve::read::cat answers a Cat body"),
            Err(error) => Err(fault(
                error
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", error.code)),
            )),
        }
    }

    fn actor(&self) -> &str {
        &self.actor
    }
}

#[cfg(test)]
mod tests {
    //! The module-grain pins the wire harness cannot reach deterministically:
    //! mid-program foreign-edit invisibility (no interleave point exists over
    //! one socket), entry-rev-vs-overlay-rev threading, and the read-site
    //! wall clock. The wire-grain laws live in `tests/script_op.rs`.

    use super::*;
    use crate::state::StateStore;
    use effects::{ReadFace, ReadPosition, ReadRecord, ScriptRecording};
    use std::fs::{create_dir_all, write};
    use std::path::PathBuf;
    use std::time::Duration;

    const DOC: &str = "---\nstatus: open\n---\n# Alpha\n\none two three\n";

    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    fn seeded_ws(home: &Path) -> PathBuf {
        let ws = home.join("ws");
        create_dir_all(&ws).unwrap();
        write(ws.join("doc.md"), DOC).unwrap();
        std::fs::canonicalize(&ws).unwrap()
    }

    fn pinned_world(
        registry: &Registry,
        ws: &Path,
    ) -> (Arc<WorkspaceEngine>, wire::Root) {
        registry.warm_or_build(ws).expect("entry pass");
        let world = registry.engine_snapshot(ws).expect("pinned world");
        let root = wire::Root(world.at_fingerprint.0.clone());
        (world, root)
    }

    fn host_of(world: &Arc<WorkspaceEngine>, root: &wire::Root, deadline: Instant) -> EntryWorldHost {
        EntryWorldHost {
            world: Arc::clone(world),
            root: root.clone(),
            deadline,
            actor: String::new(),
            overlay: None,
        }
    }

    /// The entry-world law, both halves: a foreign disk edit AFTER entry is
    /// invisible to the program's reads (they serve the pinned entry state),
    /// and the commit's §5.1 guard runs against the LIVE world — the same
    /// foreign edit refuses the splice, so nothing lands on a moved world.
    #[test]
    fn a_foreign_edit_after_entry_is_invisible_to_reads_and_refuses_the_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);

        // The foreign edit lands on disk between entry and the first read.
        write(ws.join("doc.md"), DOC.replace("open", "parked")).unwrap();

        // Read half: the entry world serves the ENTRY bytes.
        let mut host = host_of(&world, &root, Instant::now() + Duration::from_secs(7));
        let face = host.toc("doc.md", &[]).expect("entry world serves");
        assert_eq!(
            face.fm.get("status").map(String::as_str),
            Some("open"),
            "a foreign mid-program change is invisible: reads span ONE state"
        );

        // Commit half: the §5.1 guard folds the LIVE world under the flock.
        let args = wire_serve::write::SpliceArgs {
            id: None,
            path: wire::Path("doc.md".to_owned()),
            origin: wire_serve::guard::Origin::Wire,
            actor: None,
            now: None,
            receipt: None,
            if_root: Some(root.clone()),
            dry: false,
            force: false,
            edits: Vec::new(),
            plan_edits: vec![PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: Some(face.rev.clone()),
            }],
            pin: None,
        };
        let ws_root = fs::WorkspaceRoot(ws.clone());
        let refused = wire_serve::write::splice(&ws_root, None, &args, &[], None)
            .expect_err("a moved world refuses the commit");
        assert!(
            matches!(refused.code, ErrorCode::RootMismatch),
            "the §5.1 world guard, checked first: {refused:?}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("doc.md")).unwrap(),
            DOC.replace("open", "parked"),
            "nothing landed — the foreign state stands untouched"
        );
    }

    /// The entry-rev law: the license is the recording's, the value is the
    /// entry world's. A recording whose only read of the target served the
    /// OVERLAY (its rev names bytes no disk carried) still licenses the row —
    /// and the threaded token is the ENTRY rev, never the overlay rev. An
    /// empty recording licenses nothing.
    #[test]
    fn threading_licenses_from_the_recording_and_values_from_the_entry_world() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let entry_rev = entry_toc(&world, &root, "doc.md").expect("doc in world").rev;

        let armed = vec![ArmedEdit {
            path: "doc.md".to_owned(),
            edit: PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: None,
            },
            line: 2,
            depth: 0,
        }];
        let overlay_read = ScriptRecording {
            actor: String::new(),
            reads: vec![ReadRecord {
                path: "doc.md".to_owned(),
                section: None,
                line: 3,
                position: ReadPosition::Echo,
                face: ReadFace::Toc(TocFacts {
                    rev: "feedfacefeedface".to_owned(), // an overlay rev: no disk state carries it
                    fm: std::collections::BTreeMap::new(),
                    toc: Vec::new(),
                    words: 0,
                }),
            }],
        };

        let threaded = thread_entry(&armed, &overlay_read, &world, &root);
        let PlanEdit::SetProperty { rev, .. } = &threaded[0].edit else {
            panic!("shape preserved");
        };
        assert_eq!(
            rev.as_deref(),
            Some(entry_rev.as_str()),
            "the token is the ENTRY rev — the pre-batch state the §4.4 guards \
             resolve against — never the overlay rev the read served"
        );

        let unlicensed = thread_entry(
            &armed,
            &ScriptRecording {
                actor: String::new(),
                reads: Vec::new(),
            },
            &world,
            &root,
        );
        let PlanEdit::SetProperty { rev, .. } = &unlicensed[0].edit else {
            panic!("shape preserved");
        };
        assert!(
            rev.is_none(),
            "a row whose target the attempt never read threads nothing and \
             meets the engine's own guard_required"
        );
    }

    /// Wall site 2: the read builtin refuses on a lapsed clock — typed, in
    /// the entry's own vocabulary, before any serve.
    #[test]
    fn the_wall_clock_binds_at_the_read_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);

        let lapsed = Instant::now() - Duration::from_millis(1);
        let mut host = host_of(&world, &root, lapsed);
        let fault = host.toc("doc.md", &[]).expect_err("lapsed clock refuses");
        assert!(
            fault.reason.contains("wall clock elapsed"),
            "the refusal names the budget: {}",
            fault.reason
        );
    }
}
