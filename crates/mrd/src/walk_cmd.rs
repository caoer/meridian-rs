//! `mrd walk` — the context-assembly listing (U2.3; d2 §2.4 / §3).
//!
//! ```text
//! mrd walk <PAGE> [--down] [--depth N] [--json]
//! ```
//!
//! Up (default) = ancestors, the context walk: what PAGE draws from,
//! transitively. `--down` = descendants, the dependents renderer and dry-run
//! blast radius; `--depth 1` = exactly the direct dependents. Every listing
//! entry is `{selector, rev, color, depth}` and every answer cites the doc revs
//! it read (§2.4 honesty).
//!
//! Read-only: writes nothing, mints no receipt — the walk is computed per query,
//! never stored (§2.4 / §3). Output is the house grammar: a human listing by
//! default, JSON under `--json`. Exit triad (§4 preamble):
//! - **0** — clean: no red edge in the listing.
//! - **1** — a finding: at least one red edge (a broken pin in the context, or a
//!   dependent whose pin no longer resolves).
//! - **2** — bad invocation, or a structural failure (root absent from the
//!   corpus, or an in-snapshot cycle).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use model::Document;
use serde_json::{Value, json};
use view::walk::{self, Direction, WalkError, WalkReport};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, the listing
/// carries a red edge.
const EXIT_FINDING: u8 = 1;

/// Run `mrd walk <PAGE> [--down] [--depth N] [--json]`: resolve the workspace,
/// build the corpus in-process, walk the pin graph, and print the listing.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation, an unreadable workspace/corpus, a missing
/// root, or an in-snapshot cycle; exit 1 when the listing carries a red edge.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Walk::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let docs = build_docs(&resolved.workspace)?;

    // U11 — the mount table, loaded from `MERIDIAN.md`, with a corpus for the
    // roots this workspace's own lock addresses NAME ([`lock_addressed_roots`])
    // and no others. `mounts` owns the document maps; `corpus` borrows them, so
    // the owner must outlive the borrow — hence two bindings rather than one
    // expression.
    //
    // W5 — the table stays whole and only the CORPORA narrow, so every
    // `grey(unmounted)` / `path-unseeable` verdict reads the same set it always
    // did (see [`load_mounts_for`]). The narrowing is sound for the WALK in
    // particular because the walk never traverses into a root: a cross-root
    // target is a leaf (`view::walk::steps_from`), so a root's pages are read
    // only to colour an edge naming that root — which is what this set collects.
    let mounts = load_mounts_for(&lock_addressed_roots(&docs));
    let corpus = mounts.rooted(&docs);
    let mount_set = mounts.set();

    let report = walk::walk_rooted(
        &corpus,
        mount_set,
        &parsed.page,
        parsed.direction,
        parsed.depth,
    )
    .map_err(walk_error)?;

    match parsed.format {
        Format::Json => {
            let value = to_json(&resolved.workspace, &report);
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print!("{}", render_human(&report)),
    }

    let reds = report
        .entries
        .iter()
        .filter(|e| walk::color_tone(&e.color) == "red")
        .count();
    // S3-R6 — grey REFUSES, on exit 1, each with its OWN reason word, both
    // distinct in the human line and in `--json`. **No fourth exit code**, and
    // `--force` is the escape.
    //
    // Scoped to the two ROOT greys deliberately: the pre-existing greys
    // (`immutable-root`, `declared-unpinned`, …) keep exiting 0, because they
    // describe edges the ledger was never asked to measure. These two are
    // different — the address ASKED a question this machine cannot answer, and
    // exiting 0 would make unmounting a root a way to turn a red into a pass
    // through an edit to `~/MERIDIAN.md`, which cannot itself be attested.
    let root_greys: Vec<&str> = report
        .entries
        .iter()
        .filter_map(|e| walk::color_reason(&e.color))
        .filter(|w| *w == "unmounted" || *w == addr::PATH_UNSEEABLE_REASON_WORD)
        .collect();
    if reds > 0 || !root_greys.is_empty() {
        let mut findings = Vec::new();
        if reds > 0 {
            findings.push(format!("{reds} red edge(s)"));
        }
        for word in ["unmounted", addr::PATH_UNSEEABLE_REASON_WORD] {
            let n = root_greys.iter().filter(|w| **w == word).count();
            if n > 0 {
                findings.push(format!("{n} grey({word}) edge(s)"));
            }
        }
        return Err(Fail::with_code(
            EXIT_FINDING,
            format!("{} in the walk", findings.join(", ")),
        ));
    }
    Ok(())
}

