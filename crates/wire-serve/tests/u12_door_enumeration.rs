//! **U12's door enumeration, keyed by SITE and bound PER DOOR.**
//!
//! U31 made the candidate document a TYPE the `fs` byte-landing primitives
//! demand, so *"which doors land bytes"* stopped being prose and became *"which
//! sites mint a `model::CandidateDocument`"* — a set the compiler maintains.
//! This test pins that set and states, per door, what U12 does at it.
//!
//! **Amended by C3 (2026-08-01): that equivalence is now ONE-directional.** Every
//! byte landing still mints a candidate, but a mint is no longer proof of one —
//! `candidate_of_body` is also the only production mint that gives a document its
//! own PATH, and the reaction feeder needs that to match a HOOK's `paths:` scope
//! without landing anything. Such a site is pinned like every other, in the
//! [`Door::ReadOnly`] class, which owes the reader why its value cannot land. The
//! census still measures the same set from the tree; what changed is that
//! membership no longer classifies a site on its own.
//!
//! **Why a test and not the compiler.** U31's own rung is compiler-enumerated
//! and this unit inherits its result; what the compiler cannot say is whether a
//! door that HOLDS a candidate also GUARDS it, because a guard is a call, not a
//! type. So this is the ladder's middle rung, used exactly where a list is all a
//! test can give you.
//!
//! # What F2 found, and what the key had to become (S3-R82)
//!
//! The first version of this census pinned the same data it does now — the
//! `(file, function)` pair was already in `DOORS` — and then **compared only the
//! FILE PATHS**. Eight doors over three files, so the assertion compared
//! `{write.rs, realise_cmd.rs, fp.rs}` against itself: **a ninth mint in any
//! file it already knew moved neither side.** Its guard arm was a **bag count** —
//! `stored_form_guard_lazy(` occurrences against a count of `DOORS` rows — so
//! deleting the guard at the `create` birth door and duplicating one at `splice`
//! left the total unchanged and the gate green, with a `TranslatedAndGuarded`
//! door landing bytes unguarded.
//!
//! Both evasions were measured GREEN on the shipped census
//! (`results/f2-door-census-mutation-harness.sh`, `MUT_Ma_EXIT=0`,
//! `MUT_Mb_EXIT=0` at `797c4e8e`), and the fix is not a wider scan — it is the
//! ASSERTION meeting the granularity the data always had:
//!
//! - **the census key is the SITE** — `(file, mint_fn)`, never the file;
//! - **each site's mint count is pinned**, so a second mint inside a function the
//!   census already knows is a change too;
//! - **the guard is bound to the door that owes it** — per-site discharge, never
//!   a total.
//!
//! **Keyed by `file::function`, never by line**, which was right the first time
//! and is unchanged: a line number rots on the next edit and a rotted pin teaches
//! a reader to ignore the check. *Measured before re-keying: all eight mints sit
//! in eight DISTINCT functions, so no site needs a line number to be named.*
//!
//! # The partition the key stands on (the second half of F2)
//!
//! Re-keying the census left it standing on a broken production/test partition,
//! and **all four of the new site-keyed reads went through it** — so the fix was
//! green on its own terms and blind in the join. The old partition truncated the
//! file at the FIRST literal `#[cfg(test)]`, which is sound only if that marker
//! always opens a trailing test module. **Rust allows the attribute at ITEM
//! level, and the counter-example ships here:** `crates/policy/src/pack.rs` is
//! 1212 lines, its first marker at :214 is an item attribute on
//! `pub(crate) fn facts_from_markdown`, and the real `mod tests` does not open
//! until :841 — so truncating kept **213 lines and discarded 999 of 1212
//! (82.4%)**, roughly twenty production functions, in which a door would have
//! been invisible and the gate GREEN.
//!
//! **Measured population, so this partition covers the shapes that exist rather
//! than the shapes it imagines: 74 markers across 69 production files — 72 gate a
//! `mod`, 2 gate a `fn` (both in `pack.rs`), and exactly ONE file's FIRST marker
//! is item-level.** Three gated shapes exist: `mod name {`, `fn name(…) {`, and
//! `mod tests;` (semicolon, no body — measured as the LAST line of all three
//! files that use it, so gating it must not hunt a brace that does not exist).
//!
//! **Bound, stated rather than overclaimed: this was LATENT, not an open hole.**
//! `pack.rs` mints zero candidates anywhere in the file, and the markers in
//! `write.rs` and `fs/src/lib.rs` do open trailing test modules, so their
//! below-marker mints were discarded correctly and still are. Population intact,
//! instrument broken — the same shape as the keying defect beside it.
//!
//! # Precision, measured before the check was written (S3-R23 ①)
//!
//! The scan reads production `src/` only, skips doc comments and the definition
//! site itself, and classifies every hit. A new mint anywhere in the workspace is
//! reported as *unclassified*, never guessed at — the false positive that would
//! get this instrument deleted.
//!
//! **Every unrecognised shape resolves toward NOT gating, deliberately.** Test
//! code read as production reddens LOUDLY as an unpinned site; production code
//! read as test goes INVISIBLE. Only the second direction agrees with the failure
//! this census exists to catch, so the ambiguous cases are pushed into the first
//! (S3-R25 on error direction). One consequence is worth naming: `config`'s tests
//! are gated in the PARENT module (`mod tests;`), so `crates/config/src/tests.rs`
//! carries no marker of its own and is read whole as production source. Measured:
//! it mints zero candidates, and a mint appearing there would redden as an
//! unpinned site rather than hide.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A door's identity: the workspace-relative file, and the function whose body
/// holds the `candidate_of_*` call. **This pair is the census key** — the unit F2
/// found the assertion had dropped.
type Site = (String, String);

