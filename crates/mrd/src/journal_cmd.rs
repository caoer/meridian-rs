//! `mrd journal genesis` (G2) — the governed reset of the receipt journal.
//!
//! ```text
//! mrd journal genesis --ruling <REF> [--archive <PATH>] [--dry] [--json]
//! ```
//!
//! # Why a verb exists for something a text editor can already do
//! The write door refuses the reserved journal outright
//! (`wire-serve::write` — a splice targeting it is "a forge attempt"), which is
//! correct and leaves no governed path for the reset the record legitimately
//! needs. The documented procedure was therefore a HAND REWRITE done in the
//! open, with git as the only witness — and a documented hand rewrite teaches
//! every reader that the attested record is editable when it is inconvenient.
//! This verb adds no capability; it moves an existing act inside the governed
//! surface, where it leaves a row.
//!
//! # What the genesis row can honestly carry (the root question)
//! The journal is root-EXCLUDED (`fs::domain`, so guarded writes cannot
//! self-invalidate the root), so TRUNCATING it moves no root and a row claiming
//! otherwise would be fiction. But the exclusion is that ONE path: the archive
//! page this verb writes is ordinary attested content, and creating it advances
//! the root like any governed write. So the row attaches to the ARCHIVE —
//! `path` is the archive, `root_before`/`root_after` bracket its creation, and
//! every token is a fact.
//!
//! # The pointer is the point
//! `path` on the genesis row is machine-addressable: it is how anything
//! downstream finds the rows this reset moved. Recorded at mint time, when it
//! is cheap — reconstructing which archive holds which rows afterwards is the
//! blank-claim lesson that cost this lane three agents.
//!
//! A SECOND genesis archives the rows including the FIRST genesis row, so each
//! archive but the oldest names its predecessor and the chain of archives is
//! linked by the same mechanism that links the chain of rows. The oldest
//! archive holds no genesis row, and that absence IS the terminator — no
//! sentinel, no config, no is-this-the-first flag.
//!
//! # What this verb does NOT do
//! It does not improve chain durability: hand edits still grey the chain the
//! moment anyone opens an editor. A reset is the RECOVERY from that condition,
//! never a defence against it (that is S4's dated gate). And it never invents
//! the justification — `--ruling` is required, because a reset nobody will
//! defend in writing is a reset that should not happen.

use std::path::Path;

use crate::{Fail, Format};

/// The archive's default location: `meridian/journal-archive-<date>.md`, the
/// name the 2026-07-28 hand execution already established.
const ARCHIVE_DIR: &str = "meridian";

/// Parsed `mrd journal genesis` invocation.
#[derive(Debug)]
struct GenesisArgs {
    ruling: String,
    archive: Option<String>,
    dry: bool,
    json: bool,
}

/// The `mrd journal` verb family. Only `genesis` exists; an unknown subcommand
/// names what is available rather than guessing.
///
/// # Errors
/// [`Fail`] — exit 2 on a bad invocation, exit 1 when the plane refuses.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    match args.first().map(String::as_str) {
        Some("genesis") => genesis(&parse(&args[1..])?),
        Some(other) => Err(Fail::tool(format!(
            "unknown journal subcommand '{other}' — the only one is: genesis"
        ))),
        None => Err(Fail::tool(
            "usage: mrd journal genesis --ruling <REF> [--archive <PATH>] [--dry] [--json]"
                .to_owned(),
        )),
    }
}

/// Parse the flags. `--ruling` is REQUIRED and its absence is an invocation
/// fault (exit 2), never a default — see the module doc.
fn parse(args: &[String]) -> Result<GenesisArgs, Fail> {
    let mut ruling: Option<String> = None;
    let mut archive: Option<String> = None;
    let mut dry = false;
    let mut json = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry" => dry = true,
            "--json" => json = true,
            "--ruling" => {
                ruling = Some(
                    it.next()
                        .ok_or_else(|| Fail::tool("--ruling takes a value".to_owned()))?
                        .clone(),
                );
            }
            "--archive" => {
                archive = Some(
                    it.next()
                        .ok_or_else(|| Fail::tool("--archive takes a value".to_owned()))?
                        .clone(),
                );
            }
            other => {
                return Err(Fail::tool(format!("unknown option '{other}'")));
            }
        }
    }

    let ruling = ruling.ok_or_else(|| {
        Fail::tool(
            "refused: --ruling is required — genesis records the authority it acted under, \
             and the engine never invents a justification it was not given"
                .to_owned(),
        )
    })?;
    if ruling.trim().is_empty() {
        return Err(Fail::tool(
            "refused: --ruling is empty — cite the ruling that authorised this reset".to_owned(),
        ));
    }

    Ok(GenesisArgs {
        ruling,
        archive,
        dry,
        json,
    })
}

