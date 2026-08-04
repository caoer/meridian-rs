#!/usr/bin/env python3
"""U23 — mutation-proof each `mrd retire` pin: redden it, restore, prove it greens.

A pin that has never been watched to fail is a phantom-test candidate. For each
mutation below we re-introduce the exact defect the pin exists to catch, run ONLY
that pin, and record the exit code raw (never piped — a pipeline's exit is the
LAST stage's, and `${PIPESTATUS[0]}` is empty on this host; the array is
`$pipestatus` and is 1-indexed).

# This harness carries the two measured cures, and it did not invent them
All-Hands #6 recorded two ways a mutation harness lies, and it lies as "did not
fail", which reads as good news about your code:

1. **Stale artifact.** `shutil.copy2` preserves mtime, so a RESTORED file looks
   OLDER than the artifact cargo built from the MUTATED one. Cargo reuses the
   mutated binary and a LATER mutation runs with an EARLIER one still live.
   *Cure:* stamp restored files forward with `os.utime`.
2. **Filter matches nothing.** A test filter matching no test runs zero tests and
   exits 0 — indistinguishable from a pin that passed. *Cure:* refuse to read
   exit 0 as a pass when the log says `running 0 tests`.

Both are lifted from `tools/u20b-mutation-proof.py` rather than re-derived, per
that broadcast's instruction not to write a third harness that has to learn them
again.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = "crates/mrd/src/retire_cmd.rs"

# (name, test, [(file, old, new)], what the mutation re-creates)
MUTATIONS = [
    (
        "marker-span-exclusion",
        "a_second_mark_writes_nothing",
        [(
            SRC,
            "        if let Some(m) = marks.iter().find(|m| overlaps(&m.span, &span)) {",
            "        // MUTATION: stop excluding occurrences already inside a marker.\n"
            "        if let Some(m) = marks.iter().find(|m| overlaps(&m.span, &span)).filter(|_| false) {",
        )],
        "a mark operation whose image is inside its own domain — run 2 double-marks",
    ),
    (
        "positive-control",
        "a_blind_control_refuses_and_a_seeing_one_does_not",
        [(
            SRC,
            "        if counts.control == 0 {",
            "        // MUTATION: proceed on a scan the tool cannot vouch for.\n"
            "        if false && counts.control == 0 {",
        )],
        "a grep-to-zero with no positive control — a perfect pass on a wrong pattern",
    ),
    (
        "control-counts-its-own-declaration",
        "a_blind_control_refuses_and_a_seeing_one_does_not",
        [(
            SRC,
            "        if outside(&span) {\n            ctl += 1;\n        }",
            "        // MUTATION: count the control inside the block that DECLARES it.\n"
            "        ctl += 1;",
        )],
        "a control that matches its own `control:` line, so it reports healthy in every vault",
    ),
    (
        "engine-block",
        "an_engine_written_block_refuses_but_a_code_fence_only_skips",
        [(
            SRC,
            "            if lock::is_engine_emitted(&fence.lang) {",
            "            // MUTATION: write markers into bytes the ENGINE writes.\n"
            "            if false && lock::is_engine_emitted(&fence.lang) {",
        )],
        "a sweep that corrupts a meridian-lock block instead of annotating prose",
    ),
    (
        "count-classification",
        "the_second_runs_already_equals_the_first_runs_marked",
        [(
            SRC,
            "            if m.id == decl.id {\n                counts.already += 1;\n            } else {\n                counts.foreign += 1;\n            }",
            "            // MUTATION: mis-file this retirement's own markers as another's.\n"
            "            if m.id != decl.id {\n                counts.already += 1;\n            } else {\n                counts.foreign += 1;\n            }",
        )],
        "a count that does not describe the retirement it is printed under",
    ),
    (
        "orphan-silent",
        "an_undeclared_marker_id_is_reported_not_skipped",
        [(
            SRC,
            "        if !all_ids.contains(&m.id) {",
            "        // MUTATION: let a marker pointing at nothing pass silently.\n"
            "        if false && !all_ids.contains(&m.id) {",
        )],
        "a marker a reader can follow to nothing, forever, with a green board",
    ),
    (
        "reader-borrows-the-writers-clause",
        "the_refusals_are_not_one_assertion_wearing_many_names",
        [(
            SRC,
            'partial: "The report above is complete; nothing was written.".to_owned(),',
            "// MUTATION: give the READER the WRITER's partial-state clause.\n"
            "                partial: NO_MARK_CLAUSE.to_owned(), //",
        )],
        "five refusals that are one assertion wearing five names — and a report that "
        "claims it wrote nothing when it served its whole table",
    ),
    (
        "code-span-wrap",
        "a_code_span_term_is_struck_in_the_rendered_text_not_merely_in_the_bytes",
        [(
            SRC,
            "        let coded = code_span(raw, &span);",
            "        // MUTATION: wrap INSIDE the backticks. The raw bytes still look\n"
            "        // plausible; only the RENDERED text can tell.\n"
            "        let coded: Option<model::ByteSpan> = None;",
        )],
        "`~~` inside a code span, which markdown renders as four literal tildes",
    ),
    (
        "never-matched",
        "a_term_that_never_matched_refuses_rather_than_reporting_a_clean_sweep",
        [(
            SRC,
            "        if would_mark == 0 && counts.already == 0 {",
            "        // MUTATION: report a term that never matched as a clean sweep.\n"
            "        if false && would_mark == 0 && counts.already == 0 {",
        )],
        "the arm the positive control cannot see — a wrong term reading exactly as a "
        "finished retirement (all-hands #3)",
    ),
    (
        "world-guard",
        "mark_demands_a_world_guard_and_a_stale_one_refuses",
        [(
            SRC,
            "    if writing && !parsed.dry_run && parsed.expect_root.is_none() {",
            "    // MUTATION: let a sweep write with no world guard.\n"
            "    if false && writing && !parsed.dry_run && parsed.expect_root.is_none() {",
        )],
        "U9b's measured failure industrialized: a vault moving under a running sweep",
    ),
    (
        "contract-cause-dropped",
        "every_reason_word_carries_the_four_property_contract",
        [(
            SRC,
            'cause: "the engine writes those bytes, and a marker in them would corrupt an engine artifact rather than annotate prose.".to_owned(),',
            "// MUTATION: ship a refusal that states no cause.\n"
            '                    cause: String::new(),',
        )],
        "a refusal missing one of the four properties, on a surface whose contract "
        "was only ever checked on the two refusals that happened to fire together",
    ),
    (
        "contract-fix-not-runnable",
        "every_reason_word_carries_the_four_property_contract",
        [(
            SRC,
            '            fix.push_str(" Then run `mrd retire report --json` to confirm the block parses.");',
            "            // MUTATION: leave a parse fix as an edit instruction with no way\n"
            "            // to check it — the gap the coverage sweep actually found.",
        )],
        "a fix clause that describes the problem's inverse instead of naming a "
        "runnable command",
    ),
    (
        "ninth-reason-word-uncovered",
        "every_reason_word_carries_the_four_property_contract",
        [
            (SRC,
             "    TermNeverMatched,\n}",
             "    TermNeverMatched,\n    // MUTATION: a ninth reason word with no fixture.\n    NinthProbe,\n}"),
            (SRC,
             "    pub const ALL: [Reason; 8] = [",
             "    pub const ALL: [Reason; 9] = ["),
            (SRC,
             "        Reason::TermNeverMatched,\n    ];",
             "        Reason::TermNeverMatched,\n        Reason::NinthProbe,\n    ];"),
            (SRC,
             '            Reason::TermNeverMatched => "retire-term-never-matched",',
             '            Reason::TermNeverMatched => "retire-term-never-matched",\n'
             '            Reason::NinthProbe => "retire-ninth-probe",'),
            (SRC,
             "        Reason::TermNeverMatched => 7,",
             "        Reason::TermNeverMatched => 7,\n        Reason::NinthProbe => 8,"),
        ],
        "a ninth refusal shipping with NO contract coverage — the world the old "
        "census could not fail in, because it compared two hand-written lists to "
        "each other",
    ),
    (
        "measured-declared-split",
        "the_report_never_mixes_what_it_measured_with_what_it_was_told",
        [(
            SRC,
            '            "declared": { "proofs": d.proofs, "verified_by_this_tool": false },',
            '            // MUTATION: merge the two evidence classes into one table.\n'
            '            "proofs": d.proofs,',
        )],
        "a report in which a reader cannot tell what the tool measured from what it was told",
    ),
]


def run(test: str, package: str = "mrd", binary: str = "u23_retire") -> int:
    """Run ONE pin and return cargo's real exit code.

    The filter must reach a real test: a filter matching nothing runs zero tests
    and exits 0, which a harness reads as "the pin did not fail". Cure 2.
    """
    log = Path(tempfile.gettempdir()) / f"u23-mut-{test or 'suite'}.log"
    with log.open("wb") as fh:
        proc = subprocess.run(
            ["cargo", "test", "-p", package, "--test", binary, test],
            cwd=ROOT, stdout=fh, stderr=subprocess.STDOUT, check=False,
        )
    text = log.read_text(errors="replace")
    if proc.returncode == 0 and "running 0 tests" in text:
        raise SystemExit(f"{test}: filter matched no test in {package}/{binary}")
    return proc.returncode


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    results = []
    for name, test, edits, recreates in MUTATIONS:
        if only and only != name:
            continue
        backups = {}
        try:
            for rel, old, new in edits:
                path = ROOT / rel
                # LIE 3, measured here: back up on FIRST TOUCH ONLY.
                #
                # The inherited loop backed up before every edit and keyed the
                # store by path, so a row with N edits on ONE file stored the
                # state after edit N-1 as the "pristine" copy. Restore then put
                # the file back MOSTLY MUTATED, the row still reported REDDENS,
                # and the corruption surfaced only in the restored-tree check at
                # the very end — after which every later row runs against a
                # broken tree. `git status` showed 88 insertions the row was
                # supposed to have removed.
                #
                # It presents as a LATER row failing, which is All-Hands #8's
                # shape: the damage surfaces downstream of the line that caused
                # it. u20b-mutation-proof.py has the same loop and at least one
                # multi-edit row on a single file.
                if path not in backups:
                    backup = Path(tempfile.mkdtemp()) / path.name
                    shutil.copy2(path, backup)
                    backups[path] = backup
                text = path.read_text()
                if old not in text:
                    raise SystemExit(f"{name}: anchor not found in {rel}")
                path.write_text(text.replace(old, new, 1))
            code = run(test)
        finally:
            for path, backup in backups.items():
                shutil.copy2(backup, path)
                assert path.read_bytes() == backup.read_bytes(), f"restore failed: {path}"
                # Cure 1 — `copy2` preserves mtime, which makes the RESTORED file
                # look older than the artifact cargo built from the MUTATED one,
                # so the next run silently reuses a mutated binary. Stamp forward.
                os.utime(path, (time.time(), time.time()))
        verdict = "REDDENS" if code != 0 else "*** DID NOT FAIL ***"
        results.append((name, code, verdict, recreates))
        print(f"{verdict:22} {name:34} exit={code}  ({recreates})", flush=True)

    print("\n--- restored tree: proving the pins are green again ---", flush=True)
    green = run("")
    print(f"clean u23_retire suite exit={green}")
    bad = [r for r in results if r[1] == 0]
    if green:
        bad.append(("suite", green, "", "the restored tree is not green"))
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