const WRITE_RS: &str = "crates/wire-serve/src/write.rs";

/// The scope recorded for a mint sitting outside every function. It can match no
/// pinned site, so such a mint fails LOUD as unclassified rather than being
/// attributed to a neighbour.
const FILE_SCOPE: &str = "<file scope: no enclosing fn>";

/// What U12 does at a byte-landing door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Door {
    /// The candidate is TRANSLATED (agent-plane addresses this write introduces
    /// become their `obsidian://` stored form) and then GUARDED.
    TranslatedAndGuarded,
    /// The candidate is GUARDED. Its bytes are the engine's own — a lock block
    /// (positions 3 and 4, where the translation is the identity by ratified
    /// law) or an anchor promotion — so there is nothing to translate, and the
    /// guard is what says so rather than a comment claiming it.
    Guarded,
    /// **NOT COVERED BY THIS UNIT**, stated rather than absorbed (S3-R4). The
    /// door lives outside U12's named files (`write.rs` + `read.rs`), and
    /// closing it would mean a SECOND transform, which this unit's card
    /// forbids. Reported to the leader as a gap with its population.
    OutsideThisUnit,
    /// **Not a door: the mint lands no bytes.** The site mints a candidate to
    /// give a document its own PATH, reads it, and drops it — the value never
    /// reaches an `fs` byte-landing primitive.
    ///
    /// This class exists because it falsifies the equivalence this census was
    /// built on (module header: *"which doors land bytes … became which sites
    /// mint a `model::CandidateDocument`"*). That was true while the candidate
    /// type had exactly one use. C3's reaction feeder needs a document's path to
    /// match a HOOK's `paths:` scope, and `candidate_of_body` is the production
    /// mint that carries one, so a read-only caller now mints too.
    ///
    /// **The equivalence is one-directional from here on:** every byte landing
    /// still mints, but a mint is no longer proof of a byte landing. A site
    /// entering this class owes the reader the reason its value cannot land —
    /// for the one member, `feed_landed_change` takes `&Document` and returns
    /// effects, so there is no path from the mint to a write.
    ReadOnly,
}

/// One pinned byte-landing door.
struct DoorPin {
    /// Workspace-relative path of the file carrying the mint.
    file: &'static str,
    /// **The door** — the function a caller enters. For four of the five
    /// `write.rs` doors this is also where the mint and the guard sit; `splice`
    /// is the one that spans three functions, which is why the door and its mint
    /// are separate fields rather than one.
    door_fn: &'static str,
    /// **The mint site** — the function whose body holds the `candidate_of_*`
    /// call. With `file` this is the census key, because it is the thing the scan
    /// can measure.
    mint_fn: &'static str,
    /// **The discharge site** — the function calling `stored_form_guard_lazy` for
    /// THIS door, or `None` for a door outside this unit. `splice` discharges in
    /// `translate_stored_candidate`; the other four discharge in themselves.
    guard_fn: Option<&'static str>,
    /// What a reader calls this door. Prose, never matched against the tree.
    label: &'static str,
    class: Door,
}