/// One mounted root's corpus, owned — the backing store `RootedCorpus` borrows.
struct MountedCorpus {
    name: addr::MountName,
    kind: model::RootKind,
    docs: BTreeMap<String, Document>,
}

/// Load `MERIDIAN.md`'s mount table into BOTH halves resolution needs: one
/// corpus per usable root, and the projection that says what the file declares.
///
/// **S3-R50 — a declared root that cannot be used is CARRIED WITH ITS STATE, never
/// dropped.** Round 1 dropped it (`state().refuses()` → `continue`, and a
/// `build_docs_at` failure skipped it), so a root the file DECLARES fell through
/// to the *undeclared* arm and `mrd walk` told the user to declare it — while
/// `mrd config`, one command away, was already naming the path correctly.
///
/// > A word added downstream of a place that ERASES the distinction is a word
/// > that never gets used.
///
/// So the three causes arrive at the renderer distinct: **not declared at all**
/// (absent from the projection entirely → `grey(unmounted)`, teach the
/// declaration); **declared but unusable** and **declared, bound, corpus
/// unreadable** (both recorded as unreachable with their path → the shared
/// `path-unseeable` word, teach the PATH).
///
/// **Never fails the walk.** No `MERIDIAN.md`, an unparseable one, or an
/// unreadable root simply yields fewer usable roots — absence is the topology
/// working as designed (§ 8 M6), not a reason to brick every single-root
/// workspace on the planet.
pub(crate) fn load_mounts() -> Mounts {
    load_mounts_where(&|_| true)
}

/// [`load_mounts`], building a corpus ONLY for the roots in `needed`.
///
/// # Why a caller would want fewer corpora
/// Building a root's corpus is a recursive directory walk plus a read and parse
/// of every page in it — the cost is the ROOT's size, and nothing about the
/// workspace under test bounds it. `mrd status` measured 80.6% of its whole run
/// inside this function, walking 271 MB across four declared roots to colour two
/// pins, none of which addressed a root at all.
///
/// A caller that KNOWS which roots its addresses name pays for those and no
/// others. Which roots those are is the caller's question, not this loader's:
/// `status` reads them off its own lock items ([`crate::status_cmd`]), and a
/// caller that cannot know keeps [`load_mounts`].
///
/// # The declared table is unchanged, and that is the whole safety argument
/// `needed` narrows the CORPORA, never the [`addr::MountSet`]. A skipped root is
/// still pushed to `bound`, so the set answers `is_bound` exactly as it did, and
/// every `grey(unmounted)` / `path-unseeable` verdict — which read the set and
/// not the corpora — is byte-identical.
///
/// What a skipped root loses is only its docs, and the ONE consumer of those is
/// a `root:`-qualified address resolving into it
/// (`model::CorpusIndex::resolve_ref` arm (b)). That arm already exists and
/// already refuses loudly — *"the mount table binds this root, but no corpus for
/// it was loaded in this process"* — so a caller that narrows `needed` too far
/// gets a named refusal, never a false green. Laziness cannot manufacture a
/// clean answer here; it can only fail to have one.
///
/// One behaviour does change, unobservably: a skipped root whose corpus would
/// have FAILED to build lands as bound-with-no-corpus instead of `unreachable`.
/// Only an address naming that root could tell the two apart, and the caller
/// skipped it precisely because no address names it.
pub(crate) fn load_mounts_for(needed: &BTreeSet<addr::MountName>) -> Mounts {
    load_mounts_where(&|name| needed.contains(name))
}

