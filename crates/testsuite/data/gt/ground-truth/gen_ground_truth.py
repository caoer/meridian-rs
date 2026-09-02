#!/usr/bin/env python3
"""Generate ground-truth/*.expected.json for the 10 pack files.

Runs mdref.extract (the reference extractor — the GT law in code), enforces the
splice law (span slice must reproduce node prefix), writes one JSON per file.
Output structure per README:
  {"file": <fixture-relpath>, "nodes": [{kind, hpath?, span:[s,e), text_prefix_16b}]}

Run from anywhere: `python3 crates/testsuite/data/gt/ground-truth/gen_ground_truth.py`.
The expected JSON is never hand-edited; a sample change is followed by a regeneration.
"""
import json
import os
import stat
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mdref

GT = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.dirname(GT)

FILES = [
    "samples/sessions/year=2026/month=04/02-03-skill-refresh/agents/b2c3d4e5.md",
    "samples/sessions/year=2026/month=03/20-02-compound-sweep/tasks/task-07-2026-03-01-adhoc.md",
    "samples/sessions/year=2026/month=04/08-01-format-tuning/rosters/0001-worker-formatter.md",
    "samples/sessions/year=2026/month=04/15-02-lint-fix/decisions/orchestrator--bulk-commit-bypass.md",
    "samples/sessions/year=2026/month=03/14-01-adhoc/SESSION.md",
    "samples/wiki/domains/net/network/relay-hub.md",
    "samples/wiki/effects/skills/agent-sdk-notes.md",
    "samples/wiki/health/tags/domain/rover.md",
    "adversarial/anchor-edge-cases.md",
    "adversarial/callout-vs-quote.md",
]


def main():
    os.makedirs(GT, exist_ok=True)
    for rel in FILES:
        raw = open(os.path.join(FIX, rel), "rb").read()
        nodes = mdref.extract(raw)
        # splice law self-check
        for n in nodes:
            a, b = n["span"]
            assert 0 <= a < b <= len(raw), (rel, n)
            assert raw[a:a + 16].decode("utf-8", "backslashreplace") == \
                n["text_prefix_16b"], (rel, n)
        name = os.path.basename(rel).replace(".md", ".expected.json")
        out = os.path.join(GT, name)
        if os.path.exists(out):
            os.chmod(out, 0o644)
        with open(out, "w") as f:
            json.dump({"file": rel, "nodes": nodes}, f, indent=1, ensure_ascii=False)
            f.write("\n")
        os.chmod(out, stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH)
        kinds = {}
        for n in nodes:
            kinds[n["kind"]] = kinds.get(n["kind"], 0) + 1
        print(f"{name}: {len(nodes)} nodes {kinds}")


if __name__ == "__main__":
    main()
