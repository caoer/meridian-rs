//! U12's door enumeration, keyed by SITE and bound PER DOOR.
//!
//! U31 made the candidate document a type the `fs` byte-landing primitives
//! demand, so "which doors land bytes" is measured as "which sites mint a
//! `model::CandidateDocument`". This test pins that set and states, per door,
//! what U12 does at it. The equivalence is one-directional: every byte landing
//! mints a candidate, but a mint is not proof of a landing — see
//! [`Door::ReadOnly`]. The compiler enumerates the type's uses; what it cannot
//! say is whether a door that HOLDS a candidate also GUARDS it — a guard is a
//! call, not a type.
//!
//! Census rules:
//!
//! - the census key is the SITE — `(file, mint_fn)`, never the file;
//! - each site's mint count is pinned, so a second mint inside a function the
//!   census already knows is a change too;
//! - the guard is bound to the door that owes it — per-site, never a total;
//! - keyed by `file::function`, never by line (a line number rots on the next
//!   edit, and a rotted pin teaches a reader to ignore the check);
//! - test gating is per ITEM, never by file truncation, and every
//!   unrecognised shape resolves toward NOT gating: test code read as
//!   production reddens loudly as an unpinned site, while production code
//!   read as test goes invisible — only the first direction fails safe;
//! - the scan reads production `src/` only, skips doc comments and the
//!   definition site, and reports a new mint as unclassified, never guessed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A door's identity: the workspace-relative file, and the function whose body
/// holds the `candidate_of_*` call. This pair is the census key.
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
    /// NOT covered by this unit, stated rather than absorbed: the door lives
    /// outside U12's named files (`write.rs` + `read.rs`).
    OutsideThisUnit,
    /// Not a door: the mint lands no bytes. The site mints a candidate to give
    /// a document its own PATH (the reaction feeder matches a HOOK's `paths:`
    /// scope against it), reads it, and drops it. A site entering this class
    /// owes the reason its value cannot land — for the one member,
    /// `feed_landed_change` takes `&Document` and returns effects, so there is
    /// no path from the mint to a write.
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
    /// `translate_stored_candidate`; the other three discharge in themselves.
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
    // Retired doors are deleted, not tombstoned: every pin is verified to
    // exist at its file and function, so a pin naming a deleted function is
    // the enumeration failing open. Successor doors owe their own pins.
    DoorPin {
        file: "crates/run/src/fp.rs",
        door_fn: "candidate",
        mint_fn: "candidate",
        guard_fn: None,
        label: "the run plane's candidate (run::fp::candidate)",
        class: Door::OutsideThisUnit,
    },
    // ---- wire-serve/watch.rs — the reaction feeder, a mint that is NOT a
    // door. `external_effects` needs each externally-changed document to carry
    // its own path (a HOOK matches `paths:` against it); the candidate is read
    // by `feed_landed_change` and dropped — no `fs` primitive ever sees it.
    DoorPin {
        file: "crates/wire-serve/src/watch.rs",
        door_fn: "external_effects",
        mint_fn: "doc_at",
        guard_fn: None,
        label: "the reaction feeder's path-carrying read (lands no bytes)",
        class: Door::ReadOnly,
    },
    // ---- wire-serve/write.rs — the § A.7 overlay builder, a mint that is NOT
    // a door. `overlay_candidate` applies a script's OWN armed edits to the
    // entry document IN MEMORY so the in-process read serve can answer
    // read-your-own-writes; the candidate is returned as a served `Document`
    // and dropped with the attempt — no `fs` primitive ever sees it, and the
    // real commit re-validates and re-mints through `splice`'s own
    // translated-and-guarded door. The reason its value cannot land: it
    // leaves as `into_document()` to a read face; nothing on that path takes
    // a `CandidateDocument` or calls a byte-landing primitive.
    DoorPin {
        file: WRITE_RS,
        door_fn: "overlay_candidate",
        mint_fn: "overlay_candidate",
        guard_fn: None,
        label: "the § A.7 overlay serve (read-your-own-writes; lands no bytes)",
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
/// Qualifiers are stripped token by token; a comment line is never a
/// declaration. A declaration this misses costs a LOUD failure, never a
/// silent pass: its mints fall to an outer scope or to [`FILE_SCOPE`],
/// neither of which any pinned site carries.
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
/// Scopes close by indentation — sound because `cargo fmt --check` gates this
/// workspace, so a closing brace sits at its declaration's indent.
///
/// Test gating is per ITEM, never by truncation. A `#[cfg(test)]` line gates
/// the item that follows it: `mod name {` / `fn name(…) {` to the
/// matching-indent `}`; `mod tests;` gated alone, no region opened; anything
/// else NOT gated, deliberately (see the module header on error direction).
/// A misparse of either kind pushes a mint into a site the pinned list does
/// not carry, which fails loud.
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

/// Every production candidate MINT, keyed by SITE, counted. The key is the
/// `(file, function)` SITE rather than the file, and there is no `break` — a
/// second mint inside a known function is a change too.
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

/// Each pinned site mints exactly once, and the total is the pinned total —
/// the left-hand side is MEASURED FROM THE TREE, not the const.
///
/// *Fails on:* a SECOND mint added inside a function the census already pins
/// (the one evasion a site-keyed SET cannot see, because the key does not
/// move) · two `DOORS` rows pinning one site · any change to the total mint
/// population.
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

/// The arithmetic closes: every door is accounted for exactly once, and each
/// class is NON-EMPTY (an emptied population is the quietest way for coverage
/// to disappear).
///
/// *Fails on:* a door re-classified in `DOORS` without its class count
/// following · a class emptying · the `write.rs` share drifting from the
/// guarded classes. Pin-consistency over the const; the tree-facing
/// arithmetic is
/// [`every_pinned_site_mints_exactly_once_and_the_total_matches_the_pin`].
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
        "lock_write, the promotion and commit_batch land engine-composed bytes. \
         WAS 4: U9b's v1→v2 lock migration door lived here too, until DECISION 26 \
         (ZT 2026-08-04) deleted it with its crate — the two field locks were \
         hand-migrated, so the door had nothing left to migrate. It was a SECOND \
         door only because it had to locate a block the live reader refuses to \
         parse (P4); with it gone `lock_write` is again the only lock door",
    );
    assert_eq!(
        outside, 1,
        "the run plane's candidate — stated, not absorbed. WAS 3: G2's two genesis \
         mints left this count with the verb and the ledger it reset (journal \
         retirement, U6), the same way the two `realise --truth` doors left it with \
         the flag itself (registration cutover). The redesigned convergence owes its \
         own pins when it lands; the journal owes none, because nothing replaces it",
    );
    assert_eq!(
        read_only, 2,
        "the two lands-nothing mints: the reaction feeder's path-carrying read, \
         and the § A.7 overlay serve (read-your-own-writes) — each mints, and \
         each lands nothing",
    );
    assert_eq!(
        translated + guarded + outside + read_only,
        DOORS.len(),
        "every door falls in exactly one class",
    );

    // The guard covers every DOOR in U12's own file, and nothing else claims
    // to. The § A.7 overlay mint shares the file without being a door (it
    // lands no bytes), so the file census counts it beside the guarded set
    // rather than inside it.
    let in_write_rs = DOORS.iter().filter(|d| d.file == WRITE_RS).count();
    let read_only_in_write_rs = DOORS
        .iter()
        .filter(|d| d.file == WRITE_RS && d.class == Door::ReadOnly)
        .count();
    assert_eq!(
        in_write_rs,
        translated + guarded + read_only_in_write_rs,
        "U12 guards exactly the doors in its named file — the arithmetic that \
         says what this unit did and did not close; the overlay mint is in the \
         file and is not a door",
    );
}