/// Every mount root the corpus's `meridian-lock` addresses NAME — the exact set
/// of roots whose pages this run can read, and so the exact set worth building.
///
/// # Why the set is knowable before any root is loaded
/// A pin's root is a property of its ADDRESS, not of the tree the address points
/// into: `sessions:notes/plan.md` names `sessions` whether or not `sessions` is
/// declared, readable, or holds that page. Reading the name therefore costs the
/// ambient corpus that is already in memory and no root corpus at all.
///
/// The root is read off [`view::read_face::LockItem::declared_addr`] — the
/// parsed address, which that field's contract names **the structural owner**
/// (U10): *"Every consumer that needs the root, the path or the selector reads
/// THIS; nothing re-splits `declared_ref`."* A second spelling of the address
/// grammar here is exactly the drift that field exists to prevent, so this
/// function contains none.
///
/// A row with no address — a lock refusal, or a spelling outside the grammar —
/// contributes no root, which is correct rather than lossy: it resolves into no
/// root, so no root's pages can answer for it.
///
/// # Why this is the COMPLETE needed set, and not merely a heuristic (W5)
/// The set is read off DECLARED addresses, so it is only sound if an address
/// that names no root can never reach into one. It cannot:
/// `model::CorpusIndex::resolve_ref` branches on `addr.root()` FIRST, and the
/// `None` arm resolves through `three_rules(.., corpus.ambient_docs())` — the
/// ambient map alone (`crates/model/src/lib.rs`, the arm commented *"the ambient
/// root … byte-for-byte what it was before U11"*). No mounted corpus is in scope
/// on that path, so a bare or form-2 `[[wikilink]]` ref resolves ambiently or
/// misses; it never silently lands in a root this set did not name.
///
/// **This is why one computation serves `status`, `walk` and `check` alike**,
/// though W2 expected walk and check to need a different, traversal-aware set.
/// They do not, because neither verb traverses INTO a root: `view::walk`'s
/// `forward_edges` maps `corpus.ambient_docs()` only, and a cross-root target is
/// a leaf by construction — the ambient corpus holds no key spelled `root:path`,
/// so the BFS never expands one. A root's pages are read for exactly one
/// purpose, colouring an edge that NAMES it, and this set names every such root.
pub(crate) fn lock_addressed_roots(docs: &BTreeMap<String, Document>) -> BTreeSet<addr::MountName> {
    let mut roots = BTreeSet::new();
    for doc in docs.values() {
        for item in view::read_face::page_lock_items(doc) {
            if let Some(root) = item.declared_addr.as_ref().and_then(addr::Addr::root) {
                roots.insert(root.clone());
            }
        }
    }
    roots
}

/// The one loader both faces above are spellings of — so the table bind, the
/// refusal carrying, and the set assembly have exactly one implementation and
/// cannot drift between an eager caller and a narrowed one.
fn load_mounts_where(wanted: &dyn Fn(&addr::MountName) -> bool) -> Mounts {
    let Ok(resolution) = config::resolve(&config::Env::from_process()) else {
        return Mounts::default();
    };
    let Some(cfg) = resolution.config() else {
        return Mounts::default();
    };
    let Ok(table) = config::mount::bind(cfg) else {
        return Mounts::default();
    };

    let mut corpora: Vec<MountedCorpus> = Vec::new();
    let mut bound: Vec<addr::MountName> = Vec::new();
    let mut unreachable: Vec<(addr::MountName, String, String)> = Vec::new();

    for mount in table.mounts() {
        let Ok(name) = addr::MountName::parse(mount.name()) else {
            continue; // not a canonical name — no address can reach it anyway
        };
        // Declared, but the mount plane refuses it. Carried, with the path it
        // declares and that plane's own reason verbatim — never dropped.
        if mount.state().refuses() {
            // The RAW filesystem reason where the mount plane has one, so the
            // walk's own refusal reads as one sentence rather than nesting that
            // plane's whole teaching inside this plane's parenthetical. For every
            // other refusing state there is no raw reason, and that plane's
            // teaching IS the most specific thing available — carried verbatim.
            let detail = match mount.state() {
                config::mount::MountState::PathUnseeable { detail } => detail.clone(),
                other => other.detail(),
            };
            unreachable.push((name, mount.declared_path().to_owned(), detail));
            continue;
        }
        let Some(path) = mount.canonical_path() else {
            continue;
        };
        // Bound per the table, corpus not asked for. It joins `bound` exactly as
        // a built root does — the table's answer about this root is a table fact
        // and does not depend on whether this process read its pages.
        if !wanted(&name) {
            bound.push(name);
            continue;
        }
        match build_docs_at(path) {
            Ok(docs) => {
                bound.push(name.clone());
                corpora.push(MountedCorpus {
                    name,
                    kind: mount_kind(mount.kind()),
                    docs,
                });
            }
            // Bound per the table, but its corpus will not build — unreadable
            // FROM here, and just as much a declared root as any other.
            Err(e) => unreachable.push((name, path.display().to_string(), e.message)),
        }
    }

    let mut set = addr::MountSet::new(bound);
    for (name, path, detail) in unreachable {
        set = set.with_unreachable(name, path, detail);
    }
    Mounts { corpora, set }
}