/// **THE DOOR LIST** — every production site minting a `model::CandidateDocument`,
/// with what U12 does there and where its guard is discharged.
const DOORS: &[DoorPin] = &[
    // ---- wire-serve/write.rs — U12's own file, all five guarded ----
    DoorPin {
        file: WRITE_RS,
        door_fn: "splice",
        mint_fn: "build_after_doc",
        guard_fn: Some("translate_stored_candidate"),
        label: "splice (via build_after_doc)",
        class: Door::TranslatedAndGuarded,
    },
    DoorPin {
        file: WRITE_RS,
        door_fn: "create",
        mint_fn: "create",
        guard_fn: Some("create"),
        label: "create (the birth door)",
        class: Door::TranslatedAndGuarded,
    },
    DoorPin {
        file: WRITE_RS,
        door_fn: "lock_write",
        mint_fn: "lock_write",
        guard_fn: Some("lock_write"),
        label: "lock_write",
        class: Door::Guarded,
    },
    DoorPin {
        file: WRITE_RS,
        door_fn: "plan_promotion",
        mint_fn: "plan_promotion",
        guard_fn: Some("plan_promotion"),
        label: "plan_promotion (the anchor promotion)",
        class: Door::Guarded,
    },
    DoorPin {
        file: WRITE_RS,
        door_fn: "commit_batch",
        mint_fn: "commit_batch",
        guard_fn: Some("commit_batch"),
        label: "commit_batch (the public commit seam)",
        class: Door::Guarded,
    },
    // ---- outside U12's named files ----
    DoorPin {
        file: "crates/mrd/src/realise_cmd.rs",
        door_fn: "truth_deploy",
        mint_fn: "truth_deploy",
        guard_fn: None,
        label: "realise --truth file: the armed policy INDEX",
        class: Door::OutsideThisUnit,
    },
    DoorPin {
        file: "crates/mrd/src/realise_cmd.rs",
        door_fn: "body_rev",
        mint_fn: "body_rev",
        guard_fn: None,
        // The shipped label read "realise: the convergence body"; the function it
        // names mints to take the whole-file rev of the INDEX pre-image the
        // convergence is decided against. Both spellings are kept so the older
        // one stays greppable (S3-R65(d): no silent re-wording of a shipped name).
        label: "realise: the convergence body — body_rev's whole-file rev of the INDEX pre-image",
        class: Door::OutsideThisUnit,
    },
    DoorPin {
        file: "crates/run/src/fp.rs",
        door_fn: "candidate",
        mint_fn: "candidate",
        guard_fn: None,
        label: "the run plane's candidate (run::fp::candidate)",
        class: Door::OutsideThisUnit,
    },
    // ---- mrd/journal_cmd.rs — G2's genesis, two mints in two functions ----
    // Both land the ENGINE's own bytes on a CLI path, the same shape as the
    // realise doors above: outside `write.rs`/`read.rs`, so outside this unit
    // and stated rather than absorbed. Neither carries agent-plane addresses —
    // one is a header the verb composes, the other is the empty string.
    DoorPin {
        file: "crates/mrd/src/journal_cmd.rs",
        door_fn: "genesis",
        mint_fn: "write_archive",
        guard_fn: None,
        label: "journal genesis: the archive page the reset moves rows into",
        class: Door::OutsideThisUnit,
    },
    DoorPin {
        file: "crates/mrd/src/journal_cmd.rs",
        door_fn: "genesis",
        mint_fn: "truncate_journal",
        guard_fn: None,
        label: "journal genesis: emptying the live journal after the archive is durable",
        class: Door::OutsideThisUnit,
    },
    // ---- sidecar/watch.rs — C3's reaction feeder, the first mint that is NOT
    // a door. `external_effects` needs each externally-changed document to carry
    // its own path, because a HOOK matches `paths:` against it; the watcher's
    // other mint (`doc_of`) leaves the path empty. The candidate is read by
    // `feed_landed_change` and dropped — no `fs` primitive ever sees it.
    DoorPin {
        file: "crates/sidecar/src/watch.rs",
        door_fn: "external_effects",
        mint_fn: "doc_at",
        guard_fn: None,
        label: "the reaction feeder's path-carrying read (lands no bytes)",
        class: Door::ReadOnly,
    },
];

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/wire-serve`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every production `src/` file in the workspace — **except `model`, which
/// DEFINES the two constructors**: its own internal delegation
/// (`candidate_of_batch` calls `candidate_of_body`) is the definition, not a
/// door, and a definition counted as a door is the false positive that gets an
/// instrument deleted.
fn production_sources() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut out = Vec::new();
    let Ok(crates) = fs::read_dir(root.join("crates")) else {
        panic!("crates/ must be readable");
    };
    for entry in crates.flatten() {
        if entry.file_name() == "model" {
            continue;
        }
        let src = entry.path().join("src");
        if src.is_dir() {
            rs_files(&src, &mut out);
        }
    }
    out.sort();
    out
}

/// One line of a source file, with the innermost function enclosing it and
/// whether it sits inside a `#[cfg(test)]`-gated item.
struct Scoped<'a> {
    text: &'a str,
    function: Option<&'a str>,
    test_gated: bool,
}