/// The genesis leg: archive every row, truncate, and open the new chain with a
/// row describing what was done.
fn genesis(args: &GenesisArgs) -> Result<(), Fail> {
    let root = crate::preset_cmd::resolve_root()?;
    let journal_rel = fs::domain::RESERVED_JOURNAL_PATH;
    let journal_abs = root.0.join(journal_rel);

    let journal_text = std::fs::read_to_string(&journal_abs)
        .map_err(|e| Fail::tool(format!("no readable receipt journal at {journal_rel}: {e}")))?;
    let rows = receipt::journal::parse_rows(&journal_text);
    if rows.is_empty() {
        return Err(Fail::findings(format!(
            "refused: {journal_rel} carries no rows — there is nothing to archive, \
             and a genesis over an empty journal would record an act that did not happen"
        )));
    }

    let (now_unix, today) = now_facts()?;
    let archive_rel = args
        .archive
        .clone()
        .unwrap_or_else(|| format!("{ARCHIVE_DIR}/journal-archive-{today}.md"));
    let archive_abs = root.0.join(&archive_rel);
    if archive_abs.exists() {
        return Err(Fail::findings(format!(
            "refused: {archive_rel} already exists — two genesis runs onto one archive is a \
             mistake, not a batch; pass --archive to name a different page"
        )));
    }

    if args.dry {
        report(args, &archive_rel, rows.len(), None);
        return Ok(());
    }

    // The root the archive's creation moves. Both samples are real folds of the
    // hash domain: the archive page is IN it (only the journal itself is
    // excluded), so this bracket is an ordinary governed-write bracket.
    let root_before = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot fold the corpus: {e}")))?
        .1;
    let archive = archive_page(&journal_text, rows.len(), &args.ruling, &today, journal_rel);
    write_archive(&root, &archive_rel, archive)?;
    let root_after = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot fold the corpus: {e}")))?
        .1;
    truncate_journal(&root, journal_rel)?;

    // The new chain opens with the act that made it. Seq 1 by construction:
    // the journal is empty, so this row cannot be anything else.
    let row = receipt::journal::render_row(&receipt::journal::JournalRow {
        seq: 1,
        op: "genesis",
        path: &archive_rel,
        actor: None,
        now: Some(&now_unix),
        root_before: &root_before.0,
        root_after: &root_after.0,
        file: None,
        edits: Vec::new(),
    });
    fs::append_line(&root, Path::new(journal_rel), &row)
        .map_err(|e| Fail::tool(format!("cannot open the new chain in {journal_rel}: {e}")))?;

    report(args, &archive_rel, rows.len(), Some(&row));
    Ok(())
}

/// **Byte-landing door** (U12 census: `journal_cmd.rs::write_archive`, class
/// `OutsideThisUnit`) — land the archive page. Its own function so the census
/// key `(file, mint_fn)` names exactly one mint, which is the granularity that
/// census was re-keyed to after a second mint inside a known function slipped
/// past it.
fn write_archive(root: &fs::WorkspaceRoot, archive_rel: &str, page: String) -> Result<(), Fail> {
    fs::create_file(
        root,
        Path::new(archive_rel),
        &model::candidate_of_body(archive_rel, page),
    )
    .map_err(|e| Fail::tool(format!("cannot write {archive_rel}: {e}")))
}

/// **Byte-landing door** (U12 census: `journal_cmd.rs::truncate_journal`, class
/// `OutsideThisUnit`) — empty the live journal.
///
/// Ordering is load-bearing: this runs AFTER the archive is durable. A crash
/// between the two leaves the rows in BOTH places, which a reader can untangle;
/// the reverse order loses them.
///
/// The journal is root-excluded, so this lands bytes that move no root — which
/// is exactly why the genesis row describes the archive instead.
fn truncate_journal(root: &fs::WorkspaceRoot, journal_rel: &str) -> Result<(), Fail> {
    fs::replace_file(
        root,
        Path::new(journal_rel),
        &model::candidate_of_body(journal_rel, String::new()),
    )
    .map_err(|e| Fail::tool(format!("cannot truncate {journal_rel}: {e}")))
}

