//! `mrd walk` — the context-assembly listing (§2.4 / §3).
//!
//! ```text
//! mrd walk <PAGE> [--down] [--depth N] [--json]
//! ```
//!
//! Up (default) = ancestors, the context walk: what PAGE draws from, transitively. `--down` =
//! descendants, the dependents renderer and dry-run blast radius; `--depth 1` = exactly the
//! direct dependents. Every listing entry is `{selector, rev, color, depth}` and every answer
//! cites the doc revs it read.
//!
//! Read-only: writes nothing, mints no receipt — the walk is computed per query, never stored.
//! Output is the house grammar: a human listing by default, JSON under `--json`. Exit triad:
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

/// The finding leg of the triad: the invocation was well-formed, the listing carries a red edge.
const EXIT_FINDING: u8 = 1;

/// Run `mrd walk <PAGE> [--down] [--depth N] [--json]`: resolve the workspace, build the corpus
/// in-process, walk the pin graph, and print the listing. Errors [`Fail`] exit 2 on a bad
/// invocation, an unreadable workspace/corpus, a missing root, or an in-snapshot cycle; exit 1
/// when the listing carries a red edge.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Walk::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let InProcessCorpus {
        root,
        mut docs,
        unserved,
    } = build_corpus_in_process(&resolved.workspace)?;
    admit_named_page(&root, &mut docs, &parsed.page);
    let docs = docs;

    // `--down` answers a POPULATION the caller did not name — the blast radius — so it owes the
    // enumerator clause: it may exclude what its attestation cannot reach, never SILENTLY
    // (`crate::voice_excluded`, §12.1). Voiced AFTER `admit_named_page`, or the door's own named
    // subject reads as an exclusion. `--up` is not gated in: it drops nothing, naming an
    // excluded ancestor by its correct path at a red edge (measured, `1ee5317a`).
    if matches!(parsed.direction, Direction::Down) {
        crate::voice_excluded(&root, &docs, &unserved);
    }

    // The mount table, with a corpus for the roots this workspace's own lock addresses name and
    // no others. `mounts` owns the document maps; `corpus` borrows them — hence two bindings.
    let mounts = load_mounts_for(&lock_addressed_roots(&docs));
    let domain = load_domain(&resolved.workspace)?;
    // `root` is the one `build_corpus_in_process` returned above — the root this
    // corpus was BUILT from, and therefore the root its existence questions are
    // asked at (0045/0049), never the caller's cwd. Re-deriving it here would be
    // two answers to one fact inside one function.
    let corpus = mounts.rooted(&docs, &domain, &root);
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
    // Grey refuses on exit 1 with its own reason word; no fourth exit code. Scoped to the two
    // root greys.
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

/// Load `MERIDIAN.md`'s mount table into both halves resolution needs: one corpus per usable
/// root, and the projection that says what the file declares. A declared root that cannot be
/// used is carried with its state, never dropped.
///
/// Full-table face: every bound root gets a corpus. Kept for a caller that cannot compute a
/// needed set; every production verb today can, and uses [`load_mounts_for`].
#[allow(dead_code)] // full-table face — deliberate; no production caller remains after the residual narrow
pub(crate) fn load_mounts() -> Mounts {
    load_mounts_where(&|_| true)
}

/// [`load_mounts`], building a corpus only for the roots in `needed` — a corpus build costs the
/// root's size, and nothing about the workspace bounds it. Which roots are needed is the
/// caller's question: `status`/`walk`/`check` read them off lock items
/// ([`lock_addressed_roots`]); `links` and ephemeral `sql` off wikilink/embed targets
/// ([`link_addressed_roots`]). `needed` narrows the corpora, never the [`addr::MountSet`].
pub(crate) fn load_mounts_for(needed: &BTreeSet<addr::MountName>) -> Mounts {
    load_mounts_where(&|name| needed.contains(name))
}

/// Every mount root the corpus's `meridian-lock` addresses name — the exact set of roots worth
/// building. A pin's root is a property of its address, not of the tree it points into, so the
/// set is knowable from the ambient corpus alone. The root is read off
/// [`view::read_face::LockItem::declared_addr`], the structural owner, so nothing re-splits
/// `declared_ref`. A row with no address contributes no root.
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