/// The function name declared by `trimmed`, if it declares one.
///
/// Qualifiers are stripped token by token, and **a comment line is never a
/// declaration** — the brief's S3-R89 sibling clause, met twice in this milestone
/// by counts that read a `//!` header as code. A declaration this misses costs a
/// LOUD failure, never a silent pass: its mints fall to an outer scope or to
/// [`FILE_SCOPE`], neither of which any pinned site carries.
fn declared_fn(trimmed: &str) -> Option<&str> {
    const QUALIFIERS: &[&str] = &[
        "pub",
        "pub(crate)",
        "pub(super)",
        "async",
        "const",
        "unsafe",
        "default",
    ];
    if trimmed.starts_with("//") {
        return None;
    }
    let mut rest = trimmed;
    loop {
        if let Some(after) = rest.strip_prefix("fn ") {
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or_default();
            return (!name.is_empty()).then_some(name);
        }
        let (head, tail) = rest.split_once(' ')?;
        if !QUALIFIERS.contains(&head) {
            return None;
        }
        rest = tail.trim_start();
    }
}

/// Every line of `text`, each carrying the name of the INNERMOST function
/// enclosing it and whether it is `#[cfg(test)]`-gated.
///
/// **Scopes close by indentation:** a scope ends on the line that is exactly its
/// declaration's own indent followed by `}`. Sound here because
/// `cargo fmt --check` is a gate on this workspace, so a function's closing brace
/// sits at the function's own indent. A nested `fn` is therefore attributed to
/// ITSELF, which is what a door means: the innermost site holding the call.
///
/// **Test gating is per ITEM, never by truncation** — the F2 partition defect. A
/// `#[cfg(test)]` line gates the item that follows it, and the gate closes with
/// that item:
///
/// - `mod name {` / `fn name(…) {` — gated to the matching-indent `}`;
/// - `mod tests;` — one line, gated alone, no region opened (measured: this form
///   is the last line of every file using it, so hunting a closing brace would
///   swallow the rest of a file that has no rest);
/// - **anything else — NOT GATED, deliberately.** An unrecognised shape leaves
///   test code readable as production, which reddens LOUDLY as an unpinned site;
///   the opposite mistake makes production code INVISIBLE, and only that
///   direction agrees with the failure this census exists to catch.
///
/// The same reasoning covers a misparse of either kind: it pushes a mint into a
/// site the pinned list does not carry, which fails loud.
fn scoped_lines(text: &str) -> Vec<Scoped<'_>> {
    let mut fns: Vec<(usize, &str)> = Vec::new();
    let mut gate: Option<usize> = None;
    let mut pending = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let body = trimmed.trim_end();
        let indent = line.len() - trimmed.len();

        // A gated region's own closing brace belongs to it, so the flag is read
        // before the region is closed.
        let test_gated = pending || gate.is_some();
        if body == "}" && gate == Some(indent) {
            gate = None;
        }
        if body == "}" && fns.last().is_some_and(|(open_at, _)| *open_at == indent) {
            fns.pop();
        }
        let function = fns.last().map(|(_, name)| *name);
        if let Some(name) = declared_fn(trimmed) {
            fns.push((indent, name));
        }

        if !trimmed.starts_with("//") {
            if body == "#[cfg(test)]" {
                pending = true;
            } else if pending && !body.is_empty() && !body.starts_with("#[") {
                pending = false;
                if body.ends_with('{') {
                    gate = Some(indent);
                }
            }
        }

        out.push(Scoped {
            text: line,
            function,
            test_gated,
        });
    }
    out
}

