//! `mrd pin --refresh PAGE [--accept] [--dry] [--json]` — re-pin every row of PAGE's
//! `meridian-lock`, one `mrd pin` splice per row, in lock order (`docs/status.md`
//! § `mrd pin --refresh`).
//!
//! - **Rows in.** PAGE's lock is read in-process by the one lock parser ([`lock::find`]). A
//!   row's target is its `object` plus `.md`; its stored selector becomes the wire's tagged one
//!   through the walk plane's reading ([`view::walk::model_selector`]).
//! - **Pass 0** rehearses each row carrying its recorded fingerprint as its § A.3 proof and
//!   writes nothing. `pin_proof_required` there is the drift signal.
//! - **Pass 1** pins in lock order: an `ok` row with its recorded token, a drifted row (only
//!   under `--accept`) proofless — the CLI door is local-operator-trusted.
//! - **Pass 2** re-pins the rows whose target file a LATER row's anchor promotion changed, so
//!   every row's `hash` names its target file's final blob.
//!
//! Exit triad: 0 every row pinned (or `--dry` clean) / 1 a refused row, a drifted row without
//! `--accept`, or a live-pass refusal / 2 bad invocation (including a down daemon).

use std::path::Path;

use serde_json::{Value, json};
use wire::{ErrorBody, ErrorCode, HpathSeg, Path as WirePath, PinFact, PinSpec, ReadSel};

use super::{pin_request, pinning_workspace};
use crate::script::wire_host::SocketDoor;
use crate::{Fail, Format, engine, write_ipc};

/// Run `mrd pin --refresh PAGE [flags]`. Errors [`Fail`] — exit 2 on a bad invocation, exit 1
/// on a refusal or an unaccepted drift; the rows are printed either way.
pub(super) fn run(args: &[String]) -> Result<(), Fail> {
    let mut parsed = Refresh::parse(args)?;
    let workspace = pinning_workspace(&mut parsed.page, &mut parsed.display, parsed.format)?;
    let mut run = Run {
        workspace: &workspace,
        page: &parsed.page,
        shown: parsed
            .display
            .clone()
            .unwrap_or_else(|| parsed.page.clone()),
        accept: parsed.accept,
        dry: parsed.dry,
        format: parsed.format,
        rows: Vec::new(),
    };
    let pins = match lock_pins(
        &workspace,
        &parsed.page,
        &run.shown,
        parsed.display.is_some(),
    ) {
        Ok(pins) => pins,
        Err(fail) => return Err(run.stop(fail)),
    };
    run.rows = plan_rows(&pins)
        .into_iter()
        .map(|row| Outcome {
            row,
            state: State::Ok,
            fact: None,
            promoted: false,
            reason: None,
        })
        .collect();
    if run.rows.is_empty() {
        run.emit(None);
        return Ok(());
    }

    let mut door = write_ipc::connect(&workspace)?;
    run.rehearse(&mut door);
    if let Some(fail) = run.verdict() {
        return Err(run.stop(fail));
    }
    if !run.dry {
        if let Some(fail) = run.pin_pass(&mut door) {
            return Err(run.stop(fail));
        }
        if let Some(fail) = run.final_blob_pass(&mut door) {
            return Err(run.stop(fail));
        }
    }
    run.emit(None);
    Ok(())
}

/// The parsed `pin --refresh` invocation.
#[derive(Debug)]
struct Refresh {
    /// The pinning page whose lock is refreshed — the rel half on the rooted lane.
    page: String,
    /// The rooted lane's canonical spelling, echoed by the faces; `None` on the ambient lane.
    display: Option<String>,
    /// `--accept`: pin drifted rows at their live bytes.
    accept: bool,
    /// `--dry`: stop after the rehearsal.
    dry: bool,
    format: Format,
}