/// Every mount root the corpus's wikilink/embed targets name — the set of roots whose pages the
/// link plane (and the SQL projection of it) can resolve into. Mounted root corpora exist so
/// `resolve_ref` can answer a rooted spelling, so a workspace carrying none needs zero.
///
/// `path` mirrors `query::links_rooted`: `None` scans the whole ambient corpus; `Some` scans
/// that one file. The root name is read from [`addr::Addr::parse`] of each target — the same
/// grammar the resolver peels. A target outside the grammar contributes no root.
pub(crate) fn link_addressed_roots(
    docs: &BTreeMap<String, Document>,
    path: Option<&str>,
) -> BTreeSet<addr::MountName> {
    let mut roots = BTreeSet::new();
    for (source, doc) in docs {
        if path.is_some_and(|p| p != source.as_str()) {
            continue;
        }
        collect_link_roots(&doc.root, &mut roots);
    }
    roots
}

fn collect_link_roots(node: &model::Node, roots: &mut BTreeSet<addr::MountName>) {
    match &node.kind {
        model::NodeKind::Wikilink { target, .. } | model::NodeKind::Embed { target, .. } => {
            if let Ok(addr) = addr::Addr::parse(target)
                && let Some(root) = addr.root()
            {
                roots.insert(root.clone());
            }
        }
        _ => {}
    }
    for child in &node.children {
        collect_link_roots(child, roots);
    }
}

/// The one loader both faces above are spellings of, so the table bind, the refusal carrying,
/// and the set assembly cannot drift between an eager caller and a narrowed one.
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
        // Declared, but the mount plane refuses it. Carried, with the path it declares and that
        // plane's own reason verbatim — never dropped.
        if mount.state().refuses() {
            // The raw filesystem reason where the mount plane has one; otherwise that plane's
            // teaching is the most specific thing available.
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
        // Bound per the table, corpus not asked for. It joins `bound` exactly as a built root
        // does: the table's answer does not depend on whether this process read its pages.
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
            // Bound per the table, but its corpus will not build — unreadable from here, and
            // just as much a declared root as any other.
            Err(e) => unreachable.push((name, path.display().to_string(), e.message)),
        }
    }

    let mut set = addr::MountSet::new(bound);
    for (name, path, detail) in unreachable {
        set = set.with_unreachable(name, path, detail);
    }
    Mounts { corpora, set }
}

/// The mount table as the pin planes consume it: the loaded corpora, and the projection naming
/// what the file declares and which of it is usable.
#[derive(Default)]
pub(crate) struct Mounts {
    corpora: Vec<MountedCorpus>,
    set: addr::MountSet,
}

impl Mounts {
    /// The root-keyed corpus over `docs`: the ambient workspace plus one root per bound mount.
    /// One owner for this assembly — `walk`, `check` and `status` all colour pins through the
    /// same computer, so they must hand it the same corpus.
    /// `domain` is the ambient workspace's hash domain, and it is REQUIRED
    /// rather than optional: a colour door that cannot say which paths its
    /// corpus was filtered by reds every out-of-domain target (decision 0034).
    /// Forcing it at the one assembly point is what keeps the three doors
    /// answering alike.
    /// `root` is REQUIRED for the same reason one level down (decision 0049): a
    /// colour door that knows a target is out of domain but cannot READ that
    /// target greys an absence, and the corpus map cannot tell the two apart.
    pub(crate) fn rooted<'a>(
        &'a self,
        docs: &'a BTreeMap<String, Document>,
        domain: &'a fs::domain::Domain,
        root: &'a fs::WorkspaceRoot,
    ) -> model::RootedCorpus<'a> {
        let mut corpus = model::RootedCorpus::ambient(docs)
            .with_hash_domain(domain)
            .with_ambient_disk(root);
        for mount in &self.corpora {
            corpus = corpus.with_root(mount.name.clone(), mount.kind.clone(), &mount.docs);
        }
        corpus
    }

    /// The projection naming what `MERIDIAN.md` declares and which of it is usable — the mount
    /// table resolution is a lookup in.
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

/// [`build_docs`] without the workspace-resolution wrapper — a mount path is already canonical
/// (canonicalize-at-bind), so re-resolving it would ask a second owner an answered question.
fn build_docs_at(root: &Path) -> Result<BTreeMap<String, Document>, Fail> {
    let root = fs::WorkspaceRoot(root.to_path_buf());
    let (files, _fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the mounted corpus: {e}")))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    Ok(docs)
}

/// The parsed `walk` invocation: the root page, direction, optional depth bound, output format.
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

/// Build the corpus in-process from the workspace on disk, through the same `fs::build_corpus`
/// the daemon and the `links` degrade use, so the walk reads exactly the served projection.
/// The workspace's hash domain — the filter [`build_docs`] projected, handed to
/// the colour plane so it can tell an excluded target from a missing one.
///
/// An unreadable domain config FAILS the door. Degrading to the default domain
/// would claim every path is hashed, which is exactly the false red decision
/// 0034 ruled out — a fail-open in the plane whose job is to be believed.
pub(crate) fn load_domain(workspace: &Path) -> Result<fs::domain::Domain, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    fs::domain::Domain::load(&fs::WorkspaceRoot(canonical))
        .map_err(|e| Fail::tool(format!("cannot read the hash domain: {e}")))
}