fn read_pinned(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).expect("a pinned file is readable")
}

/// **Every production candidate MINT, keyed by SITE, counted.**
///
/// Two deliberate differences from the census F2 found: the key is the
/// `(file, function)` SITE rather than the file, and there is **no `break`** — a
/// second mint inside a function this list already knows is a change, and the
/// early exit is what made it invisible.
fn mint_sites() -> BTreeMap<Site, usize> {
    let root = workspace_root();
    let mut sites: BTreeMap<Site, usize> = BTreeMap::new();
    for file in production_sources() {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (function, count) in mints_in(&text) {
            *sites.entry((rel.clone(), function)).or_default() += count;
        }
    }
    sites
}

/// The mints in ONE file's text, keyed by the function holding them.
///
/// Split out so the partition can be measured against a fixture through the
/// SAME code path the census uses — a partition proven only on the tree it was
/// written against proves the tree, not the partition.
fn mints_in(text: &str) -> BTreeMap<String, usize> {
    let mut sites: BTreeMap<String, usize> = BTreeMap::new();
    for line in scoped_lines(text).iter().filter(|l| !l.test_gated) {
        let trimmed = line.text.trim_start();
        // A doc comment is a mention, not a door. Skipping it is what keeps
        // a true door distinguishable from the prose that describes one.
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("candidate_of_body(") || trimmed.contains("candidate_of_batch(") {
            *sites
                .entry(line.function.unwrap_or(FILE_SCOPE).to_string())
                .or_default() += 1;
        }
    }
    sites
}

/// **Every `stored_form_guard_lazy` call in `rel`, keyed by the function making
/// it, counted.**
///
/// The definition line is excluded STRUCTURALLY — it is a declaration, not a call
/// — rather than by filtering on guesswork, and the delegated core
/// (`stored_form_guard`) does not match this name at all.
fn guard_sites(rel: &str) -> BTreeMap<String, usize> {
    let text = read_pinned(rel);
    let mut sites: BTreeMap<String, usize> = BTreeMap::new();
    for line in scoped_lines(&text).iter().filter(|l| !l.test_gated) {
        let trimmed = line.text.trim_start();
        if trimmed.starts_with("//") || declared_fn(trimmed).is_some() {
            continue;
        }
        if trimmed.contains("stored_form_guard_lazy(") {
            *sites
                .entry(line.function.unwrap_or(FILE_SCOPE).to_string())
                .or_default() += 1;
        }
    }
    sites
}

/// How many times `name` is declared as a function in the production half of
/// `rel`.
fn declarations_of(rel: &str, name: &str) -> usize {
    let text = read_pinned(rel);
    scoped_lines(&text)
        .iter()
        .filter(|l| !l.test_gated && declared_fn(l.text.trim_start()) == Some(name))
        .count()
}

/// Whether `door_fn`'s own body in `rel` contains a call to `callee`.
fn body_calls(rel: &str, door_fn: &str, callee: &str) -> bool {
    let text = read_pinned(rel);
    let needle = format!("{callee}(");
    scoped_lines(&text)
        .iter()
        .filter(|l| !l.test_gated && l.function == Some(door_fn))
        .any(|line| {
            let trimmed = line.text.trim_start();
            !trimmed.starts_with("//") && trimmed.contains(&needle)
        })
}

/// **The door SET is exactly the pinned list, keyed by SITE.**
///
/// *Fails on:* a mint in a NEW file · a mint in a NEW function of a file already
/// pinned (the case the file-keyed assertion could not see) · a pinned function
/// renamed or its mint removed · a mint that lands outside every function.
#[test]
fn the_byte_landing_door_set_is_exactly_the_pinned_list() {
    let measured: BTreeSet<Site> = mint_sites().into_keys().collect();
    let pinned: BTreeSet<Site> = DOORS
        .iter()
        .map(|d| (d.file.to_string(), d.mint_fn.to_string()))
        .collect();
    assert_eq!(
        measured, pinned,
        "the set of SITES minting a candidate changed — classify the new door in DOORS \
         (what does U12 do there, and where is its guard discharged?) rather than editing \
         this assertion",
    );
}

