//! **The three ingress classes the compiler CANNOT reach** — U10 part (b).
//! The address is a type (`crates/addr`), and retyping `lock::PinEntry` propagated it
//! across the `lock` / `view` / `wire-serve` cluster **by compiler**: every site where
//! the data carrying an address changed type had to discharge the parse or fail to build.
//! That is the ladder's top rung and it is exhaustive over TYPE FLOW.
//! It stops at three classes, because in each the address arrives as a raw string into a
//! slot whose type does not change:
//! | Class | Why the compiler cannot reach it | |---|---| | `markdown-body` | `dest:
//! &str` from pulldown-cmark, and `syntax` is UPSTREAM of `model` — a type in a crate
//! depending on `model` is architecturally unreachable from it | | `cli-argv` |
//! hand-rolled splits over `String` args into `String` fields — nothing changes type | |
//! `wire-decode` | `crates/wire` deps are serde only and criterion 7 freezes v2
//! byte-identity; a `root:`-bearing target decodes into a plain `String`, and the
//! compiler cannot force a call that does not exist |
//! So this is the ladder's MIDDLE rung, used exactly where plan § 3.1 says *a list is all
//! a test can give you*: it proves the door SET, not that each door is closed. **It fails
//! when a FOURTH ingress appears** — a new raw split of an address spelling anywhere in
//! the workspace source is an unclassified site until someone writes down which class it
//! belongs to.
//! **Precision, measured before the check was written** (S3-R23 ①): the scan finds every
//! `#`-split in workspace `src/`, address-bearing or not, and the table classifies each.
//! It therefore never *guesses* that a site is a defect — it reports that the set changed
//! and demands a classification. Two of today's eleven rows are `not-an-address` and are
//! pinned as such, which is what keeps a true positive distinguishable from a frontmatter
//! parse.
//!
//!
//!
//!

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What a raw `#`-split site is, once a human has looked at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// **INGRESS.** A wikilink `dest` from the markdown body. `syntax` is
    /// upstream of `model`, so no type living downstream can reach it.
    MarkdownBody,
    /// **INGRESS.** A CLI positional or flag value: `String` argv into a
    /// `String` field. Nothing changes type, so nothing recompiles.
    CliArgv,
    /// **INGRESS.** A client-supplied spelling arriving over the wire, where
    /// `crates/wire`'s serde-only dependency set and v2 byte-identity
    /// (criterion 7) keep the slot a plain `String`.
    WireDecode,
    /// Not an ingress: a site that reads a spelling the engine already holds.
    /// The address type reaches these through normal call flow; they are listed
    /// so the arithmetic closes, not because they are doors.
    InternalReader,
    /// Not an address at all. Pinned so a true ingress stays distinguishable
    /// from an ordinary `#` split — the false positive that would get this
    /// instrument deleted.
    NotAnAddress,
}

/// **THE DOOR LIST.** Every raw `#`-split in workspace `src/`, with its class.
///
/// Lines are deliberately absent: a line number rots on the next edit and the
/// rot reads exactly like a real change. The `needle` is a stable fragment of
/// the site's own enclosing item, so a site that MOVES stays matched and a site
/// that DISAPPEARS is a real finding.
const PINNED: &[(&str, &str, Class)] = &[
    // ---- INGRESS 1: the markdown body (`syntax`, upstream of `model`) -------
    (
        "crates/syntax/src/lib.rs",
        "fn split_wikilink_target",
        Class::MarkdownBody,
    ),
    (
        "crates/syntax/src/lib.rs",
        "pub fn block_slot",
        Class::MarkdownBody,
    ),
    // ---- INGRESS 2: CLI argv (`String` in, `String` field out) -------------
    ("crates/mrd/src/pin_cmd.rs", "fn parse", Class::CliArgv),
    ("crates/mrd/src/put_cmd.rs", "fn parse", Class::CliArgv),
    ("crates/mrd/src/read_cmd.rs", "fn parse", Class::CliArgv),
    // FINDING 02's A4, and it is CLI argv despite living in `view`: the walk
    // root reaches `page_of` as the bare positional `walk_cmd.rs` collected
    // (`walk_cmd.rs` → `page: String` → `view::walk::walk(.., &parsed.page, ..)`).
    // Classified `internal-reader` in this table's first draft — corrected after
    // tracing the argument rather than trusting the crate it sits in.
    ("crates/view/src/walk.rs", "fn page_of", Class::CliArgv),
    // ---- INGRESS 3: wire decode (serde-only, v2 byte-identity frozen) ------
    (
        "crates/model/src/walk.rs",
        "fn parse_linktext",
        Class::WireDecode,
    ),
    // ---- Not doors: readers of a spelling the engine already holds ---------
    (
        "crates/model/src/selector.rs",
        "pub fn parse",
        Class::InternalReader,
    ),
    // ---- Not addresses: pinned so a real ingress stays distinguishable -----
    ("crates/model/src/walk.rs", "fn stage2", Class::NotAnAddress),
];

/// The `#`-split forms a raw address spelling can be taken apart with.
const NEEDLES: &[&str] = &[
    "split_once('#')",
    "rsplit_once('#')",
    "splitn(2, '#')",
    ".split('#')",
    ".find('#')",
];