/// The mount table as the pin planes consume it: the loaded corpora, and the
/// projection naming what the file declares and which of it is usable.
#[derive(Default)]
pub(crate) struct Mounts {
    corpora: Vec<MountedCorpus>,
    set: addr::MountSet,
}

impl Mounts {
    /// The ROOT-KEYED corpus over `docs` — the ambient workspace plus one root
    /// per bound mount.
    ///
    /// **One owner for this assembly** (S3-R59): `walk`, `check` and `status` all
    /// colour pins through the same computer, so they must hand it the same
    /// corpus. Two call sites building it by hand is how one of them came to hand
    /// over an ambient-only corpus and a default mount table, and answer
    /// `grey(unmounted)` for every state of a bound root (F6).
    pub(crate) fn rooted<'a>(
        &'a self,
        docs: &'a BTreeMap<String, Document>,
    ) -> model::RootedCorpus<'a> {
        let mut corpus = model::RootedCorpus::ambient(docs);
        for mount in &self.corpora {
            corpus = corpus.with_root(mount.name.clone(), mount.kind.clone(), &mount.docs);
        }
        corpus
    }

    /// The projection naming what `MERIDIAN.md` declares and which of it is
    /// usable — the mount table resolution is a lookup in.
    pub(crate) fn set(&self) -> &addr::MountSet {
        &self.set
    }
}

/// The mount plane's kind, as the resolver's kind.
fn mount_kind(kind: config::MountKind) -> model::RootKind {
    match kind {
        config::MountKind::Vault => model::RootKind::Vault,
        config::MountKind::GitFolder => {
            model::RootKind::Opaque(config::MountKind::GitFolder.as_str().to_owned())
        }
    }
}