/// **Each pinned site mints exactly once, and the total is the pinned total.**
///
/// This is the assertion that replaces `assert_eq!(DOORS.len(), 8)` — a tautology
/// over the hardcoded const, which asserted that a literal list has the length of
/// that literal list. Here the left-hand side is MEASURED FROM THE TREE.
///
/// *Fails on:* a SECOND mint added inside a function the census already pins (the
/// one evasion a site-keyed SET still cannot see, because the key does not move)
/// · two `DOORS` rows pinning one site, which would let a duplicate pin satisfy
/// the set comparison · any change to the total mint population.
#[test]
fn every_pinned_site_mints_exactly_once_and_the_total_matches_the_pin() {
    let measured = mint_sites();
    assert!(
        !measured.is_empty(),
        "the population this gate iterates is non-empty (S3-R37)",
    );
    for ((file, function), count) in &measured {
        assert_eq!(
            *count, 1,
            "{file}::{function} mints {count} candidates — a door is ONE mint at ONE site; \
             pin the new one in DOORS with its own class and guard site",
        );
    }
    let pinned_sites: BTreeSet<Site> = DOORS
        .iter()
        .map(|d| (d.file.to_string(), d.mint_fn.to_string()))
        .collect();
    assert_eq!(
        pinned_sites.len(),
        DOORS.len(),
        "two DOORS rows pin the same site — the site is the key, so the list must not \
         carry it twice",
    );
    let total: usize = measured.values().sum();
    assert_eq!(
        total,
        DOORS.len(),
        "the tree mints at {total} sites and DOORS pins {} — every mint is a door and \
         every door is pinned",
        DOORS.len(),
    );
}

/// **The arithmetic closes** (R32): every door is accounted for exactly once,
/// and each class is NON-EMPTY (S3-R37 — a gate whose population empties is the
/// quietest way for coverage to disappear).
///
/// *Fails on:* a door re-classified in `DOORS` without its class count following
/// · a class emptying · the `write.rs` share drifting from the guarded classes.
/// **These are pin-consistency assertions over the const**, not measurements of
/// the tree: they make an edit to `DOORS` deliberate. The tree-facing arithmetic
/// is [`every_pinned_site_mints_exactly_once_and_the_total_matches_the_pin`].
#[test]
fn the_arithmetic_closes_and_no_class_is_empty() {
    let translated = DOORS
        .iter()
        .filter(|d| d.class == Door::TranslatedAndGuarded)
        .count();
    let guarded = DOORS.iter().filter(|d| d.class == Door::Guarded).count();
    let outside = DOORS
        .iter()
        .filter(|d| d.class == Door::OutsideThisUnit)
        .count();
    let read_only = DOORS.iter().filter(|d| d.class == Door::ReadOnly).count();

    assert_eq!(translated, 2, "splice and create carry user-supplied bytes");
    assert_eq!(
        guarded, 3,
        "lock_write, the promotion and commit_batch land engine-composed bytes",
    );
    assert_eq!(
        outside, 5,
        "two realise doors, the run plane, and G2's two genesis mints — stated, not absorbed",
    );
    assert_eq!(
        read_only, 1,
        "the reaction feeder's path-carrying read — it mints, and it lands nothing",
    );
    assert_eq!(
        translated + guarded + outside + read_only,
        DOORS.len(),
        "every door falls in exactly one class",
    );

    // The guard covers every door in U12's own file, and nothing else claims to.
    let in_write_rs = DOORS.iter().filter(|d| d.file == WRITE_RS).count();
    assert_eq!(
        in_write_rs,
        translated + guarded,
        "U12 guards exactly the doors in its named file — the arithmetic that \
         says what this unit did and did not close",
    );
}

