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

use std::collections::BTreeSet;
use std::path::Path;

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
    let mut parsed = Walk::parse(args)?;
    let cwd = current_dir()?;
    // The rooted lane (§4.1 colon law): a head-colon PAGE is an agent-plane
    // address, never a literal path — resolve it to the named root's bound
    // workspace and walk THERE. The ambient lane resolves from cwd exactly as
    // before. A resolution refusal is the address answer (exit 1, the
    // `{workspace, error}` frame under `--json`), never a literal-path walk.
    let workspace = match crate::rooted::enter(&parsed.page, "walk", "Nothing was walked.") {
        Ok(Some((rel, rooted))) => {
            // The CANONICAL spelling, not the caller's bytes (§ 4.6a). Both
            // faces, deliberately: `walk.root` is a STRUCTURED field a consumer
            // parses with nothing to recover from, and splitting the law — JSON
            // canonical, human as-typed — would mean two answers to "which root
            // did I walk" from one command. One law is cheaper to hold than an
            // exception, and the alias is still visible where it belongs
            // (`mrd resolve`, the `mounts` row).
            parsed.display = Some(rooted.canonical_ref(&rel));
            parsed.page = rel;
            parsed.rooted = Some(rooted.name.clone());
            rooted.workspace
        }
        Ok(None) => ambient_workspace(&cwd)?,
        // The refusal frames with the workspace the caller stands in — no
        // target workspace exists to name.
        Err(error) => {
            let ambient = ambient_workspace(&cwd)?;
            return Err(crate::engine::json_refusal(parsed.format, &ambient, &error));
        }
    };
    // §1 admission, before any corpus is read: without it `admit_named_page`'s
    // `fs::load` resolves an absolute spelling verbatim and this door WALKS a
    // page from outside the root (wire-contract §12.1, the door-family clause).
    // On the rooted lane the rel half is already confined ([`crate::rooted`]),
    // so this admission is a no-op pass there.
    crate::path_law::admit(&workspace, &parsed.page, "walk", "Nothing was walked.")?;
    let InProcessCorpus { root, mut docs } = build_corpus_in_process(&workspace)?;
    admit_named_page(&root, &mut docs, &parsed.page);
    let docs = docs;

    // `--down` answers a POPULATION the caller did not name — the blast radius — so it owes the
    // enumerator clause: it may exclude what its attestation cannot reach, never SILENTLY
    // (`crate::voice_excluded`, §12.1). Voiced AFTER `admit_named_page`, or the door's own named
    // subject reads as an exclusion. `--up` is not gated in: it drops nothing, naming an
    // excluded ancestor by its correct path at a red edge (measured, `1ee5317a`).
    if matches!(parsed.direction, Direction::Down) {
        crate::voice_excluded(&root, &docs);
    }

    // The mount table, with a corpus for the roots this workspace's own lock addresses name and
    // no others. `mounts` owns the document maps; `corpus` borrows them — hence two bindings.
    let mounts = load_mounts_for(&lock_addressed_roots(&docs));
    let domain = load_domain(&workspace)?;
    // `root` is the one `build_corpus_in_process` returned above — the root this
    // corpus was BUILT from, and therefore the root its existence questions are
    // asked at (0045/0049), never the caller's cwd. Re-deriving it here would be
    // two answers to one fact inside one function.
    let corpus = mounts.rooted(&docs, &domain, &root);
    let mount_set = mounts.set();

    let mut report = walk::walk_rooted(
        &corpus,
        mount_set,
        &parsed.page,
        parsed.direction,
        parsed.depth,
    )
    .map_err(|e| walk_error(e, &workspace, &cwd, &parsed))?;
    // The echo law at the face: the walk root renders rooted where the caller
    // wrote rooted, with the root half CANONICAL (§ 4.6a — an alias is a lookup
    // spelling). Entries are untouched — as-if-cd'd into the named root (see
    // [`Walk`]).
    if let Some(display) = &parsed.display {
        report.root.clone_from(display);
    }

    match parsed.format {
        Format::Json => {
            let value = to_json(&workspace, &report);
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

/// The shared mount-table corpus assembly (`wire_serve::mount_corpus` — one
/// owner across the CLI pin planes and the § A.10 wire serve path), with the
/// CLI's voicing folded on: the shared loader returns each mounted root's
/// unserved members, and this face speaks them to stderr.
pub(crate) type Mounts = wire_serve::mount_corpus::MountCorpora;

/// Full-table face: every bound root gets a corpus. Kept for a caller that cannot compute a
/// needed set; every production verb today can, and uses [`load_mounts_for`].
#[allow(dead_code)] // full-table face — deliberate; no production caller remains after the residual narrow
pub(crate) fn load_mounts() -> Mounts {
    voiced(wire_serve::mount_corpus::load_mounts_where(&|_| true))
}

/// [`load_mounts`], building a corpus only for the roots in `needed` — a corpus build costs the
/// root's size, and nothing about the workspace bounds it. Which roots are needed is the
/// caller's question: `status`/`walk`/`check` read them off lock items
/// ([`lock_addressed_roots`]); `links` and ephemeral `sql` off wikilink/embed targets
/// ([`link_addressed_roots`]). `needed` narrows the corpora, never the [`addr::MountSet`].
pub(crate) fn load_mounts_for(needed: &BTreeSet<addr::MountName>) -> Mounts {
    voiced(wire_serve::mount_corpus::load_mounts_for(needed))
}

/// Voice each mounted root's unserved members — the shared loader returns
/// them (voicing is the caller's; the wire host has no stderr), the CLI
/// speaks them exactly as it always has.
fn voiced(mounts: Mounts) -> Mounts {
    for corpus in &mounts.corpora {
        crate::voice_unserved(&corpus.unserved);
    }
    mounts
}

/// Every mount root the corpus's `meridian-lock` addresses name — moved to
/// the walk plane ([`view::walk::lock_addressed_roots`]) so the CLI verbs and
/// the wire serve path read ONE owner; this spelling stays for the in-crate
/// callers (`check`/`status`).
pub(crate) fn lock_addressed_roots(docs: &model::Docs) -> BTreeSet<addr::MountName> {
    walk::lock_addressed_roots(docs)
}

/// Every mount root the corpus's wikilink/embed targets name — moved to the
/// walk plane ([`view::walk::link_addressed_roots`]) beside its lock-address
/// sibling, so the CLI verbs and the § A.11 wire serve path read ONE owner;
/// this spelling stays for the in-crate callers.
pub(crate) fn link_addressed_roots(
    docs: &model::Docs,
    path: Option<&str>,
) -> BTreeSet<addr::MountName> {
    walk::link_addressed_roots(docs, path)
}

/// The parsed `walk` invocation: the root page, direction, optional depth bound, output format.
#[derive(Debug)]
struct Walk {
    page: String,
    direction: Direction,
    depth: Option<u32>,
    format: Format,
    /// The rooted lane's typed spelling (`root:rel`) — the walk root the face
    /// echoes, so the caller sees what they wrote (the read door's echo law).
    /// `None` on the ambient lane. Entries stay as-if-cd'd into the named
    /// root: bare for that root's own pages, root-qualified for foreign pins —
    /// exactly the render the same walk gives from inside the root.
    display: Option<String>,
    /// The named root — puts the miss teaching on the rooted branch (scoped
    /// to that root, no ambient cwd respelling). `None` on the ambient lane.
    rooted: Option<addr::MountName>,
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
            display: None,
            rooted: None,
        })
    }
}