impl Refresh {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Vec<&str> = Vec::new();
        let mut accept = false;
        let mut dry = false;
        let mut json = false;
        for arg in args {
            match arg.as_str() {
                "--refresh" => {}
                "--accept" => accept = true,
                "--dry" => dry = true,
                "--json" => json = true,
                "--fingerprint" => {
                    return Err(Fail::tool(
                        "--fingerprint does not ride --refresh: each row carries its own \
                         recorded fingerprint as its proof. Pin one section with a token of \
                         your own through `mrd pin PAGE TARGET#SELECTOR --fingerprint TOKEN`."
                            .to_owned(),
                    ));
                }
                "--vibe" => {
                    return Err(Fail::tool(
                        "--vibe does not ride --refresh: a refresh computes each row's blob \
                         read-only. Commit the target files to make the refreshed hashes \
                         durable, or pin one section with `mrd pin PAGE TARGET#SELECTOR --vibe`."
                            .to_owned(),
                    ));
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => positional.push(value),
            }
        }
        let page = match positional.as_slice() {
            [page] => (*page).to_owned(),
            [] => {
                return Err(Fail::tool(
                    "--refresh needs the PAGE whose lock to re-pin — e.g. \
                     `mrd pin --refresh notes/plan.md`"
                        .to_owned(),
                ));
            }
            many => {
                return Err(Fail::tool(format!(
                    "--refresh takes PAGE alone, no TARGET#SELECTOR: it re-pins every row \
                     PAGE's meridian-lock already holds. Got: {}",
                    many.join(" ")
                )));
            }
        };
        Ok(Refresh {
            page,
            display: None,
            accept,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// PAGE's lock rows, read in-process by the one lock parser. A page with no lock has no rows.
///
/// # Errors
/// Exit 2 when PAGE breaks the §1 path law or has no file; exit 1 when its lock refuses to
/// parse — nothing is written either way.
fn lock_pins(
    workspace: &Path,
    page: &str,
    shown: &str,
    rooted: bool,
) -> Result<Vec<lock::PinEntry>, Fail> {
    // §1 admission before the page is read: `fs::load` resolves an absolute spelling verbatim
    // (wire-contract §12.1, the door-family clause). On the rooted lane the rel half is
    // already confined — a no-op pass.
    crate::path_law::admit(workspace, page, "pin", "Nothing was written.")?;
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let doc = fs::load(&root, Path::new(page)).map_err(|e| {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Fail::tool(format!("cannot read {shown}: {e}. Nothing was written."));
        }
        let mut m = format!(
            "no page {shown}, so there is no meridian-lock to refresh. Nothing was written."
        );
        if !rooted
            && let Ok(cwd) = std::env::current_dir()
            && let Some(suffix) = crate::path_law::cwd_respell_suffix(workspace, &cwd, page)
        {
            m.push_str(&suffix);
        }
        Fail::tool(m)
    })?;
    match lock::find(&doc) {
        Ok(found) => Ok(found.map(|f| f.lock.pins).unwrap_or_default()),
        Err(e) => Err(Fail::findings(format!(
            "the meridian-lock block on {shown} refuses to parse: {e}. Nothing was written — \
             a refresh re-pins the rows the lock holds, and a lock that does not parse \
             holds none it can name. Repair the block, then run the refresh again."
        ))),
    }
}

/// One lock row, read for a refresh: where it points, how the wire addresses it, and what it
/// attests today.
#[derive(Debug)]
struct Row {
    /// The splice target: the row's `object` plus `.md` — the inverse of the mint, which stores
    /// the target path minus `.md`, so a cross-root `name:rel` object keeps its spelling.
    target: String,
    /// The row's own R4 selector, echoed on the `--json` face.
    lock_selector: lock::Selector,
    /// The wire selector, or why no door re-pins this row.
    selector: Result<ReadSel, String>,
    /// The fingerprint the row records — the proof its rehearsal carries.
    recorded: String,
}

impl Row {
    /// `TARGET#SELECTOR` for a human line. Display only: nothing parses it back.
    fn label(&self) -> String {
        match &self.lock_selector {
            lock::Selector::Path(elements) if elements.is_empty() => {
                format!("{} path []", self.target)
            }
            lock::Selector::Path(elements) => format!("{}#{}", self.target, elements.join("/")),
            lock::Selector::Properties(keys) => {
                format!("{} properties [{}]", self.target, keys.join(", "))
            }
        }
    }
}

/// The rows of one lock, in lock order. A row repeating an earlier row's key — the same object
/// and selector — is refused: a pin upserts the FIRST row under its key, so the repeat could
/// never be refreshed.
fn plan_rows(pins: &[lock::PinEntry]) -> Vec<Row> {
    pins.iter()
        .enumerate()
        .map(|(i, pin)| {
            let repeated = pins[..i].iter().any(|earlier| earlier.key() == pin.key());
            Row {
                target: format!("{}.md", pin.object),
                lock_selector: pin.selector.clone(),
                selector: if repeated {
                    Err(REPEATED_ROW.to_owned())
                } else {
                    wire_selector(pin)
                },
                recorded: pin.fingerprint.clone(),
            }
        })
        .collect()
}

/// A lock row's selector as the wire's tagged one, through the walk plane's one reading of the
/// stored spelling: heading segments via the R4 occurrence spelling (`"Dup#2"` → `n: 2`), a
/// sole `^id` as the anchor. The rows no door mints are refused with their reason.
fn wire_selector(pin: &lock::PinEntry) -> Result<ReadSel, String> {
    use model::selector::Selector;
    if matches!(pin.selector, lock::Selector::Properties(_)) {
        return Err(PROPERTIES_ROW.to_owned());
    }
    match view::walk::model_selector(&pin.object, &pin.selector) {
        Selector::Heading(segments) => Ok(ReadSel::Hpath {
            hpath: segments
                .into_iter()
                .map(|seg| HpathSeg { h: seg.h, n: seg.n })
                .collect(),
        }),
        Selector::Block(anchor) => Ok(ReadSel::Anchor { anchor }),
        Selector::Page => Err(WHOLE_BODY_ROW.to_owned()),
        Selector::ImmutableRoot { .. } => Err(IMMUTABLE_ROOT_ROW.to_owned()),
    }
}

/// A repeat of an earlier row's key.
const REPEATED_ROW: &str = "this row repeats an earlier row's object and selector, and a pin \
     updates the first row under its key only, so this one can never be refreshed — remove one \
     of the two by hand";

/// A `properties` row.
const PROPERTIES_ROW: &str = "a `properties` row claims frontmatter keys, and no door mints or \
     re-pins one — the pin door addresses body sections only";

/// A `path: []` row.
const WHOLE_BODY_ROW: &str = "a `path: []` row claims the whole body, and no door mints or \
     re-pins one — a page-level pin cannot localize drift";

/// A `seq-N` row.
const IMMUTABLE_ROOT_ROW: &str = "a `seq-N` row names a transcript immutable root, which is \
     never resolved, so no door re-pins it";

/// Where one row stands in the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The target still carries the recorded content.
    Ok,
    /// The target moved: the recorded fingerprint no longer matches.
    Drifted,
    /// `--dry --accept`: a drifted row the live run would pin at its live bytes.
    WouldAccept,
    /// A drifted row pinned at its live bytes.
    Accepted,
    /// An `Ok` row pinned again for its target file's final blob.
    RePinned,
    /// The engine, or the row's own planning, refused it.
    Refused,
    /// The live pass stopped before this row.
    NotRun,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Drifted => "drifted",
            State::WouldAccept => "would-accept",
            State::Accepted => "accepted",
            State::RePinned => "re-pinned",
            State::Refused => "refused",
            State::NotRun => "not-run",
        }
    }
}