/// **Every door in U12's own file BINDS its own guard.**
///
/// The census F2 found counted `stored_form_guard_lazy(` occurrences against a
/// count of `DOORS` rows: a bag against a bag, with nothing tying a call to a
/// door. **Here the comparison is per discharge SITE**, so moving a guard from
/// one door to another changes both sides in opposite directions instead of
/// cancelling out.
///
/// *Fails on:* the guard deleted at any door (its site's count falls to zero and
/// the key vanishes) · a guard duplicated anywhere (that site counts 2) · a guard
/// appearing in a function no door pins (an unexpected key) · a door in `write.rs`
/// pinned with no discharge site at all.
#[test]
fn every_door_in_this_units_file_binds_its_own_guard() {
    let mut expected: BTreeMap<String, usize> = BTreeMap::new();
    for door in DOORS.iter().filter(|d| d.file == WRITE_RS) {
        let site = door.guard_fn.unwrap_or_else(|| {
            panic!(
                "the door '{}' is in U12's own file and names no guard discharge site — \
                 a door with no guard is the thing this census exists to refuse",
                door.label,
            )
        });
        *expected.entry(site.to_string()).or_default() += 1;
    }
    assert!(
        !expected.is_empty(),
        "the population this gate iterates is non-empty (S3-R37)",
    );
    let measured = guard_sites(WRITE_RS);
    assert_eq!(
        measured, expected,
        "the artifact guard is no longer discharged once per door — each door in write.rs \
         discharges `stored_form_guard_lazy` exactly once, at the site DOORS names for it. \
         A guard that moved between doors leaves the TOTAL unchanged and shows up here",
    );
}

/// **Every pinned name exists in the tree, exactly once.**
///
/// The brief's S3-R89 sibling clause, applied to this census: a name matched
/// across files written by different hands is a statement about the spelling
/// until the spelling is confirmed present. A pin naming a function that does not
/// exist would otherwise report *zero guards at that site* — an alarming answer
/// produced by the pin, not by the code. A name declared TWICE would merge two
/// functions into one site and could hide a door inside another door's body.
///
/// *Fails on:* a pinned `door_fn` / `mint_fn` / `guard_fn` that no longer exists
/// (a rename) · a pinned name declared more than once in its file · a pinned file
/// that has moved.
#[test]
fn every_pinned_name_is_declared_exactly_once_in_its_file() {
    for door in DOORS {
        for (role, name) in [
            ("door_fn", Some(door.door_fn)),
            ("mint_fn", Some(door.mint_fn)),
            ("guard_fn", door.guard_fn),
        ] {
            let Some(name) = name else { continue };
            let found = declarations_of(door.file, name);
            assert_eq!(
                found, 1,
                "{}: {role} '{name}' is declared {found} times in {} — a pinned name must \
                 resolve to exactly one function, or this census measures the wrong body",
                door.label, door.file,
            );
        }
    }
}

/// **A door whose guard is discharged ELSEWHERE still reaches it from its own
/// body.**
///
/// `splice` mints in `build_after_doc` and discharges in
/// `translate_stored_candidate` — so pinning the discharge site alone leaves a
/// hole: delete `splice`'s call to `translate_stored_candidate` and the guard
/// still exists, in a function nothing reaches. This arm closes that by asserting
/// the door's own body reaches both its mint and its guard.
///
/// *Fails on:* a door losing the call to its mint helper · a door losing the call
/// to the helper that discharges its guard · the indirect population emptying,
/// which would leave this arm iterating nothing (S3-R37).
#[test]
fn a_door_reaches_its_own_mint_and_its_own_guard() {
    let mut indirect = 0;
    for door in DOORS.iter().filter(|d| d.file == WRITE_RS) {
        if door.mint_fn != door.door_fn {
            indirect += 1;
            assert!(
                body_calls(door.file, door.door_fn, door.mint_fn),
                "{}: the door '{}' no longer calls its mint helper '{}' — the site this \
                 census pins is not the one the door reaches",
                door.label,
                door.door_fn,
                door.mint_fn,
            );
        }
        let Some(guard_fn) = door.guard_fn else {
            continue;
        };
        if guard_fn != door.door_fn {
            indirect += 1;
            assert!(
                body_calls(door.file, door.door_fn, guard_fn),
                "{}: the door '{}' no longer calls '{guard_fn}', the function discharging \
                 its artifact guard — the guard exists and the door does not reach it",
                door.label,
                door.door_fn,
            );
        }
    }
    assert!(
        indirect > 0,
        "no door in write.rs discharges its guard or mints outside its own body, so this \
         arm iterates an empty population (S3-R37). If every door was inlined, strike this \
         arm deliberately and say so — do not leave it green over nothing",
    );
}