/// [`build_docs`] without the workspace-resolution wrapper — a mount path is
/// already canonical (canonicalize-at-bind, § 8 B-1), so re-resolving it would
/// ask a second owner the question the mount table already answered.
fn build_docs_at(root: &Path) -> Result<BTreeMap<String, Document>, Fail> {
    let root = fs::WorkspaceRoot(root.to_path_buf());
    let (files, _fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the mounted corpus: {e}")))?;
    let (_index, docs) = fs::build_corpus(files)
        .map_err(|e| Fail::tool(format!("cannot build the mounted corpus: {e}")))?;
    Ok(docs)
}

/// The parsed `walk` invocation: the root page, direction, optional depth bound,
/// output format.
#[derive(Debug)]
struct Walk {
    page: String,
    direction: Direction,
    depth: Option<u32>,
    format: Format,
}

impl Walk {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut page: Option<String> = None;
        let mut direction = Direction::Up;
        let mut depth: Option<u32> = None;
        let mut json = false;

        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--down" => direction = Direction::Down,
                "--json" => json = true,
                "--depth" => {
                    let raw = it
                        .next()
                        .ok_or_else(|| Fail::tool("--depth needs a value".to_owned()))?;
                    depth = Some(parse_depth(raw)?);
                }
                flag if flag.starts_with("--depth=") => {
                    depth = Some(parse_depth(&flag["--depth=".len()..])?);
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if page.is_none() => page = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }

        let page = page.ok_or_else(|| {
            Fail::tool("walk needs a PAGE (a workspace-relative selector)".to_owned())
        })?;
        Ok(Walk {
            page,
            direction,
            depth,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// Parse a `--depth` value: a non-negative integer.
fn parse_depth(raw: &str) -> Result<u32, Fail> {
    raw.parse::<u32>()
        .map_err(|_| Fail::tool(format!("--depth expects a non-negative integer, got {raw}")))
}

/// Build the corpus in-process from the workspace on disk — the same U1 builder
/// (`fs::build_corpus`) the daemon and the `links` degrade use, so the walk reads
/// exactly the served projection. The wire/daemon walk leg is U3.1.
pub(crate) fn build_docs(workspace: &Path) -> Result<BTreeMap<String, Document>, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let (files, _fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let (_index, docs) =
        fs::build_corpus(files).map_err(|e| Fail::tool(format!("cannot build the corpus: {e}")))?;
    Ok(docs)
}

/// Map a [`WalkError`] to the exit-2 tool failure with a teaching diagnostic.
fn walk_error(error: WalkError) -> Fail {
    match error {
        WalkError::RootNotFound(page) => Fail::tool(format!("walk root not in the corpus: {page}")),
        WalkError::Cycle(loop_pages) => {
            Fail::tool(format!("in-snapshot cycle: {}", loop_pages.join(" -> ")))
        }
    }
}

/// Render the listing as an indented human block: the header, one line per
/// entry (`depth N  <color>  <selector>  rev=<rev>`), then the rev citations.
fn render_human(report: &WalkReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    match report.depth_bound {
        Some(bound) => {
            let _ = writeln!(
                out,
                "walk {} {} (depth <= {bound})",
                report.direction.label(),
                report.root
            );
        }
        None => {
            let _ = writeln!(out, "walk {} {}", report.direction.label(), report.root);
        }
    }
    if report.entries.is_empty() {
        let _ = writeln!(out, "  (nothing)");
    } else {
        for entry in &report.entries {
            let rev = entry.rev.as_deref().unwrap_or("-");
            let _ = writeln!(
                out,
                "  depth {}  {}  {}  rev={rev}",
                entry.depth,
                walk::color_label(&entry.color),
                entry.selector,
            );
            // S3-R51 — the teaching refusal, indented beneath the row it
            // explains. The house pattern, copied from `mrd config`, which
            // already prints `MountState::detail()` under each mount rather
            // than inventing a second layout for the same job.
            if let Some(teaching) = walk::color_teaching(&entry.color, &entry.selector) {
                let _ = writeln!(out, "      {teaching}");
            }
        }
    }
    let _ = writeln!(out, "revs-read:");
    for cite in &report.revs_read {
        let _ = writeln!(out, "  {} @ {}", cite.path, cite.doc_rev);
    }
    out
}

/// The `--json` shape: the workspace plus the walk object (direction, root,
/// depth bound, entries, rev citations). Colors split into a stable
/// `color`/`reason` pair plus the reason's `detail` — which fingerprint-triple
/// member is unknown, or why a lock refused — so a machine reader sees exactly
/// what [`walk::color_label`] renders for a human, never a reason word stripped
/// of the damage it names.
fn to_json(workspace: &Path, report: &WalkReport) -> Value {
    let entries: Vec<Value> = report
        .entries
        .iter()
        .map(|entry| {
            json!({
                "depth": entry.depth,
                "selector": entry.selector,
                "rev": entry.rev,
                "color": walk::color_tone(&entry.color),
                "reason": walk::color_reason(&entry.color),
                "detail": walk::color_detail(&entry.color),
                // S3-R51 — the teaching refusal now has an output path. `null`
                // for every color that teaches nothing, so the field never
                // invents advice it does not have.
                "teaching": walk::color_teaching(&entry.color, &entry.selector),
            })
        })
        .collect();
    let revs_read: Vec<Value> = report
        .revs_read
        .iter()
        .map(|cite| json!({ "path": cite.path, "doc_rev": cite.doc_rev }))
        .collect();
    json!({
        "workspace": workspace.display().to_string(),
        "walk": {
            "direction": report.direction.label(),
            "root": report.root,
            "depth_bound": report.depth_bound,
            "entries": entries,
            "revs_read": revs_read,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::selector::{Color, GreyReason, RedReason};
    use view::walk::{RevCitation, WalkEntry};

    fn report() -> WalkReport {
        WalkReport {
            direction: Direction::Up,
            root: "a.md".to_string(),
            depth_bound: None,
            entries: vec![
                WalkEntry {
                    selector: "b.md".to_string(),
                    rev: Some("bbbbbbbbbbbbbbbb".to_string()),
                    color: Color::Green,
                    depth: 1,
                },
                WalkEntry {
                    selector: "c.md".to_string(),
                    rev: Some("cccccccccccccccc".to_string()),
                    color: Color::Green,
                    depth: 2,
                },
            ],
            revs_read: vec![
                RevCitation {
                    path: "a.md".to_string(),
                    doc_rev: "aaaaaaaaaaaaaaaa".to_string(),
                },
                RevCitation {
                    path: "b.md".to_string(),
                    doc_rev: "bbbbbbbbbbbbbbbb".to_string(),
                },
                RevCitation {
                    path: "c.md".to_string(),
                    doc_rev: "cccccccccccccccc".to_string(),
                },
            ],
        }
    }

    /// The human render is byte-expected: header, depth-tagged entries with color
    /// and rev, then the rev citations (§2.4).
    #[test]
    fn render_human_is_byte_expected() {
        let expected = "\
walk up a.md
  depth 1  green  b.md  rev=bbbbbbbbbbbbbbbb
  depth 2  green  c.md  rev=cccccccccccccccc
revs-read:
  a.md @ aaaaaaaaaaaaaaaa
  b.md @ bbbbbbbbbbbbbbbb
  c.md @ cccccccccccccccc
";
        assert_eq!(render_human(&report()), expected);
    }

    /// A depth bound renders in the header; a grey entry carrying no rev prints
    /// `rev=-` and the color reason. The subject is the RENDER SHAPE, so any
    /// reason-carrying grey serves — this one is `immutable-root`, whose reason
    /// word stands alone with no detail to compose.
    #[test]
    fn render_human_bounded_and_grey() {
        let mut r = report();
        r.direction = Direction::Down;
        r.depth_bound = Some(1);
        r.entries = vec![WalkEntry {
            selector: "b.md".to_string(),
            rev: None,
            color: Color::Grey(GreyReason::ImmutableRoot),
            depth: 1,
        }];
        r.revs_read = vec![RevCitation {
            path: "a.md".to_string(),
            doc_rev: "aaaaaaaaaaaaaaaa".to_string(),
        }];
        let expected = "\
walk down a.md (depth <= 1)
  depth 1  grey immutable-root  b.md  rev=-
revs-read:
  a.md @ aaaaaaaaaaaaaaaa
";
        assert_eq!(render_human(&r), expected);
    }

    /// An empty walk renders `(nothing)` and still cites the root rev.
    #[test]
    fn render_human_empty_walk() {
        let mut r = report();
        r.entries.clear();
        r.revs_read = vec![RevCitation {
            path: "a.md".to_string(),
            doc_rev: "aaaaaaaaaaaaaaaa".to_string(),
        }];
        let expected = "\
walk up a.md
  (nothing)
revs-read:
  a.md @ aaaaaaaaaaaaaaaa
";
        assert_eq!(render_human(&r), expected);
    }

    /// `--json` carries the split color/reason and the depth-tagged entries.
    #[test]
    fn json_shape_splits_color_and_reason() {
        let mut r = report();
        r.entries[1].color = Color::Red(RedReason::Drifted);
        let value = to_json(Path::new("/ws"), &r);
        let entries = value["walk"]["entries"].as_array().unwrap();
        assert_eq!(entries[0]["color"], json!("green"));
        assert_eq!(entries[0]["reason"], Value::Null);
        assert_eq!(entries[1]["color"], json!("red"));
        assert_eq!(entries[1]["reason"], json!("content-drifted"));
        assert_eq!(entries[1]["depth"], json!(2));
        assert_eq!(value["walk"]["direction"], json!("up"));
    }

    #[test]
    fn parse_accepts_down_depth_and_json() {
        let w = Walk::parse(&[
            "notes/a.md".to_string(),
            "--down".to_string(),
            "--depth".to_string(),
            "1".to_string(),
            "--json".to_string(),
        ])
        .expect("parse");
        assert_eq!(w.page, "notes/a.md");
        assert!(matches!(w.direction, Direction::Down));
        assert_eq!(w.depth, Some(1));
        assert!(matches!(w.format, Format::Json));
    }

    #[test]
    fn parse_accepts_depth_equals_form() {
        let w = Walk::parse(&["a.md".to_string(), "--depth=3".to_string()]).expect("parse");
        assert_eq!(w.depth, Some(3));
    }

    #[test]
    fn parse_requires_a_page() {
        let err = Walk::parse(&["--down".to_string()]).expect_err("no page");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn parse_rejects_unknown_flag_and_bad_depth() {
        assert_eq!(
            Walk::parse(&["a.md".to_string(), "--nope".to_string()])
                .unwrap_err()
                .code,
            2
        );
        assert_eq!(
            Walk::parse(&["a.md".to_string(), "--depth".to_string(), "x".to_string()])
                .unwrap_err()
                .code,
            2
        );
    }
}
