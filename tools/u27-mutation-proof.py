#!/usr/bin/env python3
"""U27 — two-arm proof for every frozen-v2 key-set pin.

A pin that has never been WATCHED to fail is not known to be a detector
(All-Hands #3: a control that emits the same bytes in the passing and the
failing world is a decoration). So for each arm below we build the failing
world explicitly, run the pins, and record WHICH ones reddened — then restore
and prove the same pins green again. The diff between the two arms is the
proof; "it would have caught it" is not.

Both All-Hands #6 cures are applied, because a mutation harness that lies says
DID-NOT-FAIL, which reads as good news:

  LIE 1, stale artifact — `shutil.copy2`/`cp -p` preserve mtime, so a RESTORED
  file looks older than the artifact cargo built from the MUTATED one and cargo
  reuses the mutated binary. Cure: restore with plain copy and stamp mtime
  FORWARD (`os.utime`) so every arm rebuilds.

  LIE 2, filter matches nothing — a filter matching no test runs zero tests and
  exits 0, indistinguishable from a pin that passed. Cure: run whole test
  BINARIES (no name filter) and refuse any log containing "running 0 tests".

Each arm also declares the EXACT set of tests it must redden. An arm that
reddens more or fewer than declared fails the proof, so an over-broad mutation
cannot be mistaken for a precise detector.

Usage: python3 tools/u27-mutation-proof.py [arm-name]
"""

import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

LIVE = ("sidecar", "u27_v2_key_set_pins")
TYPE = ("wire", "u27_frozen_key_sets")

def injector(indent: str) -> str:
    """A self-contained recursive key injector, defined AT the mutation site.

    It is inlined into each edit rather than shared, so every arm below is ONE
    edit at ONE site (All-Hands #16, lie 4: a multi-edit row can redden for an
    edit you did not name, and you cannot attribute the red). Every line carries
    the `MUTATION:` marker so the fleet arm-D audit
    (`git grep -n "MUTATION:" HEAD`) can see this harness's residue — a marker
    only this harness recognises would be invisible to it (All-Hands #15).
    """
    body = [
        "fn u27_leak(v: &mut serde_json::Value) { // MUTATION:",
        "    match v { // MUTATION:",
        "        serde_json::Value::Object(o) => { // MUTATION:",
        "            for (_k, child) in o.iter_mut() { u27_leak(child); } // MUTATION:",
        '            o.insert("v3_leak".to_string(), serde_json::json!(1)); // MUTATION:',
        "        } // MUTATION:",
        "        serde_json::Value::Array(a) => { // MUTATION:",
        "            for child in a.iter_mut() { u27_leak(child); } // MUTATION:",
        "        } // MUTATION:",
        "        _ => {} // MUTATION:",
        "    } // MUTATION:",
        "} // MUTATION:",
    ]
    return "".join(f"{indent}{line}\n" for line in body)

DEMOTE_ANCHOR = (
    "        let demoted = rev::demote_v2(response);\n"
    "        serde_json::to_writer(&mut *output, demoted.as_ref().unwrap_or(response))?;"
)

NOTIFY_ANCHOR = (
    "            } else {\n"
    "                serde_json::to_writer(&mut *output, &frame)?;\n"
    "            }"
)

