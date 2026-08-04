//! **THE INSTRUMENT BEHIND THE COUNT.** Establishes the parser-invisible lock
//! set as a DERIVED QUERY over the live vaults — never a hand-listed set, which
//! is the trap the parser-blindness card names.
//!
//! Kept rather than thrown away because a count is only as good as the ability
//! to re-run it at a stated revision. Run it before quoting any number.
//!
//! For every markdown page in both vaults it reports two numbers:
//!   TEXT   — how many ` ```meridian-lock ` fence openers the BYTES contain
//!   PARSER — how many the ENGINE sees (`lock::block_spans`)
//! The gap is the invisible set. Every gap block is printed WITH ITS LINE AND
//! ITS ENCLOSING CONTEXT, so the classification is opened rather than asserted
//! (#23: if you classify a block, you opened it).
//!
//! # The category trap, measured the hard way
//! "A fence opener in the bytes" and "a lock block" are DIFFERENT CATEGORIES,
//! and two earlier versions of the TEXT arm were wrong in opposite directions:
//! `trim_end() == FENCE` MISSED a real block written inside a blockquote
//! (`> ```meridian-lock`), reporting 19/7/12; `contains(FENCE)` COUNTED prose
//! mentions and table cells as blocks, reporting 29/8/21. The arm below counts
//! FENCE-SHAPED openers only — line-initial after stripping blockquote markers
//! and indentation — and reports 20/8/12.
//!
//! All three runs were on the same tree. The number moved because the QUESTION
//! moved.
//!
//! Read-only. `cargo test -p lockmigrate --test parser_reach_survey -- --ignored --nocapture`

const VAULTS: &[&str] = &[
    "/Users/Shared/projects/field-notes",
    "/Users/Shared/projects/field-notes-sessions",
];

const FENCE: &str = "```meridian-lock";

#[test]
#[ignore = "reads the live vaults; run deliberately"]
fn survey_parser_reach() {
    let mut text_total = 0usize;
    let mut parser_total = 0usize;
    let mut gap_total = 0usize;

    for vault in VAULTS {
        let root = std::path::Path::new(vault);
        if !root.exists() {
            println!("SKIP {vault}");
            continue;
        }
        println!("\n################ {vault}");
        let mut pages: Vec<std::path::PathBuf> = Vec::new();
        collect(root, &mut pages);
        pages.sort();

        for page in pages {
            let Ok(raw) = std::fs::read_to_string(&page) else {
                continue;
            };
            // TEXT arm: count fence openers at line start, the same shape a
            // grep sees.
            // BARE substring, exactly what a grep sees. An earlier version of
            // this arm required `l.trim_end() == FENCE` and MISSED a fence
            // written inside a blockquote (`> ```meridian-lock`) that the
            // PARSER does see — an instrument narrower than the thing it was
            // being compared against, which made the gap wrong in the
            // reassuring direction.
            // FENCE-SHAPED only: the opener must START the line, after
            // stripping blockquote markers and indentation. Anything else is
            // the string appearing in PROSE or an inline code span, which is
            // not a block under any parser and must not be counted as one.
            //
            // Both earlier arms were wrong in opposite directions and it is
            // worth carrying why: `trim_end() == FENCE` MISSED a real block
            // written inside a blockquote; `contains(FENCE)` COUNTED prose
            // mentions and table cells as blocks. "A fence opener in the bytes"
            // and "a lock block" are different categories, and the gap between
            // text and parser is only meaningful for the first category.
            let text: Vec<(usize, &str)> = raw
                .lines()
                .enumerate()
                .filter(|(_, l)| {
                    let bare = l.trim_start().trim_start_matches(['>', ' ']).trim_start();
                    bare.starts_with(FENCE)
                })
                .map(|(i, l)| (i + 1, l))
                .collect();
            if text.is_empty() {
                continue;
            }
            // PARSER arm: what the ENGINE admits as a lock block.
            let doc = model::build(raw.clone(), syntax::parse(&raw));
            let spans = lock::block_spans(&doc);

            let rel = page.strip_prefix(root).unwrap_or(&page);
            text_total += text.len();
            parser_total += spans.len();

            if text.len() == spans.len() {
                println!(
                    "  ok   text={} parser={}  {}",
                    text.len(),
                    spans.len(),
                    rel.display()
                );
                continue;
            }

            let gap = text.len() - spans.len();
            gap_total += gap;
            println!(
                "  GAP  text={} parser={}  ({gap} invisible)  {}",
                text.len(),
                spans.len(),
                rel.display()
            );

            // OPEN each invisible one: which line, and what encloses it.
            let visible_lines: Vec<usize> = spans
                .iter()
                .map(|s| raw[..s.start].lines().count() + 1)
                .collect();
            for (line, _) in &text {
                if visible_lines.contains(line) {
                    continue;
                }
                println!("        line {line}: enclosing context ->");
                for (n, l) in raw.lines().enumerate() {
                    let n = n + 1;
                    // The nearest preceding line that opens a fence of 4+ backticks.
                    if n < *line && l.trim_start().starts_with("````") {
                        println!("          opener  L{n}: {}", l.trim_end());
                    }
                    if n >= *line {
                        break;
                    }
                }
            }
        }
    }

    println!("\n================ TOTALS");
    println!("  text-visible fence openers : {text_total}");
    println!("  parser-visible lock blocks : {parser_total}");
    println!("  INVISIBLE (the gap)        : {gap_total}");
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}