/// The archive page. Everything here is mechanical EXCEPT the justification,
/// which is why `--ruling` is a required input rather than a generated line:
/// the page's real value is why the reset was needed, and no verb can produce
/// that.
fn archive_page(
    journal_text: &str,
    rows: usize,
    ruling: &str,
    today: &str,
    journal_rel: &str,
) -> String {
    format!(
        "---\n\
         tags: [meta/health, type/archive]\n\
         created: {today}\n\
         status: superseded\n\
         genesis.ruling: '{ruling}'\n\
         genesis.rows: {rows}\n\
         genesis.from: '{journal_rel}'\n\
         description: The receipt journal as it stood before the {today} genesis reset — \
         {rows} rows, preserved verbatim, superseded never destroyed\n\
         ---\n\
         \n\
         # Receipt journal archive — pre-genesis, {today}\n\
         \n\
         > [!WARNING] This is a superseded record, not the live journal\n\
         > The live journal is `{journal_rel}`. These {rows} rows were moved here by\n\
         > `mrd journal genesis` under the authority cited in `genesis.ruling`\n\
         > above: **{ruling}**.\n\
         >\n\
         > Superseded, never destroyed — and never re-rendered either, since a\n\
         > re-render is a rewrite with better manners. The rows below are the\n\
         > bytes that were live.\n\
         \n\
         {journal_text}"
    )
}

/// Report the outcome. The two facts a reader most needs are the ones a reset
/// is most likely to be misread about: WHERE the rows went, and that the chain
/// is now GREY rather than green.
fn report(args: &GenesisArgs, archive_rel: &str, rows: usize, row: Option<&str>) {
    let dry = args.dry;
    match if args.json {
        Format::Json
    } else {
        Format::Human
    } {
        Format::Json => {
            let value = serde_json::json!({
                "dry": dry,
                "archive": archive_rel,
                "rows_archived": rows,
                "ruling": args.ruling,
                "genesis_row": row,
                "chain_after": "grey(no-baseline)",
            });
            println!("{value}");
        }
        Format::Human => {
            if dry {
                println!("genesis --dry: nothing written");
            }
            println!("archive: {archive_rel} ({rows} row(s))");
            println!("ruling: {}", args.ruling);
            if let Some(row) = row {
                println!("genesis row: {row}");
            }
            // Said out loud because a reset that renders grey WILL be read as a
            // failure by anyone who was not told first.
            println!(
                "chain after genesis: grey(no-baseline) — NOT green. The new chain has one row \
                 and nothing before it to continue from; that is the same grey the engine \
                 renders for any journal without a baseline."
            );
        }
    }
}

/// The §9 time facts, minted at the CLI boundary like every other verb's:
/// unix seconds for the row, and a `YYYY-MM-DD` date for the archive name.
fn now_facts() -> Result<(String, String), Fail> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Fail::tool(format!("cannot read the clock: {e}")))?
        .as_secs();
    Ok((secs.to_string(), civil_date(secs)))
}

/// `YYYY-MM-DD` in UTC from unix seconds — days-since-epoch through the civil
/// calendar (Howard Hinnant's algorithm), so the archive name needs no
/// date crate.
fn civil_date(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_civil_date_matches_known_epochs() {
        assert_eq!(civil_date(0), "1970-01-01");
        // 2026-07-28T00:00:00Z
        assert_eq!(civil_date(1_785_196_800), "2026-07-28");
    }

    #[test]
    fn a_missing_ruling_is_an_invocation_fault() {
        let err = parse(&[]).expect_err("genesis without --ruling must refuse");
        assert_eq!(err.code, 2);
        assert!(
            err.message.contains("--ruling is required"),
            "{}",
            err.message
        );
    }

    #[test]
    fn an_empty_ruling_is_refused_too() {
        let args = vec!["--ruling".to_owned(), "   ".to_owned()];
        let err = parse(&args).expect_err("a blank ruling is not a citation");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn the_archive_page_carries_the_ruling_and_the_rows() {
        let page = archive_page(
            "- op=splice ^r-000001\n",
            1,
            "Ruling A",
            "2026-07-28",
            "j.md",
        );
        assert!(page.contains("genesis.ruling: 'Ruling A'"), "{page}");
        assert!(page.contains("genesis.rows: 1"), "{page}");
        assert!(page.contains("status: superseded"), "{page}");
        assert!(page.contains("- op=splice ^r-000001"), "{page}");
    }
}