/// The ambient workspace for `cwd`, per the settled resolution ladder — the
/// lane every walk took before the rooted lane existed (byte-identical to the
/// read door's helper, the doors sharing the lane).
fn ambient_workspace(cwd: &Path) -> Result<std::path::PathBuf, Fail> {
    let resolved = crate::resolve::resolve_runtime(workspace::Base::Cwd(cwd)).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    Ok(resolved.workspace)
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

pub(crate) fn build_docs(workspace: &Path) -> Result<model::Docs, Fail> {
    build_corpus_in_process(workspace).map(|corpus| corpus.docs)
}

/// The in-process corpus build, whole: what [`build_docs`] returns plus the root it drops.
/// [`crate::voice_excluded`] takes both — the root to enumerate the declined class from, the
/// docs to keep an admitted named page out of the voice. The unserved map never leaves the
/// build: it is voiced right where it is minted ([`crate::voice_unserved`]), and the declined
/// class it used to help subtract is structurally disjoint from it.
struct InProcessCorpus {
    /// The canonical workspace root the snapshot was taken at.
    root: fs::WorkspaceRoot,
    /// The served projection — the corpus the walk reads.
    docs: model::Docs,
}

/// Build [`InProcessCorpus`] from the workspace on disk. [`build_docs`] is this with the
/// root dropped.
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
    Ok(InProcessCorpus { root, docs })
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
pub(crate) fn admit_named_page(root: &fs::WorkspaceRoot, docs: &mut model::Docs, page: &str) {
    if docs.contains_key(page) {
        return;
    }
    if let Ok(doc) = fs::load(root, Path::new(page)) {
        docs.insert(page.to_owned(), std::sync::Arc::new(doc));
    }
}

/// Map a [`WalkError`] to the exit-2 tool failure with a teaching diagnostic.
fn walk_error(error: WalkError, workspace: &Path, cwd: &Path, parsed: &Walk) -> Fail {
    match error {
        // The refusal keeps its exact wording and gains its recovery only for
        // the enumeration gesture (laws.md § the face-honesty law, clause 3):
        // `mrd walk *` asks to see the corpus at a door that walks from one
        // page (`.` and `` refuse at the §1 admission before reaching here).
        // A genuine missing page gets no pointer, because none is right.
        WalkError::RootNotFound(page) if crate::names_the_whole_corpus(&page) => {
            Fail::tool(format!(
                "walk root not in the corpus: {page} — to list the corpus instead of walking from one \
             page, `mrd links --json` enumerates every file."
            ))
        }
        // The rooted miss is scoped to the NAMED root (F4): it echoes the
        // caller's spelling, names which root was searched, and never carries
        // the ambient cwd respelling — advice for a different mistake.
        WalkError::RootNotFound(page) if parsed.rooted.is_some() => {
            let name = parsed.rooted.as_ref().expect("guarded by arm");
            Fail::tool(format!(
                "walk root not in the corpus: {} — no page `{page}` in root `{name}` \
                 (workspace {})",
                parsed.display.as_deref().unwrap_or(&page),
                workspace.display()
            ))
        }
        // The ambient miss carries the family's fitted respelling when it is
        // earned — the same sentence, the same one computation, as the read
        // door's.
        WalkError::RootNotFound(page) => {
            let mut m = format!("walk root not in the corpus: {page}");
            if let Some(suffix) = crate::path_law::cwd_respell_suffix(workspace, cwd, &page) {
                m.push_str(&suffix);
            }
            Fail::tool(m)
        }
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