fn workspace_root() -> PathBuf {
    // `crates/addr` → `crates` → the workspace.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/addr sits two levels below the workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `(crate-relative file, enclosing item)` that splits on `#`, scanned
/// from the tree rather than remembered.
///
/// The enclosing item is the nearest preceding `fn`/`const`/`struct` signature
/// line — the stable anchor a line number cannot be.
fn scan() -> BTreeSet<(String, String)> {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut found = BTreeSet::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&crates)
        .expect("crates/ is readable")
        .flatten()
    {
        let path = entry.path();
        // `crates/addr` OWNS the split — it is the one place the grammar is
        // implemented, so it is not a door.
        if path.is_dir() && path.file_name().is_some_and(|n| n != "addr") {
            dirs.push(path.join("src"));
        }
    }
    let mut files = Vec::new();
    for dir in &dirs {
        rust_sources(dir, &mut files);
    }
    for file in files {
        let text = fs::read_to_string(&file).expect("a source file is readable");
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let mut item = String::from("<file scope>");
        let mut in_test_mod = false;
        for line in text.lines() {
            let trimmed = line.trim();
            // A test module's fixtures are not engine doors.
            if trimmed == "mod tests {" {
                in_test_mod = true;
            }
            if let Some(sig) = enclosing_item(trimmed) {
                item = sig;
            }
            if trimmed.starts_with("//") || in_test_mod {
                continue; // prose about a split is not a split
            }
            if NEEDLES.iter().any(|n| line.contains(n)) {
                found.insert((rel.clone(), item.clone()));
            }
        }
    }
    found
}

/// The signature line's stable fragment, or `None` when this line opens no item.
fn enclosing_item(trimmed: &str) -> Option<String> {
    for kw in ["fn ", "pub fn ", "const ", "pub const ", "struct "] {
        if trimmed.starts_with(kw) {
            let head = trimmed.split(['(', '<', ':', ' ']).collect::<Vec<_>>();
            let name = head.get(if kw.starts_with("pub") { 2 } else { 1 })?;
            let word = kw.trim_end().rsplit(' ').next().unwrap_or(kw);
            return Some(format!(
                "{}{word} {name}",
                if kw.starts_with("pub") { "pub " } else { "" }
            ));
        }
    }
    None
}

/// **THE GATE.** The scanned set equals the pinned set — so a fourth ingress
/// cannot appear without someone writing down which class it belongs to.
#[test]
fn the_ingress_set_is_exactly_the_pinned_door_list() {
    let scanned = scan();
    let pinned: BTreeSet<(String, String)> = PINNED
        .iter()
        .map(|(file, item, _)| ((*file).to_string(), (*item).to_string()))
        .collect();

    let appeared: Vec<_> = scanned.difference(&pinned).collect();
    let vanished: Vec<_> = pinned.difference(&scanned).collect();

    assert!(
        appeared.is_empty(),
        "A FOURTH INGRESS APPEARED — {} unclassified site(s) split an address \
         spelling by hand:\n{:#?}\n\nEach one is a door the address TYPE does \
         not close. Classify it in PINNED (markdown-body / cli-argv / \
         wire-decode / internal-reader / not-an-address), or route it through \
         `addr::Addr::parse`. See docs/address-grammar.md § 4.",
        appeared.len(),
        appeared,
    );
    assert!(
        vanished.is_empty(),
        "A PINNED DOOR VANISHED — {} site(s) no longer split by hand:\n{:#?}\n\n\
         That is progress, not a bug: drop the row from PINNED so the list keeps \
         naming what is actually there.",
        vanished.len(),
        vanished,
    );
}

/// The arithmetic closes (R32): every scanned site is accounted for by exactly
/// one class, and the three INGRESS classes are each non-empty.
///
/// A door list whose classes were all empty would pass the set comparison above
/// and prove nothing — this is the acceptance half (S3-R8(c)).
#[test]
fn every_ingress_class_is_populated_and_the_arithmetic_closes() {
    let scanned = scan();
    assert_eq!(
        scanned.len(),
        PINNED.len(),
        "doors found ({}) must equal doors named ({})",
        scanned.len(),
        PINNED.len(),
    );

    let count = |class: Class| PINNED.iter().filter(|(_, _, c)| *c == class).count();
    assert_eq!(
        count(Class::MarkdownBody)
            + count(Class::CliArgv)
            + count(Class::WireDecode)
            + count(Class::InternalReader)
            + count(Class::NotAnAddress),
        PINNED.len(),
        "every pinned row carries exactly one class",
    );
    for (class, why) in [
        (
            Class::MarkdownBody,
            "`[[sessions:notes.md#Design]]` in a body IS a cross-root address",
        ),
        (
            Class::CliArgv,
            "`mrd pin 'sessions:notes.md#Design'` is a cross-root address",
        ),
        (
            Class::WireDecode,
            "a `root:`-bearing target decodes into a plain String",
        ),
    ] {
        assert!(
            count(class) > 0,
            "{class:?} must name at least one site — {why}. An empty class means \
             the enumeration stopped describing the engine.",
        );
    }
}

/// Each pinned door still EXISTS. A list that names vanished sites rots into
/// confident fiction; this is the check that it still describes the tree.
#[test]
fn every_pinned_door_still_exists_in_the_tree() {
    let root = workspace_root();
    for (file, item, class) in PINNED {
        let path = root.join(file);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{file} ({class:?}) is unreadable: {e}"));
        let name = item.strip_prefix("pub ").unwrap_or(item);
        assert!(
            text.contains(name),
            "{file} no longer carries `{item}` ({class:?}) — the door list names \
             something that is not there",
        );
    }
}