/// One row's run: its state, the last fact the engine answered for it, and why it refused.
#[derive(Debug)]
struct Outcome {
    row: Row,
    state: State,
    /// The last pin fact for this row — rehearsed, then minted.
    fact: Option<PinFact>,
    /// Did the pin that stands promote an anchor into the target (would, under `--dry`)?
    promoted: bool,
    /// The refusal's text, when the row refused.
    reason: Option<String>,
}

/// A refresh in flight: the page it writes, the faces' spelling of it, and every row's outcome.
struct Run<'a> {
    workspace: &'a Path,
    /// The page as the wire addresses it.
    page: &'a str,
    /// The page as the faces name it — the canonical `root:rel` on the rooted lane.
    shown: String,
    accept: bool,
    dry: bool,
    format: Format,
    rows: Vec<Outcome>,
}

/// One pin splice for a refresh row. Success is the engine's pin fact.
fn splice_pin(
    door: &mut SocketDoor,
    page: &str,
    row: &Row,
    selector: &ReadSel,
    proof: Option<&str>,
    dry: bool,
) -> Result<PinFact, Box<ErrorBody>> {
    let pin = PinSpec {
        target: WirePath(row.target.clone()),
        selector: selector.clone(),
        vibe: None,
        fingerprint: proof.map(str::to_owned),
        sec_rev: None,
    };
    let body = write_ipc::call(door, &pin_request(page, &pin, dry))?;
    let body = write_ipc::project_body(&body);
    body.get("pin")
        .cloned()
        .and_then(|fact| serde_json::from_value::<PinFact>(fact).ok())
        .ok_or_else(|| {
            let mut err = ErrorBody::new(ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon answered the pin of {} with no pin fact: {body}",
                row.label()
            ));
            Box::new(err)
        })
}

