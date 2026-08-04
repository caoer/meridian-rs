#!/usr/bin/env python3
"""Prove the residue-gate guard reddens on every way its gates can vanish.

`board_residue` is a disclosed counter whose production value is permanently
ZERO by ruling, so the five tests in `crates/view/tests/board_residue_gates.rs`
are not evidence ABOUT the instrument — they ARE it. This script re-creates each
way they could disappear and records which arm catches it.

THE ROWS ARE THE INVALIDATION CONDITIONS (all-hands 22), executed rather than
asserted in prose. Each row names the change the guard claims immunity to, makes
exactly that change, and records cargo's real exit code. A condition nobody ran
is a comfortable sentence.

ONE EDIT PER ROW, so a red can be attributed to the defect it names — the
attribution rule the u20b harness learned by conflating two edits into one row.
The row is skipped-and-reported rather than aborting the run when an anchor goes
stale, because a harness that stops early reports a lower bound as a total.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATES = ROOT / "crates/view/tests/board_residue_gates.rs"
GUARD = ROOT / "crates/view/tests/residue_gates_may_not_vanish.rs"

# Which arm each row exercises, spelled out so a reader can see the split rather
# than infer it from a name:
#   ARM 1 — crates/view/Cargo.toml declares both files as [[test]] targets, so
#           the change fails at TARGET RESOLUTION. Not a test; cannot be
#           filtered, ignored, or skipped.
#   ARM 2 — tests/residue_gates_may_not_vanish.rs, which reads the real bytes.
#   CONTROL — arm 2's own scanner, checked against hand-authored input.

# (name, arm, kind, payload, the condition this row tests)
ROWS = [
    (
        "gates-file-deleted",
        "ARM 1",
        "delete",
        GATES,
        "deleting the gates file is a cargo target-resolution failure",
    ),
    (
        "gates-file-renamed",
        "ARM 1",
        "rename",
        GATES,
        "renaming the gates file cannot quietly orphan it",
    ),
    (
        "guard-file-deleted",
        "ARM 1",
        "delete",
        GUARD,
        "the guard is pinned by the same mechanism it provides — deleting IT is "
        "also a build failure, so arm 2 cannot be removed silently either",
    ),
    (
        "test-renamed",
        "ARM 2",
        "edit",
        (
            GATES,
            "fn the_synthetic_row_sits_between_classified_rows()",
            "fn the_synthetic_row_sits_between_classified_rows_RENAMED()",
        ),
        "a renamed gate reads as a MISSING gate, never as a passing one",
    ),
    (
        "test-ignored",
        "ARM 2",
        "edit",
        (
            GATES,
            "#[test]\nfn a_verdict_less_row_vanishes_from_board_and_is_disclosed_by_residue",
            "#[test]\n#[ignore]\nfn a_verdict_less_row_vanishes_from_board_and_is_disclosed_by_residue",
        ),
        "THE CENTRAL ROW: #[ignore] is the disarm that emits the same bytes to a "
        "gate as a pass",
    ),
    (
        "ignore-with-a-reason",
        "ARM 2",
        "edit",
        (
            GATES,
            "#[test]\nfn predicate_is_one_definition_shared_by_both_projections",
            '#[test]\n#[ignore = "flaky in CI"]\nfn predicate_is_one_definition_shared_by_both_projections',
        ),
        "the plausible spelling of the same disarm — the one a literal "
        "`#[ignore]` match would miss",
    ),
    (
        "test-attribute-removed",
        "ARM 2",
        "edit",
        (
            GATES,
            "#[test]\nfn residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty",
            "fn residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty",
        ),
        "disarm IN PLACE: the function stays, the file keeps its length, nothing "
        "runs",
    ),
    (
        "unpinned-gate-added",
        "ARM 2",
        "edit",
        (
            GATES,
            "// ---------------------------------------------------------------------------\n// helpers",
            "#[test]\nfn a_new_gate_nobody_pinned() {}\n\n"
            "// ---------------------------------------------------------------------------\n// helpers",
        ),
        "a gate added without a pin is a gate the next deletion meets no red for",
    ),
    (
        "scanner-blinded",
        "CONTROL",
        "edit",
        (
            GUARD,
            '} else if line.starts_with("#[ignore") {\n                ignored = true;',
            "} else if false {\n                ignored = true;",
        ),
        "arm 2's own machinery: blind the scanner to #[ignore] and its control "
        "reddens, so the two assertions above are evidence rather than output",
    ),
]


def run(tag: str) -> int:
    """Run the view package and return cargo's real exit code, never a pipe's.

    The WHOLE package, not one binary: an arm-1 row is a build failure and there
    is no test binary left to point at.

    ONE LOG PER ROW, and the name is load-bearing. A single shared log is
    overwritten by the final restored-tree run, so the only surviving evidence
    describes the GREEN tree — a reader who opens it to check WHY a row reddened
    is handed the state after the mutation was undone, and every row looks
    incidental. Measured here: the first version of this script did exactly that.
    """
    log = Path(tempfile.gettempdir()) / f"residue-gates-{tag}.log"
    with log.open("wb") as fh:
        proc = subprocess.run(
            ["cargo", "test", "-p", "view"],
            cwd=ROOT, stdout=fh, stderr=subprocess.STDOUT, check=False,
        )
    return proc.returncode


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    results = []

    for name, arm, kind, payload, condition in ROWS:
        if only and only != name:
            continue
        backups: dict[Path, Path] = {}
        stale = None
        moved_to = None
        try:
            if kind == "edit":
                target, old, new = payload
                backups[target] = Path(tempfile.mkdtemp()) / target.name
                shutil.copy2(target, backups[target])
                text = target.read_text()
                if old not in text:
                    # A stale anchor is a ROW FAILURE, not a run abort: aborting
                    # here would silently cancel every row below while the output
                    # still read like a list of REDDENS.
                    stale = target
                else:
                    target.write_text(text.replace(old, new, 1))
            else:
                backups[payload] = Path(tempfile.mkdtemp()) / payload.name
                shutil.copy2(payload, backups[payload])
                if kind == "delete":
                    payload.unlink()
                else:  # rename
                    moved_to = payload.with_name(payload.stem + "_moved.rs")
                    payload.rename(moved_to)
            code = None if stale else run(name)
        finally:
            if moved_to and moved_to.exists():
                moved_to.unlink()
            for target, backup in backups.items():
                shutil.copy2(backup, target)
                assert target.read_bytes() == backup.read_bytes(), f"restore failed: {target}"
                # copy2 preserves mtime, which makes the RESTORED file look older
                # than the artifact cargo built from the mutated one — the next
                # row then reuses a mutated binary and reports a false green.
                os.utime(target, (time.time(), time.time()))

        if stale:
            verdict = "*** ANCHOR STALE ***"
        else:
            verdict = "REDDENS" if code != 0 else "*** DID NOT FAIL ***"
        results.append((name, code, verdict))
        print(f"{verdict:22} {arm:8} {name:24} exit={code}  ({condition})", flush=True)

    print("\n--- restored tree: the gates and their guard are green again ---", flush=True)
    green = run("restored")
    print(f"clean view suite exit={green}")
    bad = [r for r in results if r[1] == 0 or r[1] is None]
    return 1 if bad or green else 0


if __name__ == "__main__":
    sys.exit(main())
