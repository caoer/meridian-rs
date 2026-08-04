//! **The FIELD dry run** — `lockmigrate::sweep` pointed at ZT's live vaults,
//! read-only, and the report printed for a human.
//!
//! `#[ignore]` by design: it reads paths outside the workspace, so it is not a
//! CI gate. Run it deliberately:
//!
//! ```text
//! cargo test -p lockmigrate --test field_dryrun -- --ignored --nocapture
//! ```
//!
//! **It cannot write.** `Options::dry` is `true` and `dry_run_writes_nothing`
//! in `gates.rs` asserts on BYTES that the dry path leaves a vault untouched;
//! this file additionally re-reads every candidate page afterwards and fails if
//! a single byte moved. The real sweep is not run from here, ever — it runs
//! through `mrd lock-migrate` inside the Leader's quiesce window, after a
//! pre-sweep commit in each vault.
//!
//! The vault list lives in `VAULTS` rather than being discovered, because a
//! sweep that finds its own targets is a sweep nobody reviewed.

use lockmigrate::{Options, PageVerdict, sweep};

/// ZT's live vaults, both git repositories (the restore-point precondition).
const VAULTS: &[&str] = &[
    "/Users/Shared/projects/field-notes",
    "/Users/Shared/projects/field-notes-sessions",
];

#[test]
#[ignore = "reads ZT's live vaults; run deliberately with --ignored"]
fn field_dry_run() {
    let mut any = false;
    for vault in VAULTS {
        let path = std::path::Path::new(vault);
        if !path.exists() {
            println!("SKIP {vault} — not present on this host");
            continue;
        }
        any = true;
        let root = fs::WorkspaceRoot(path.to_path_buf());

        // Snapshot every markdown byte we could possibly touch, so "wrote
        // nothing" is measured rather than trusted.
        let before = snapshot(path);

        let report = sweep(
            &root,
            &Options {
                dry: true,
                ..Options::default()
            },
        )
        .expect("the dry sweep runs to a verdict");

        println!("\n{}", report.render());
        println!("--- per-page verdicts, with the rule that decided each ---");
        for page in &report.pages {
            let rule = match page {
                PageVerdict::Migrated { .. } => "MIGRATE: terminal, single, parses as v1",
                PageVerdict::AlreadyV2 { .. } => "SKIP: already v2 (idempotence)",
                PageVerdict::NotEnginePlaced { .. } => {
                    "EXCLUDED by rule 1 (placement): content follows the LAST block, \
                     so the engine did not place it"
                }
                PageVerdict::MultipleBlocks { .. } => {
                    "REFUSED by rule 2 (arity): terminal lock, but more than one block"
                }
                PageVerdict::Unparseable { .. } => "REFUSED by rule 3 (parse)",
                PageVerdict::Unconvertible { .. } => "REFUSED: damaged row",
            };
            println!("  {:<70} {rule}", page.path());
        }

        let after = snapshot(path);
        // Report CHANGED PATHS, never contents — an `assert_eq!` over two maps
        // of file bytes prints the whole vault on failure (measured: 1.2 GB),
        // which buries the one fact you needed.
        let changed: Vec<&String> = before
            .keys()
            .chain(after.keys())
            .filter(|k| before.get(*k) != after.get(*k))
            .collect();
        assert!(
            changed.is_empty(),
            "THE DRY RUN WROTE TO {vault} — this must never happen. Changed: {changed:?}"
        );
        println!(
            "\nBYTES UNMOVED in {vault} ({} lock-bearing page(s) watched)",
            before.len()
        );
    }
    assert!(any, "no vault was present — the dry run proved nothing");
}

/// `(relative path, content)` for every page that CARRIES A LOCK BLOCK — which
/// is exactly the sweep's possible write set, and nothing else.
///
/// # Why not every markdown file
/// It was, first, and it produced a FALSE POSITIVE that is worth keeping in the
/// record: `field-notes-sessions` is a LIVE session tree, and the agent fleet
/// wrote to it during the ~60 s sweep. The whole-vault snapshot caught those
/// writes and blamed them on a dry run that had migrated nothing at all.
///
/// **That false positive is evidence for P13 step 2.** The runbook quiesces the
/// fleet before the real sweep, and this is the measurement showing why: the
/// vault moves under you while you work if nobody stops it. Narrowing the watch
/// to lock-bearing pages makes the assertion answer the question it was asked
/// — did THE SWEEP write — instead of "did anything on this machine write".
fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut walk = vec![root.to_path_buf()];
    while let Some(dir) = walk.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                walk.push(path);
            } else if path.extension().is_some_and(|e| e == "md")
                && let Ok(bytes) = std::fs::read(&path)
                && let Ok(text) = std::str::from_utf8(&bytes)
                && text.contains("```meridian-lock")
            {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                out.insert(rel.display().to_string(), bytes.clone());
            }
        }
    }
    out
}
