#!/usr/bin/env python3
"""U20b — mutation-proof each §7 pin: redden it, restore, prove it greens again.

A pin that has never been watched to fail is a phantom-test candidate. For each
mutation below we re-introduce the exact defect the pin exists to catch, run ONLY
that pin, and record the exit code raw (never piped — the pipeline's exit is
`tail`'s, always 0).

Every file is backed up byte-for-byte before mutation and restored after, and the
restore is VERIFIED by comparison, so a crash mid-run cannot leave a mutation in
the tree.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (name, test, [(file, old, new)], what the mutation re-creates)
MUTATIONS = [
    (
        "framing",
        "framing_a_subscription_never_desyncs_another_connection",
        [(
            "crates/registry/src/server.rs",
            "        let out = handle_line(registry, &mut attached, &mut rev, &mut armed, &line);\n"
            "        writer.write_all(out.as_bytes())?;",
            "        let out = handle_line(registry, &mut attached, &mut rev, &mut armed, &line);\n"
            "        // MUTATION: interleave push frames onto the REQUEST connection.\n"
            "        if let Some(ws) = attached.as_deref() {\n"
            "            let ring = registry.ring(ws);\n"
            "            for frame in ring.frames_after(0) {\n"
            "                wire_serve::ring::write_frame(&mut writer, &frame, rev == Rev::V3)?;\n"
            "            }\n"
            "        }\n"
            "        writer.write_all(out.as_bytes())?;",
        )],
        "the sidecar's interleaved framing: notifications on a request connection",
    ),
    (
        "s6-keying",
        "s6_one_ring_per_workspace_not_per_spelling",
        [(
            "crates/registry/src/registry.rs",
            "Arc::clone(rings.entry(workspace.to_path_buf()).or_insert_with(|| {",
            "// MUTATION: ONE GLOBAL RING for every workspace.\n"
            "        Arc::clone(rings.entry(std::path::PathBuf::from(\"/\")).or_insert_with(|| {",
        )],
        "the S6 defect the constraint names: one global ring leaking across workspaces",
    ),
    (
        "s2-gate",
        "s2_sub_stands_behind_the_workspace_bind",
        [(
            "crates/registry/src/server.rs",
            "    let Some(ws) = attached else {\n        return Err(wire_serve::bad_request(",
            "    // MUTATION: route `sub` BEFORE the bound-workspace guard.\n"
            "    if matches!(op, Op::Sub { .. }) && attached.is_none() {\n"
            "        *armed = Some(0);\n"
            "        return Ok(ResponseBody::Root { root: Root(\"b3:0\".into()), seq: 0 });\n"
            "    }\n"
            "    let Some(ws) = attached else {\n        return Err(wire_serve::bad_request(",
        )],
        "a `sub` that reaches past the gate every other wire op stands behind",
    ),
    (
        "mint-isolation",
        "a_pushed_frame_mints_no_read_receipt",
        [(
            "crates/registry/src/server.rs",
            "        for frame in ring.frames_after(delivered) {\n"
            "            wire_serve::ring::write_frame(writer, &frame, rev == Rev::V3)?;\n"
            "            delivered = frame.delta.seq;\n"
            "        }",
            "        for frame in ring.frames_after(delivered) {\n"
            "            wire_serve::ring::write_frame(writer, &frame, rev == Rev::V3)?;\n"
            "            // MUTATION: mint a read receipt from the PUSH path.\n"
            "            registry\n"
            "                .read_mints(ws)\n"
            "                .mint(\"agent:subscriber\", \"plan.md\", \"Goals\", \"pushed\");\n"
            "            delivered = frame.delta.seq;\n"
            "        }",
        )],
        "a read-only channel opening a write door by minting from frames",
    ),
    (
        "seq-contiguity",
        "the_delta_chain_is_contiguous",
        [(
            "crates/wire-serve/src/watch.rs",
            "            let mut frame = crate::write::assemble_delta(\n                ring.seq(),",
            "            // MUTATION: allocate `seq` from a stale snapshot instead of\n"
            "            // the live ring — the defect a seq source outside the lock has.\n"
            "            let mut frame = crate::write::assemble_delta(\n                0,",
        )],
        "a seq allocator reading a stale counter — the class the SeqSink rule prevents",
    ),
    (
        "v2-reserved-effects",
        ("wire-serve", "u20b_v2_notification_shape",
         "a_v2_notification_frame_never_grows_a_post_v2_field"),
        [(
            "crates/wire-serve/src/rev.rs",
            'pub const V2_RESERVED_FIELDS: &[ReservedField] = &[ReservedField {\n    key: "effects",',
            '// MUTATION: withdraw the `effects` row from the registry.\n'
            'pub const V2_RESERVED_FIELDS: &[ReservedField] = &[ReservedField {\n    key: "effects_WITHDRAWN",',
        )],
        "the measured frozen-v2 leak: a post-v2 field on a v2 subscriber's wire",
    ),
    (
        "v2-reserved-ladder-vintage",
        ("wire-serve", "u20b_v2_notification_shape",
         "a_post_v2_field_is_demoted_even_with_no_authorship_mark"),
        [(
            "crates/wire-serve/src/rev.rs",
            '        key: "new_fingerprint",\n        position: Position::ErrorPayload,',
            '        // MUTATION: withdraw the vintage row for a ladder extra.\n'
            '        key: "new_fingerprint_WITHDRAWN",\n        position: Position::ErrorPayload,',
        )],
        "a post-v2 ladder field reaching a v2 session because vintage was keyed on authorship",
    ),
    (
        "v2-provenance-not-vintage",
        ("wire-serve", "u20b_v2_notification_shape",
         "a_non_ladder_refusal_keeps_its_v2_legal_message_and_path"),
        # ONE edit, deliberately. The two-edit version of this row is what
        # lie 3 corrupted, and it also masked a blind pin: it reddened via the
        # SECOND edit (disabling the early return) while the pin could not
        # detect the FIRST edit at all. One edit = the honest defect, and the
        # pin must catch it unaided.
        [(
            "crates/wire-serve/src/rev.rs",
            "        if ladder_authored {\n            error.message = None;\n            error.path = None;\n        }",
            "        // MUTATION: treat the v2-LEGAL slots as vintage — the\n"
            "        // regression the \"migrate all six\" order would have shipped.\n"
            "        error.message = None;\n        error.path = None;",
        )],
        "stripping a legitimate v2 message from a non-ladder refusal",
    ),
    (
        "reap-exemption",
        "a_live_subscription_survives_the_reaper",
        [(
            "crates/registry/src/registry.rs",
            ".filter(|(key, entry)| entry.last_use <= cutoff && !subscribed.contains(*key))",
            "// MUTATION: reap without exempting live subscriptions.\n"
            "                .filter(|(_, entry)| entry.last_use <= cutoff)",
        )],
        "reaping a workspace out from under a parked subscriber",
    ),
]


def run(test: str, package: str = "registry", binary: str = "sub_push") -> int:
    """Run ONE pin and return cargo's real exit code.

    The filter must reach a real test: a filter matching nothing runs zero tests
    and exits 0, which a mutation harness reads as "the pin did not fail".
    Measured — an early version pointed every mutation at one binary and a
    wire-serve pin silently reported DID NOT FAIL while never running.
    """
    log = Path(tempfile.gettempdir()) / f"u20b-mut-{test}.log"
    with log.open("wb") as fh:
        proc = subprocess.run(
            ["cargo", "test", "-p", package, "--test", binary, test],
            cwd=ROOT, stdout=fh, stderr=subprocess.STDOUT, check=False,
        )
    if proc.returncode == 0 and "running 0 tests" in log.read_text(errors="replace"):
        raise SystemExit(f"{test}: filter matched no test in {package}/{binary}")
    return proc.returncode


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    results = []
    for name, test, edits, recreates in MUTATIONS:
        if only and only != name:
            continue
        package, binary = "registry", "sub_push"
        if isinstance(test, tuple):
            package, binary, test = test
        backups = {}
        try:
            for rel, old, new in edits:
                path = ROOT / rel
                # BACK UP ON FIRST TOUCH ONLY. Taking the backup inside the loop
                # and keying it by path means a row with N edits on ONE file
                # stores the state after edit N-1 as its "pristine" copy — the
                # restore then puts the file back MOSTLY MUTATED, and the row
                # still prints REDDENS, because that verdict is a claim about
                # the TEST while the restore is a claim about the SOURCE.
                # Measured, not theorised: this shipped an unconditional strip
                # to a pushed branch.
                if path not in backups:
                    backup = Path(tempfile.mkdtemp()) / path.name
                    shutil.copy2(path, backup)
                    backups[path] = backup
                text = path.read_text()
                if old not in text:
                    raise SystemExit(f"{name}: anchor not found in {rel}")
                path.write_text(text.replace(old, new, 1))
            code = run(test, package, binary)
        finally:
            for path, backup in backups.items():
                shutil.copy2(backup, path)
                assert path.read_bytes() == backup.read_bytes(), f"restore failed: {path}"
                # `copy2` preserves mtime, which makes the RESTORED file look
                # older than the artifact cargo built from the MUTATED one — so
                # the next run silently reuses a mutated binary. Measured: it
                # made a later mutation report "did not fail" while an unrelated
                # mutation was still live in the rlib. Stamp it forward.
                os.utime(path, (time.time(), time.time()))
        verdict = "REDDENS" if code != 0 else "*** DID NOT FAIL ***"
        results.append((name, test, code, verdict, recreates))
        print(f"{verdict:22} {name:16} exit={code}  ({recreates})", flush=True)

    print("\n--- restored tree: proving the pins are green again ---", flush=True)
    green = run("")
    print(f"clean registry suite exit={green}")
    shape = subprocess.run(
        ["cargo", "test", "-p", "wire-serve", "--test", "u20b_v2_notification_shape"],
        cwd=ROOT, capture_output=True, check=False,
    ).returncode
    print(f"clean v2-shape suite exit={shape}")
    green = green or shape
    bad = [r for r in results if r[2] == 0] or ([("suite", "", green, "", "")] if green else [])
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