/// A rehearsed fact that would land under a different lock key than the row's own — another
/// target spelling, or a canonical selector the row does not carry (an occurrence ordinal a
/// heading chain now needs). Re-pinning it would add a row beside this one, never refresh it.
fn key_mismatch(row: &Row, selector: &ReadSel, fact: &PinFact) -> Option<String> {
    if fact.target.0 == row.target && &fact.selector == selector {
        return None;
    }
    Some(format!(
        "the engine resolves this row to {}#{}, a different lock key than the row's own, so \
         re-pinning it would add a second row instead of refreshing this one. Pin that address \
         with `mrd pin` and remove the stale row by hand.",
        fact.target.0,
        fact.selector.display()
    ))
}

impl Run<'_> {
    /// Pass 0: one dry pin per row carrying its recorded fingerprint. `pin_proof_required` is the
    /// drift signal; the drifted row is rehearsed again proofless, so a refusal it would meet at
    /// the live pass is reported now. Writes nothing.
    fn rehearse(&mut self, door: &mut SocketDoor) {
        for o in &mut self.rows {
            let selector = match &o.row.selector {
                Ok(selector) => selector,
                Err(reason) => {
                    o.state = State::Refused;
                    o.reason = Some(reason.clone());
                    continue;
                }
            };
            let recorded = Some(o.row.recorded.as_str());
            let proved = splice_pin(door, self.page, &o.row, selector, recorded, true);
            let (state, answer) = match proved {
                Err(e) if e.code == ErrorCode::PinProofRequired => (
                    State::Drifted,
                    splice_pin(door, self.page, &o.row, selector, None, true),
                ),
                answer => (State::Ok, answer),
            };
            let refusal = match answer {
                Ok(fact) => {
                    let mismatch = key_mismatch(&o.row, selector, &fact);
                    if mismatch.is_none() {
                        o.state = state;
                        o.promoted = fact.promoted;
                        o.fact = Some(fact);
                    }
                    mismatch
                }
                Err(e) => Some(engine::refusal_text(&e)),
            };
            if let Some(reason) = refusal {
                o.state = State::Refused;
                o.reason = Some(reason);
            }
        }
    }

    /// The rehearsal's verdict: a refused row stops the run, and so does a drifted one without
    /// `--accept`. Under `--dry --accept` a drifted row reads `would-accept`.
    fn verdict(&mut self) -> Option<Fail> {
        let total = self.rows.len();
        let refused: Vec<String> = self
            .rows
            .iter()
            .filter(|o| o.state == State::Refused)
            .map(|o| {
                format!(
                    "\n  {}: {}",
                    o.row.label(),
                    o.reason.as_deref().unwrap_or("refused")
                )
            })
            .collect();
        if !refused.is_empty() {
            return Some(Fail::findings(format!(
                "{} of {total} pin(s) in {} refused — nothing was written:{}",
                refused.len(),
                self.shown,
                refused.concat()
            )));
        }
        let drifted = self
            .rows
            .iter()
            .filter(|o| o.state == State::Drifted)
            .count();
        if drifted > 0 && !self.accept {
            return Some(Fail::findings(format!(
                "{drifted} of {total} pin(s) in {shown} drifted: the target no longer carries \
                 the content the row attests, and the refresh does not re-pin moved content \
                 unasked. Nothing was written. Read what changed (`mrd walk {shown}`), fix the \
                 page where a claim no longer holds, then run \
                 `mrd pin --refresh {shown} --accept`.",
                shown = self.shown
            )));
        }
        if self.dry {
            for o in &mut self.rows {
                if o.state == State::Drifted {
                    o.state = State::WouldAccept;
                }
            }
        }
        None
    }

    /// Pass 1: pin every row in lock order. An `ok` row carries its recorded token, verified
    /// again under the write flock; a drifted row (only under `--accept`) carries none — the CLI
    /// door is local-operator-trusted and pins the live bytes, and `--accept` is the operator's
    /// statement that they re-read them.
    fn pin_pass(&mut self, door: &mut SocketDoor) -> Option<Fail> {
        let mut stopped = None;
        for (i, o) in self.rows.iter_mut().enumerate() {
            let Ok(selector) = &o.row.selector else {
                continue; // a clean rehearsal leaves no such row
            };
            let accepting = o.state == State::Drifted;
            let proof = (!accepting).then_some(o.row.recorded.as_str());
            match splice_pin(door, self.page, &o.row, selector, proof, false) {
                Ok(fact) => {
                    o.state = if accepting {
                        State::Accepted
                    } else {
                        State::Ok
                    };
                    o.promoted = fact.promoted;
                    o.fact = Some(fact);
                }
                Err(e) => {
                    o.state = State::Refused;
                    o.fact = None;
                    o.reason = Some(engine::refusal_text(&e));
                    stopped = Some(i);
                    break;
                }
            }
        }
        let i = stopped?;
        for o in &mut self.rows[i + 1..] {
            o.state = State::NotRun;
            o.fact = None;
        }
        let landed: Vec<String> = self.rows[..i].iter().map(|o| o.row.label()).collect();
        Some(Fail::findings(format!(
            "the pin of {} refused after a clean rehearsal — the target moved between the \
             passes, or a write gate refused: {} Landed before it: {}. Not attempted: {} \
             row(s). Run `{}` again once the cause is fixed; the rows that landed re-pin as ok.",
            self.rows[i].row.label(),
            self.rows[i].reason.as_deref().unwrap_or("refused"),
            if landed.is_empty() {
                "none".to_owned()
            } else {
                landed.join(", ")
            },
            self.rows.len() - i - 1,
            self.rerun()
        )))
    }

    /// Pass 2: a row's `hash` is its target file's blob at that row's pin, so a LATER row's anchor
    /// promotion into the same file leaves every earlier row into it naming a blob no later state
    /// holds. Exactly those rows are pinned again, carrying their pass-1 fingerprint; the anchors
    /// exist now, so nothing promotes again.
    fn final_blob_pass(&mut self, door: &mut SocketDoor) -> Option<Fail> {
        let stale: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                self.rows[i + 1..]
                    .iter()
                    .any(|later| later.promoted && later.row.target == self.rows[i].row.target)
            })
            .collect();
        let rerun = self.rerun();
        for i in stale {
            let o = &mut self.rows[i];
            let (Ok(selector), Some(minted)) = (&o.row.selector, &o.fact) else {
                continue; // every row pinned in pass 1 holds both
            };
            let proof = minted.fingerprint.clone();
            match splice_pin(
                door,
                self.page,
                &o.row,
                selector,
                Some(proof.as_str()),
                false,
            ) {
                Ok(fact) => {
                    if o.state == State::Ok {
                        o.state = State::RePinned;
                    }
                    o.promoted |= fact.promoted;
                    o.fact = Some(fact);
                }
                Err(e) => {
                    let reason = engine::refusal_text(&e);
                    let fail = Fail::findings(format!(
                        "every row landed, but re-pinning {} for its target file's final blob \
                         refused: {reason} Rows into that file pinned before a later anchor \
                         still name an earlier blob of it. Run `{rerun}` again once the cause \
                         is fixed.",
                        o.row.label()
                    ));
                    o.state = State::Refused;
                    o.fact = None;
                    o.reason = Some(reason);
                    return Some(fail);
                }
            }
        }
        None
    }

    /// The command that resumes this run.
    fn rerun(&self) -> String {
        let accept = if self.accept { " --accept" } else { "" };
        format!("mrd pin --refresh {}{accept}", self.shown)
    }

    /// Print the run, then hand back the failure that stopped it.
    fn stop(&self, fail: Fail) -> Fail {
        self.emit(Some(fail.message.as_str()));
        fail
    }

    /// Both faces. Human: one line per row, then a summary when the run did not stop (a stop's
    /// message goes to stderr). JSON: `{workspace, page, rows, error}`.
    fn emit(&self, error: Option<&str>) {
        match self.format {
            Format::Json => {
                let value = json!({
                    "workspace": self.workspace.display().to_string(),
                    "page": self.shown,
                    "rows": self.rows.iter().map(row_json).collect::<Vec<_>>(),
                    "error": error,
                });
                println!("{}", serde_json::to_string_pretty(&value).expect("json"));
            }
            Format::Human => {
                for o in &self.rows {
                    println!("{}", human_line(o));
                }
                if error.is_none() {
                    println!("{}", self.summary());
                }
            }
        }
    }

    /// The closing line of a run that did not stop.
    fn summary(&self) -> String {
        let count = |state: State| self.rows.iter().filter(|o| o.state == state).count();
        if self.rows.is_empty() {
            format!("no pins in {} — nothing to refresh", self.shown)
        } else if self.dry {
            format!(
                "rehearsed {} pin(s) in {}: {} ok, {} would-accept — dry run, nothing written",
                self.rows.len(),
                self.shown,
                count(State::Ok),
                count(State::WouldAccept)
            )
        } else {
            format!(
                "refreshed {} pin(s) in {}: {} ok, {} accepted, {} re-pinned",
                self.rows.len(),
                self.shown,
                count(State::Ok),
                count(State::Accepted),
                count(State::RePinned)
            )
        }
    }
}