# (arm, [(file, old, new)], {binary: {tests that MUST redden}}, what it recreates)
ARMS = [
    (
        "demotion-disabled",
        [(
            "crates/sidecar/src/lib.rs",
            DEMOTE_ANCHOR,
            "        let demoted: Option<wire::Response> = None; // MUTATION:\n"
            "        serde_json::to_writer(&mut *output, demoted.as_ref().unwrap_or(response))?;",
        )],
        {
            LIVE: {"cas_mismatch_error_key_set_is_frozen"},
            TYPE: set(),
        },
        "THE MEASURED U11 LEAK: rev::demote_v2 bypassed, so the v3-additive "
        "mismatch ladder (rung/diff/new_fingerprint/message/path) rides a v2 "
        "cas_mismatch. Live-only by construction — the TYPE plane cannot see it.",
    ),
    (
        "leak-response-objects",
        [(
            "crates/sidecar/src/lib.rs",
            DEMOTE_ANCHOR,
            "        let demoted = rev::demote_v2(response);\n"
            + injector("        ")
            + "        let mut v = serde_json::to_value(demoted.as_ref().unwrap_or(response))?;\n"
            "        u27_leak(&mut v); // MUTATION:\n"
            "        serde_json::to_writer(&mut *output, &v)?;",
        )],
        {LIVE: "ALL_BUT_NOTIFICATION", TYPE: set()},
        "one v3-additive key on EVERY object of every v2 RESPONSE frame — body, "
        "error, and every nested row. Proves each response pin distinguishes an "
        "added key AT ITS OWN GRAIN, not merely at body level. The notification "
        "pin must stay GREEN here: it reads a frame this site never writes, and "
        "that separation is what makes the next arm's red attributable.",
    ),
    (
        "leak-notification-objects",
        [(
            "crates/sidecar/src/lib.rs",
            NOTIFY_ANCHOR,
            "            } else {\n"
            + injector("                ")
            + "                let mut v = serde_json::to_value(&frame)?;\n"
            "                u27_leak(&mut v); // MUTATION:\n"
            "                serde_json::to_writer(&mut *output, &v)?;\n"
            "            }",
        )],
        {
            LIVE: {"delta_notification_key_sets_are_frozen"},
            TYPE: set(),
        },
        "the same leak on the NOTIFICATION path, which `flush_subs` writes — a "
        "different code path from every response. Exactly one pin reads that "
        "frame, so this red is attributable to this edit alone.",
    ),
    (
        "content-span-population-flip",
        [(
            "crates/wire/src/lib.rs",
            '    #[serde(skip_serializing_if = "Option::is_none")]\n'
            "    pub content_span: Option<Span>,\n"
            "    pub node_rev: NodeRev,",
            "    #[serde(skip_serializing)]\n"
            "    pub content_span: Option<Span>,\n"
            "    pub node_rev: NodeRev,",
        )],
        {
            LIVE: {"toc_body_and_row_key_sets_are_frozen"},
            TYPE: {"toc_row_key_set_is_frozen"},
        },
        "a changed POPULATION RULE on an existing Option — no field added, no "
        "field removed. This is the class that leaves `git diff` on the wire "
        "types looking innocent.",
    ),
    (
        "v3-field-serde-flip",
        [(
            "crates/wire/src/lib.rs",
            '    #[serde(skip_serializing_if = "Option::is_none")]\n    pub words: Option<u64>,\n}',
            "    #[serde(skip_serializing)]\n    pub words: Option<u64>,\n}",
        )],
        {
            LIVE: set(),
            TYPE: {"extract_node_key_set_is_frozen_plus_the_v3_host_face_trio"},
        },
        "a v3-additive field that no live v2 path populates. TYPE-only by "
        "construction: the live plane is BLIND here, which is why the suite is "
        "a pair and not one file.",
    ),
]


def run_binary(package, binary):
    """Run a whole test binary. Returns (exit_code, failed_test_names)."""
    log = Path(tempfile.gettempdir()) / f"u27-mut-{package}-{binary}.log"
    with log.open("wb") as fh:
        proc = subprocess.run(
            ["cargo", "test", "-p", package, "--test", binary],
            cwd=ROOT, stdout=fh, stderr=subprocess.STDOUT, check=False,
        )
    text = log.read_text(errors="replace")
    # CURE 2: "did not fail" and "never ran" are the same bytes without this.
    if "running 0 tests" in text:
        raise SystemExit(f"{package}/{binary}: ran ZERO tests — proof is void")
    if "running " not in text:
        raise SystemExit(f"{package}/{binary}: no test run at all — proof is void\n{text[-2000:]}")
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", text, re.MULTILINE))
    ran = set(re.findall(r"^test (\S+) \.\.\. (?:ok|FAILED)$", text, re.MULTILINE))
    return proc.returncode, failed, ran