/// Every door in U12's own file BINDS its own guard. The comparison is per
/// discharge SITE, so moving a guard between doors changes both sides in
/// opposite directions instead of cancelling out.
///
/// *Fails on:* the guard deleted at any door · a guard duplicated anywhere ·
/// a guard appearing in a function no door pins · a door in `write.rs` pinned
/// with no discharge site at all.
#[test]
fn every_door_in_this_units_file_binds_its_own_guard() {
    let mut expected: BTreeMap<String, usize> = BTreeMap::new();
    // `ReadOnly` is exempt BY ITS DEFINITION, not by grace: the class means
    // "not a door — the mint lands no bytes", and what such a site owes is
    // the reason its value cannot land (stated at its pin), never a guard
    // over bytes that do not exist. Every DOOR class in this file still
    // refuses without a discharge site.
    for door in DOORS
        .iter()
        .filter(|d| d.file == WRITE_RS && d.class != Door::ReadOnly)
    {
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

/// Every pinned name exists in the tree, exactly once. A pin naming a missing
/// function would report *zero guards at that site* — an alarm produced by
/// the pin, not the code; a name declared TWICE would merge two functions
/// into one site and could hide a door inside another door's body.
///
/// *Fails on:* a pinned `door_fn` / `mint_fn` / `guard_fn` that no longer
/// exists (a rename) · a pinned name declared more than once in its file · a
/// pinned file that has moved.
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

/// The partition gates ITEMS, not the rest of the file. A fixture is the only
/// way to measure a partition rather than the tree it was written against,
/// and it cannot rot: the shapes are literals here.
///
/// *Fails on:* a partition that truncates at the first marker instead of
/// gating the item (`after_an_item_level_gate` disappears) · a gated `mod`
/// leaking its contents · the semicolon form swallowing the lines after it ·
/// an unrecognised shape being gated instead of left visible.
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

/// Each class claims exactly the guard its position entitles it to.
///
/// *Fails on:* an `OutsideThisUnit` door quietly given a discharge site · a
/// door in U12's own file classed as outside it · a guarded door outside
/// U12's named file · a `ReadOnly` site claiming a guard, which would assert
/// a byte landing it does not perform.
///
/// Matched exhaustively on the CLASS rather than `if`/`else`, so the arms
/// cannot silently absorb a class added later.
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