/// The row's state, `TARGET#SELECTOR`, and the short fingerprint it carries.
fn human_line(o: &Outcome) -> String {
    let word = o.state.word();
    let label = o.row.label();
    match (o.state, &o.fact) {
        (State::Refused | State::NotRun, _) | (_, None) => format!("{word:<12}  {label}"),
        (State::Drifted | State::WouldAccept, Some(fact)) => format!(
            "{word:<12}  {label}  recorded {} live {}",
            short(&o.row.recorded),
            short(&fact.fingerprint)
        ),
        (_, Some(fact)) => format!("{word:<12}  {label}  {}", short(&fact.fingerprint)),
    }
}

/// The first twelve digest characters of an `fp1.…` token — enough to tell two apart by eye.
fn short(token: &str) -> &str {
    let digest = token.rsplit('.').next().unwrap_or(token);
    digest.get(..12).unwrap_or(digest)
}

/// One row on the `--json` face. `selector` is the row's own R4 selector; `fingerprint` and
/// `blob` are what the run minted or rehearsed, `null` where the row refused or never ran.
fn row_json(o: &Outcome) -> Value {
    let mut selector = serde_json::Map::new();
    selector.insert(
        o.row.lock_selector.key().to_owned(),
        json!(o.row.lock_selector.elements()),
    );
    let fact = o
        .fact
        .as_ref()
        .filter(|_| !matches!(o.state, State::Refused | State::NotRun));
    json!({
        "target": o.row.target,
        "selector": selector,
        "state": o.state.word(),
        "fingerprint": fact.map(|f| f.fingerprint.as_str()),
        "blob": fact.and_then(|f| f.blob.as_deref()),
        "promoted": o.promoted,
        "recorded": o.row.recorded,
        "reason": o.reason,
    })
}