/// The canonical workspace root, resolved exactly as [`build_docs`] and
/// [`load_domain`] resolve it — the root the ambient corpus was built from, and
/// therefore the root its existence questions are asked at (decision 0049).
pub(crate) fn workspace_root(workspace: &Path) -> Result<fs::WorkspaceRoot, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    Ok(fs::WorkspaceRoot(canonical))
}

pub(crate) fn build_docs(workspace: &Path) -> Result<BTreeMap<String, Document>, Fail> {
    build_corpus_in_process(workspace).map(|corpus| corpus.docs)
}

/// The in-process corpus build, whole: what [`build_docs`] returns plus the two values it
/// drops. [`crate::voice_excluded`] takes all three, and a caller that only ever gets `docs`
/// back cannot name what the hash domain left out.
struct InProcessCorpus {
    /// The canonical workspace root the snapshot was taken at.
    root: fs::WorkspaceRoot,
    /// The served projection — the corpus the walk reads.
    docs: BTreeMap<String, Document>,
    /// Hash-domain members `fs::build_corpus` could not serve, by member and condition.
    unserved: BTreeMap<String, String>,
}

/// Build [`InProcessCorpus`] from the workspace on disk. [`build_docs`] is this with the two
/// extra values dropped.
fn build_corpus_in_process(workspace: &Path) -> Result<InProcessCorpus, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let (files, _fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    Ok(InProcessCorpus {
        root,
        docs,
        unserved,
    })
}

/// Fold a NAMED page the hash domain excludes into the corpus map before a door
/// answers about it.
///
/// `fs::build_corpus` projects the domain, and the domain gates HASHING, not
/// load: a door the caller names ONE path at serves it or is a door defect
/// (`docs/wire-contract.md` §12.1, the door-family clause). So membership decides
/// what an ENUMERATION walks, never what a named path is entitled to — the same
/// single-file load the read and write doors already run (registry
/// `doc_or_refusal`, `wire_serve::load_doc`), reached from the in-process faces.
///
/// A path with no file, or one whose bytes are not UTF-8, is left absent: the
/// caller's own miss diagnostic then says so, as it does for any other miss.
pub(crate) fn admit_named_page(
    root: &fs::WorkspaceRoot,
    docs: &mut BTreeMap<String, Document>,
    page: &str,
) {
    if docs.contains_key(page) {
        return;
    }
    if let Ok(doc) = fs::load(root, Path::new(page)) {
        docs.insert(page.to_owned(), doc);
    }
}

/// Map a [`WalkError`] to the exit-2 tool failure with a teaching diagnostic.
fn walk_error(error: WalkError) -> Fail {
    match error {
        // The refusal keeps its exact wording and gains its recovery only for
        // the enumeration gesture (laws.md § the face-honesty law, clause 3):
        // `mrd walk .` asks to see the corpus at a door that walks from one
        // page. A genuine missing page gets no pointer, because none is right.
        WalkError::RootNotFound(page) if crate::names_the_whole_corpus(&page) => Fail::tool(format!(
            "walk root not in the corpus: {page} — to list the corpus instead of walking from one \
             page, `mrd links --json` enumerates every file."
        )),
        WalkError::RootNotFound(page) => Fail::tool(format!("walk root not in the corpus: {page}")),
        WalkError::Cycle(loop_pages) => {
            Fail::tool(format!("in-snapshot cycle: {}", loop_pages.join(" -> ")))
        }
    }
}

/// Render the listing as an indented human block: the header, one line per entry
/// (`depth N  <color>  <selector>  rev=<rev>`), then the rev citations.
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
            // The teaching refusal, indented beneath the row it explains.
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

/// The `--json` shape: the workspace plus the walk object (direction, root, depth bound,
/// entries, rev citations). Colors split into a stable `color`/`reason` pair plus the reason's
/// `detail`, so a machine reader sees exactly what [`walk::color_label`] renders for a human.
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
                // `null` for every color that teaches nothing, so the field never invents advice.
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

    /// The human render is byte-expected: header, depth-tagged entries with color and rev, then
    /// the rev citations.
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

    /// A depth bound renders in the header; a grey entry carrying no rev prints `rev=-` and the
    /// color reason.
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