/// **The partition gates ITEMS, not the rest of the file.**
///
/// The F2 partition defect had no failing signal on this tree — `pack.rs` mints
/// nothing, so the census stayed green with 82% of that file unread. **A fixture
/// is the only way to measure a partition rather than the tree it was written
/// against**, and it cannot rot: the shapes are literals here, not another
/// crate's structure, so refactoring `pack.rs` can never redden this.
///
/// *Fails on:* a partition that truncates at the first marker instead of gating
/// the item (the `pack.rs` shape — `after_an_item_level_gate` disappears) · a
/// gated `mod` leaking its contents · the semicolon form swallowing the lines
/// after it · an unrecognised shape being gated instead of left visible.
#[test]
fn the_partition_gates_items_not_the_rest_of_the_file() {
    // Every gated shape measured in this workspace, plus the ambiguous one.
    let fixture = "\
fn before_any_gate() {
    model::candidate_of_body(path, body);
}

#[cfg(test)]
#[allow(dead_code)]
fn a_gated_helper() {
    model::candidate_of_body(path, body);
}

fn after_an_item_level_gate() {
    model::candidate_of_batch(path, raw, sealed);
}

#[cfg(test)]
mod tests {
    fn inside_a_gated_module() {
        model::candidate_of_body(path, body);
    }
}

fn after_a_gated_module() {
    model::candidate_of_body(path, body);
}

#[cfg(test)]
mod tests;

fn after_the_semicolon_form() {
    model::candidate_of_body(path, body);
}

#[cfg(test)]
static AN_UNRECOGNISED_SHAPE: usize = 0;

fn after_an_unrecognised_shape() {
    model::candidate_of_body(path, body);
}
";
    let measured = mints_in(fixture);
    let expected: BTreeMap<String, usize> = [
        ("before_any_gate", 1),
        ("after_an_item_level_gate", 1),
        ("after_a_gated_module", 1),
        ("after_the_semicolon_form", 1),
        ("after_an_unrecognised_shape", 1),
    ]
    .into_iter()
    .map(|(name, n)| (name.to_string(), n))
    .collect();
    assert_eq!(
        measured, expected,
        "the production/test partition changed shape. A gated ITEM hides itself and \
         nothing after it; truncating at the first marker would drop every site below \
         it — which is how 82% of crates/policy/src/pack.rs went unread while this \
         census reported green",
    );
}

/// **Each class claims exactly the guard its position entitles it to.**
///
/// *Fails on:* an `OutsideThisUnit` door quietly given a discharge site (which
/// would make the gap read as closed) · a door in U12's own file classed as
/// outside it · a guarded door outside U12's named file · a `ReadOnly` site
/// claiming a guard, which would assert a byte landing it does not perform.
///
/// Matched exhaustively on the CLASS rather than tested with an `if`/`else`: the
/// arms then cannot silently absorb a class added later — which is exactly how
/// C3's `ReadOnly` site first arrived here, classified by an `else` written when
/// every mint was a door.
#[test]
fn each_class_claims_exactly_the_guard_its_position_entitles_it_to() {
    for door in DOORS {
        match door.class {
            Door::TranslatedAndGuarded | Door::Guarded => assert_eq!(
                door.file, WRITE_RS,
                "{}: a guarded door lives in U12's named file",
                door.label,
            ),
            Door::OutsideThisUnit => {
                assert_ne!(
                    door.file, WRITE_RS,
                    "{}: a door in U12's own file cannot be classed OutsideThisUnit",
                    door.label,
                );
                assert!(
                    door.guard_fn.is_none(),
                    "{}: a door outside this unit names a guard discharge site — U12 closes no \
                     door there, and a pin saying otherwise reads as a closed gap",
                    door.label,
                );
            }
            Door::ReadOnly => assert!(
                door.guard_fn.is_none(),
                "{}: a site that lands no bytes names a stored-form guard — the guard exists \
                 to translate addresses a WRITE introduces, so claiming one here asserts a \
                 byte landing that does not happen",
                door.label,
            ),
        }
    }
}