#[cfg(test)]
mod tests {
    //! Invocation parsing and row planning — the passes run end to end in
    //! `crates/mrd/tests/pin_refresh_e2e.rs`, where a real workspace and a daemon exist.

    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    /// `--refresh` takes PAGE alone, in any position, with `--accept`, `--dry` and `--json`.
    #[test]
    fn refresh_takes_page_and_its_three_flags() {
        let bare = Refresh::parse(&argv(&["--refresh", "notes/plan.md"])).expect("parses");
        assert_eq!(bare.page, "notes/plan.md");
        assert!(!bare.accept && !bare.dry);
        assert!(matches!(bare.format, Format::Human));

        let full = Refresh::parse(&argv(&[
            "plan.md",
            "--json",
            "--accept",
            "--refresh",
            "--dry",
        ]))
        .expect("parses");
        assert_eq!(full.page, "plan.md");
        assert!(full.accept && full.dry);
        assert!(matches!(full.format, Format::Json));
    }

    /// No TARGET beside `--refresh`, no single-pin flags, no unknown flag, and a PAGE is owed —
    /// each refused at exit 2, naming what it refused.
    #[test]
    fn refresh_refuses_a_target_and_the_single_pin_flags() {
        for (args, names) in [
            (vec!["--refresh", "a.md", "b.md#A"], "PAGE alone"),
            (vec!["--refresh"], "needs the PAGE"),
            (
                vec!["--refresh", "a.md", "--fingerprint", "fp1.x"],
                "--fingerprint",
            ),
            (vec!["--refresh", "a.md", "--vibe"], "--vibe"),
            (vec!["--refresh", "a.md", "--nope"], "--nope"),
        ] {
            let err = Refresh::parse(&argv(&args)).expect_err("refused");
            assert_eq!(err.code, 2, "{args:?}");
            assert!(err.message.contains(names), "{args:?}: {}", err.message);
        }
    }