def apply_edits(edits):
    backups = {}
    for rel, old, new in edits:
        path = ROOT / rel
        if path not in backups:
            backups[path] = path.read_bytes()
        text = path.read_text()
        if old not in text:
            raise SystemExit(f"anchor not found in {rel}:\n{old[:200]}")
        path.write_text(text.replace(old, new, 1))
    return backups


def restore(backups):
    for path, blob in backups.items():
        path.write_bytes(blob)
        assert path.read_bytes() == blob, f"restore failed: {path}"
        # CURE 1: never preserve mtime on restore, or cargo reuses the binary
        # built from the MUTATED source and a later arm reports DID-NOT-FAIL.
        now = time.time()
        os.utime(path, (now, now))


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    bad = []

    print("=== baseline: the pins must be GREEN before any arm ===", flush=True)
    for pkg, binary in (LIVE, TYPE):
        code, failed, ran = run_binary(pkg, binary)
        print(f"  {pkg}/{binary}: exit={code} ran={len(ran)} failed={sorted(failed)}")
        if code != 0:
            bad.append(f"baseline {pkg}/{binary} is not green")

    for name, edits, expect, recreates in ARMS:
        if only and only != name:
            continue
        print(f"\n=== arm: {name} ===\n  recreates: {recreates}", flush=True)
        backups = apply_edits(edits)
        try:
            for target, want in expect.items():
                pkg, binary = target
                code, failed, ran = run_binary(pkg, binary)
                if want == "ALL":
                    want = ran
                elif want == "ALL_BUT_NOTIFICATION":
                    want = ran - {"delta_notification_key_sets_are_frozen"}
                verdict = "OK" if failed == want else "*** PROOF FAILED ***"
                if failed != want:
                    bad.append(f"{name}/{binary}: reddened {sorted(failed)}, expected {sorted(want)}")
                print(
                    f"  {verdict:22} {pkg}/{binary} exit={code} ran={len(ran)} "
                    f"reddened={len(failed)}/{len(want)}",
                    flush=True,
                )
                if failed and len(failed) <= 4:
                    for t in sorted(failed):
                        print(f"      red: {t}")
        finally:
            restore(backups)

    # REDDENS is a claim about the TEST; the two checks below are the claim
    # about the SOURCE (All-Hands #16). They are different claims and a run
    # that reads only the first is checking its tests while its source rots.
    print("\n=== restored tree, claim 1 of 2: the pins must be GREEN again ===", flush=True)
    for pkg, binary in (LIVE, TYPE):
        code, failed, ran = run_binary(pkg, binary)
        print(f"  {pkg}/{binary}: exit={code} ran={len(ran)} failed={sorted(failed)}")
        if code != 0:
            bad.append(f"post-restore {pkg}/{binary} is not green")

    print("\n=== restored tree, claim 2 of 2: NO mutation residue in source ===", flush=True)
    # Two scopes, because each is blind to the other (All-Hands #14/#15):
    # the working tree cannot see committed residue, and HEAD cannot see
    # uncommitted residue. Neither sees residue below HEAD in history.
    dirty = subprocess.run(
        ["git", "status", "--short"], cwd=ROOT, capture_output=True, text=True, check=False
    ).stdout.strip()
    print(f"  working tree: {dirty or '(clean)'}")
    for scope in (["git", "grep", "-n", "MUTATION:", "--", "*.rs"],
                  ["git", "grep", "-n", "MUTATION:", "HEAD", "--", "*.rs"]):
        hits = [
            h for h in subprocess.run(
                scope, cwd=ROOT, capture_output=True, text=True, check=False
            ).stdout.splitlines()
            if "u27-mutation-proof" not in h
        ]
        label = "HEAD" if "HEAD" in scope else "working tree"
        print(f"  MUTATION: in {label}: {len(hits)} hit(s)")
        for h in hits:
            print(f"      {h}")
            bad.append(f"mutation residue in {label}: {h}")

    if bad:
        print("\nPROOF INCOMPLETE:")
        for b in bad:
            print(f"  - {b}")
        return 1
    print("\nAll arms proved: every pin was watched to redden in its failing world.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