    fn entry(object: &str, selector: lock::Selector) -> lock::PinEntry {
        lock::PinEntry::new(object, "blob", selector, "fp1.span2.b3.00")
    }

    fn path(elements: &[&str]) -> lock::Selector {
        lock::Selector::Path(elements.iter().map(|e| (*e).to_owned()).collect())
    }

    /// A row's target is its object plus `.md`, a cross-root object keeping its spelling; a
    /// heading path becomes segments read through the occurrence spelling, a sole `^id` the
    /// anchor.
    #[test]
    fn lock_rows_become_wire_pins() {
        let rows = plan_rows(&[
            entry("notes/guide", path(&["Guide", "Dup#2"])),
            entry("sessions:notes/a", path(&["^claim"])),
        ]);
        assert_eq!(rows[0].target, "notes/guide.md");
        assert_eq!(
            rows[0].selector,
            Ok(ReadSel::Hpath {
                hpath: vec![
                    HpathSeg {
                        h: "Guide".into(),
                        n: None
                    },
                    HpathSeg {
                        h: "Dup".into(),
                        n: Some(2)
                    },
                ]
            })
        );
        assert_eq!(rows[0].recorded, "fp1.span2.b3.00");
        assert_eq!(rows[0].label(), "notes/guide.md#Guide/Dup#2");
        assert_eq!(rows[1].target, "sessions:notes/a.md");
        assert_eq!(
            rows[1].selector,
            Ok(ReadSel::Anchor {
                anchor: "claim".into()
            })
        );
    }

    /// The rows no door mints or re-pins refuse with a reason, as does a repeat of an earlier
    /// row's key — while the first row under that key stays addressable.
    #[test]
    fn unpinnable_rows_refuse_with_a_reason() {
        let rows = plan_rows(&[
            entry("guide", path(&[])),
            entry("guide", lock::Selector::Properties(vec!["status".into()])),
            entry("session-1", path(&["seq-3"])),
            entry("guide", path(&["Guide"])),
            entry("guide", path(&["Guide"])),
        ]);
        for (row, names) in rows.iter().zip(["path: []", "properties", "seq-N"]) {
            let reason = row.selector.as_ref().expect_err("refused");
            assert!(reason.contains(names), "{}: {reason}", row.label());
        }
        assert!(rows[3].selector.is_ok(), "the first row under a key stays");
        let repeat = rows[4].selector.as_ref().expect_err("the repeat refuses");
        assert!(repeat.contains("repeats an earlier row"), "{repeat}");
    }
}
