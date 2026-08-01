#!/usr/bin/env python3
"""Extract every source comment in the meridian-rs workspace as a dated,
rev-pinned, machine-readable inventory.

The audit question this serves: which comments report a FINDING (a fact about
the product's state) rather than explain the code under the cursor. This script
does not answer that — it produces the exhaustive corpus and a high-recall
candidate filter over it, so a reader re-runs the extraction rather than
trusting a hand-built list.

Comments are extracted by a small Rust lexer, not by grep: `//` inside a string
literal is not a comment, and `/* */` nests in Rust. Contiguous comment lines
collapse into one block, because a finding is written as a paragraph.

Usage (see --help):
    sweep.py blocks      # every comment block, JSONL
    sweep.py candidates  # blocks matching the finding-language lexicon, JSONL
    sweep.py counts      # per-crate totals, JSON
    sweep.py residue     # deterministic random sample of NON-candidates, JSONL
    sweep.py triggers    # print the lexicon itself
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------
# Rust lexing: enough state to know whether a `//` starts a comment.
# --------------------------------------------------------------------------


@dataclass
class RawComment:
    line: int  # 1-indexed
    kind: str  # outer-doc | inner-doc | line | block-doc | block
    text: str  # comment body, marker stripped, right-trimmed
    col: int  # 0-indexed column the marker starts at
    own_line: bool  # True when only whitespace precedes the marker


def _classify_line_marker(src: str, i: int) -> tuple[str, int]:
    """Return (kind, body_start) for a `//`-family marker at index i."""
    # Order matters: `///` and `//!` before `//`; `////` is a plain comment
    # (rustdoc treats 4+ slashes as non-doc).
    if src.startswith("////", i):
        return "line", i + 4
    if src.startswith("///", i):
        return "outer-doc", i + 3
    if src.startswith("//!", i):
        return "inner-doc", i + 3
    return "line", i + 2


def _classify_block_marker(src: str, i: int) -> tuple[str, int]:
    if src.startswith("/**", i) and not src.startswith("/**/", i):
        return "block-doc", i + 3
    if src.startswith("/*!", i):
        return "block-doc", i + 3
    return "block", i + 2


def lex_comments(src: str) -> list[RawComment]:
    """Extract comments from Rust source, skipping string/char/raw-string bodies."""
    out: list[RawComment] = []
    i, n = 0, len(src)
    line = 1
    line_start = 0  # index of the first char of the current line

    def at_own_line(marker_idx: int) -> bool:
        return src[line_start:marker_idx].strip() == ""

    while i < n:
        c = src[i]

        if c == "\n":
            line += 1
            i += 1
            line_start = i
            continue

        # --- raw string: r"..." / r#"..."# / br#"..."# -----------------------
        m = re.compile(r'(?:b?r)(#*)"').match(src, i)
        if m and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            hashes = m.group(1)
            close = '"' + hashes
            j = src.find(close, m.end())
            j = n if j == -1 else j + len(close)
            line += src.count("\n", i, j)
            # recompute line_start to the last newline before j
            nl = src.rfind("\n", i, j)
            if nl != -1:
                line_start = nl + 1
            i = j
            continue

        # --- normal string: "..." with escapes -------------------------------
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            line += src.count("\n", i, j)
            nl = src.rfind("\n", i, j)
            if nl != -1:
                line_start = nl + 1
            i = j
            continue

        # --- char literal: only the strict forms, so lifetimes stay untouched -
        if c == "'":
            m = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'])'").match(src, i)
            if m:
                i = m.end()
                continue
            i += 1  # a lifetime tick; nothing to skip
            continue

        # --- line comment ----------------------------------------------------
        if src.startswith("//", i):
            kind, body_start = _classify_line_marker(src, i)
            end = src.find("\n", i)
            end = n if end == -1 else end
            out.append(
                RawComment(
                    line=line,
                    kind=kind,
                    text=src[body_start:end].rstrip(),
                    col=i - line_start,
                    own_line=at_own_line(i),
                )
            )
            i = end
            continue

        # --- block comment (Rust nests them) ---------------------------------
        if src.startswith("/*", i):
            kind, body_start = _classify_block_marker(src, i)
            own = at_own_line(i)
            col = i - line_start
            start_line = line
            depth, j = 1, body_start
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            body = src[body_start : max(body_start, j - 2)]
            out.append(
                RawComment(
                    line=start_line,
                    kind=kind,
                    text=body.strip(),
                    col=col,
                    own_line=own,
                )
            )
            line += src.count("\n", i, j)
            nl = src.rfind("\n", i, j)
            if nl != -1:
                line_start = nl + 1
            i = j
            continue

        i += 1

    return out


# --------------------------------------------------------------------------
# Blocks: a finding is a paragraph, so contiguous comment lines group.
# --------------------------------------------------------------------------

DOC_KINDS = {"outer-doc", "inner-doc", "block-doc"}


@dataclass
class Block:
    block_id: str
    crate: str
    path: str  # repo-relative, valid at `rev`
    line_start: int
    line_end: int
    kind: str  # dominant kind of the run
    own_line: bool
    in_test: bool
    lines: int
    words: int
    text: str
    text_sha256: str
    triggers: list[str] = field(default_factory=list)
    introduced: dict | None = None      # oldest blame row in the block's range
    last_touched: dict | None = None    # newest blame row in the block's range
    commits: list[str] = field(default_factory=list)


def _test_line_ranges(src: str) -> list[tuple[int, int]]:
    """Line ranges (1-indexed, inclusive) covered by `#[cfg(test)]` items.

    Heuristic by design: brace-counted from the attribute to the item's close.
    Reported per-block as `in_test` so a reader can weight test comments
    differently; it is not used to exclude anything.
    """
    ranges: list[tuple[int, int]] = []
    for m in re.finditer(r"#\[cfg\(test\)\]", src):
        brace = src.find("{", m.end())
        if brace == -1:
            continue
        depth, j, n = 1, brace + 1, len(src)
        while j < n and depth:
            ch = src[j]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            j += 1
        ranges.append((src.count("\n", 0, m.start()) + 1, src.count("\n", 0, j) + 1))
    return ranges


def blocks_for_file(repo_rel: str, crate: str, src: str) -> list[Block]:
    comments = lex_comments(src)
    tests = _test_line_ranges(src)

    def in_test(ln: int) -> bool:
        return any(a <= ln <= b for a, b in tests)

    out: list[Block] = []
    run: list[RawComment] = []

    def flush() -> None:
        if not run:
            return
        start, end = run[0].line, run[-1].line
        text = "\n".join(c.text for c in run).strip()
        kinds = [c.kind for c in run]
        kind = "doc" if any(k in DOC_KINDS for k in kinds) else "line"
        if len(run) == 1 and run[0].kind.startswith("block"):
            kind = run[0].kind
        digest = hashlib.sha256(text.encode()).hexdigest()
        out.append(
            Block(
                block_id=f"{repo_rel}:{start}:{digest[:12]}",
                crate=crate,
                path=repo_rel,
                line_start=start,
                line_end=end,
                kind=kind,
                own_line=run[0].own_line,
                in_test=in_test(start),
                lines=len(run),
                words=len(text.split()),
                text=text,
                text_sha256=digest,
            )
        )
        run.clear()

    for c in comments:
        if not run:
            run.append(c)
            continue
        prev = run[-1]
        contiguous = c.line == prev.line + 1
        same_family = (prev.kind in DOC_KINDS) == (c.kind in DOC_KINDS)
        # A trailing comment on a code line never joins an own-line paragraph.
        same_placement = prev.own_line == c.own_line
        if contiguous and same_family and same_placement and not prev.kind.startswith("block"):
            run.append(c)
        else:
            flush()
            run.append(c)
    flush()
    return out


# --------------------------------------------------------------------------
# The finding-language lexicon. High recall on purpose: a worker discards a
# false positive in seconds, while a false negative never reaches the inventory.
# --------------------------------------------------------------------------

TRIGGERS: dict[str, list[str]] = {
    # A whole plane / arm / path is claimed dead, missing, or unused.
    "reachability": [
        r"\bunreachable\b", r"\bnever (?:fires|runs|called|reached|happens|hit)\b",
        r"\bno (?:caller|consumer|user|reader|writer)s?\b", r"\bnothing (?:calls|uses|reads|writes)\b",
        r"\bdead (?:code|arm|branch|path)\b", r"\bcannot be (?:reached|hit|constructed)\b",
        r"\bis silent by construction\b", r"\bonly (?:call|use)r\b",
    ],
    # Claims about what is or is not built.
    "not-built": [
        r"\bnot (?:yet )?(?:built|implemented|wired|used|exposed|supported|possible|done|available)\b",
        r"\bdoes ?n[o']t exist\b", r"\bno (?:such|impl(?:ementation)?)\b", r"\bunimplemented\b",
        r"\bstub(?:bed)?\b", r"\bplaceholder\b", r"\bnot (?:a )?thing\b", r"\bmissing\b",
    ],
    # Time-indexed assertions — true when written, unverified now.
    "time-indexed": [
        r"\btoday\b", r"\bcurrently\b", r"\bat present\b", r"\bfor now\b", r"\bas of\b",
        r"\bright now\b", r"\bat the moment\b", r"\bso far\b", r"\bstill\b", r"\byet\b",
        r"\balready\b", r"\bno longer\b", r"\bused to\b", r"\bpreviously\b", r"\bformerly\b",
        r"\bsince the\b", r"\bwhen .{0,30}\blands\b", r"\bonce .{0,30}\blands\b",
    ],
    # Deferred / planned work — a roadmap statement, not a code explanation.
    "deferred": [
        r"\bTODO\b", r"\bFIXME\b", r"\bXXX\b", r"\bHACK\b", r"\bdeferred?\b", r"\bpostponed\b",
        r"\bplanned\b", r"\bwill be\b", r"\bpending\b", r"\bfuture\b", r"\bnext (?:leg|wave|pass|step)\b",
        r"\beventually\b", r"\blater\b", r"\bfollow-?up\b", r"\bshould (?:be|become|move)\b",
    ],
    # Cites a conversation the reader was not in.
    "session-dependent": [
        r"\bwe (?:decided|agreed|discussed|chose|ruled|settled|said|found|learned)\b",
        r"\bas (?:discussed|agreed|decided|noted above|established)\b",
        r"\bper (?:the )?(?:discussion|conversation|advisor|ruling|user|ZT|decision)\b",
        r"\bZT\b", r"\bthe user\b", r"\bthe advisor\b", r"\bratified\b", r"\bthe ruling\b",
        r"\bamend(?:ed|ment)\b", r"\bsuperseded?\b", r"\bearlier (?:discussion|session|pass)\b",
        r"\bdogfood\b", r"\bthe card\b", r"\bthe memo\b", r"\bsession\b",
    ],
    # Migration / parity narration — status of a project, not of this function.
    "migration": [
        r"\bmeridian-go\b", r"\bport(?:ed|ing)?\b", r"\bparity\b", r"\bmigrat(?:e|ed|ion|ing)\b",
        r"\bcutover\b", r"\bleg\b", r"\bend-?state\b", r"\bdies\b", r"\blegacy\b", r"\bobsolete\b",
    ],
    # Explicit finding markers.
    "flagged": [
        r"\bNOTE:", r"\bIMPORTANT:", r"\bWARNING:", r"\bCAVEAT:", r"\bBUG\b", r"\bbroken\b",
        r"\bwrong\b", r"\bincorrect\b", r"\bfinding\b", r"\bknown issue\b", r"\bworkaround\b",
        r"\bsurpris(?:e|ing)\b", r"\bgotcha\b",
    ],
}

_COMPILED = {name: [re.compile(p, re.IGNORECASE) for p in pats] for name, pats in TRIGGERS.items()}


def match_triggers(text: str) -> list[str]:
    hits: list[str] = []
    for name, pats in _COMPILED.items():
        for p in pats:
            if p.search(text):
                hits.append(name)
                break
    return hits


# --------------------------------------------------------------------------
# Walking the workspace
# --------------------------------------------------------------------------

SKIP_DIRS = {"target", ".git", "result", "node_modules", ".direnv"}


def rust_files(root: Path) -> list[Path]:
    out: list[Path] = []
    for base, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        for f in sorted(files):
            if f.endswith(".rs"):
                out.append(Path(base) / f)
    return out


def crate_of(repo_rel: str) -> str:
    parts = repo_rel.split("/")
    return parts[1] if len(parts) > 2 and parts[0] == "crates" else "(root)"


def git_rev(root: Path) -> str:
    try:
        return subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except Exception:
        return "(unknown)"


def blame_file(root: Path, repo_rel: str) -> dict[int, dict]:
    """line-number -> {commit, date, author, summary} for one file at HEAD."""
    try:
        raw = subprocess.run(
            ["git", "-C", str(root), "blame", "--line-porcelain", "HEAD", "--", repo_rel],
            capture_output=True, text=True, check=True,
        ).stdout
    except subprocess.CalledProcessError:
        return {}
    out: dict[int, dict] = {}
    cur: dict = {}
    lineno = 0
    for ln in raw.splitlines():
        m = re.match(r"^([0-9a-f]{40}) \d+ (\d+)", ln)
        if m:
            cur = {"commit": m.group(1)}
            lineno = int(m.group(2))
        elif ln.startswith("author "):
            cur["author"] = ln[7:]
        elif ln.startswith("author-time "):
            cur["ts"] = int(ln[12:])
        elif ln.startswith("summary "):
            cur["summary"] = ln[8:]
        elif ln.startswith("\t"):
            import datetime as _dt

            cur["date"] = (
                _dt.datetime.fromtimestamp(cur.get("ts", 0), _dt.UTC).strftime("%Y-%m-%d")
                if cur.get("ts")
                else "?"
            )
            out[lineno] = {k: cur.get(k) for k in ("commit", "author", "date", "summary")}
    return out


def enrich_blame(root: Path, blocks: list[Block]) -> None:
    """Attach introduced/last_touched to each block, one blame per file."""
    by_file: dict[str, list[Block]] = {}
    for b in blocks:
        by_file.setdefault(b.path, []).append(b)
    for path, bs in by_file.items():
        bl = blame_file(root, path)
        if not bl:
            continue
        for b in bs:
            rows = [bl[n] for n in range(b.line_start, b.line_end + 1) if n in bl]
            if not rows:
                continue
            rows_sorted = sorted(rows, key=lambda r: r["date"] or "")
            b.introduced = rows_sorted[0]
            b.last_touched = rows_sorted[-1]
            b.commits = sorted({r["commit"][:8] for r in rows})


def collect(root: Path) -> list[Block]:
    all_blocks: list[Block] = []
    for f in rust_files(root):
        rel = str(f.relative_to(root))
        try:
            src = f.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for b in blocks_for_file(rel, crate_of(rel), src):
            b.triggers = match_triggers(b.text)
            all_blocks.append(b)
    return all_blocks


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Comment inventory for the meridian-rs workspace.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Every emission carries the rev it ran at. Pin a rev by pointing\n"
            "--root at a worktree checked out there:\n"
            "  git worktree add --detach /tmp/pin <rev>\n"
            "  tools/comment-audit/sweep.py candidates --root /tmp/pin > candidates.jsonl\n"
        ),
    )
    ap.add_argument(
        "mode",
        choices=["blocks", "candidates", "counts", "residue", "triggers", "selftest", "blame"],
    )
    ap.add_argument("--root", default=".", help="workspace root (default: cwd)")
    ap.add_argument("--crate", action="append", help="restrict to crate(s); repeatable")
    ap.add_argument("--min-words", type=int, default=0, help="drop blocks shorter than N words")
    ap.add_argument("--sample", type=int, default=100, help="residue: sample size")
    ap.add_argument("--seed", type=int, default=20260801, help="residue: RNG seed (deterministic)")
    args = ap.parse_args()

    if args.mode == "triggers":
        json.dump(TRIGGERS, sys.stdout, indent=2)
        print()
        return 0

    root = Path(args.root).resolve()
    if args.mode == "selftest":
        return selftest()

    rev = git_rev(root)
    blocks = collect(root)
    if args.crate:
        want = set(args.crate)
        blocks = [b for b in blocks if b.crate in want]
    if args.min_words:
        blocks = [b for b in blocks if b.words >= args.min_words]

    header = {
        "_meta": {
            "rev": rev,
            "root": str(root),
            "mode": args.mode,
            "total_blocks": len(blocks),
            "candidates": sum(1 for b in blocks if b.triggers),
            "tool": "tools/comment-audit/sweep.py",
        }
    }

    if args.mode == "counts":
        per: dict[str, dict] = {}
        for b in blocks:
            d = per.setdefault(
                b.crate,
                {"blocks": 0, "comment_lines": 0, "candidates": 0, "doc": 0, "line": 0, "in_test": 0},
            )
            d["blocks"] += 1
            d["comment_lines"] += b.lines
            d["candidates"] += 1 if b.triggers else 0
            d["doc"] += 1 if b.kind == "doc" else 0
            d["line"] += 1 if b.kind == "line" else 0
            d["in_test"] += 1 if b.in_test else 0
        trig: dict[str, int] = {}
        for b in blocks:
            for t in b.triggers:
                trig[t] = trig.get(t, 0) + 1
        json.dump({**header, "per_crate": per, "per_trigger": trig}, sys.stdout, indent=2)
        print()
        return 0

    if args.mode in ("candidates", "blame"):
        sel = [b for b in blocks if b.triggers]
        if args.mode == "blame":
            enrich_blame(root, sel)
    elif args.mode == "residue":
        pool = [b for b in blocks if not b.triggers]
        rng = random.Random(args.seed)
        sel = rng.sample(pool, min(args.sample, len(pool)))
        sel.sort(key=lambda b: (b.path, b.line_start))
        header["_meta"]["residue_pool"] = len(pool)
        header["_meta"]["seed"] = args.seed
    else:
        sel = blocks

    header["_meta"]["emitted"] = len(sel)
    print(json.dumps(header["_meta"]))
    for b in sel:
        print(json.dumps(asdict(b)))
    return 0


def selftest() -> int:
    """Lexer guards: the cases grep gets wrong."""
    cases = [
        ('let u = "http://x/y";', 0, "url in string is not a comment"),
        ('let s = "// not a comment";', 0, "slashes in string"),
        ("// real\nlet x = 1;", 1, "plain line comment"),
        ('let r = r#"// raw "#;', 0, "raw string"),
        ("/// doc\n/// more\nfn f() {}", 1, "doc run collapses to one block"),
        ("/* a /* nested */ still */ let x = 1;", 1, "nested block comment"),
        ("let c = '/'; // after char\n", 1, "char literal then comment"),
        ("fn f<'a>(x: &'a str) {} // lifetime\n", 1, "lifetimes do not open strings"),
        ("let x = 1; // trailing\n// own line\n", 2, "trailing and own-line do not merge"),
    ]
    failures = 0
    for src, want, label in cases:
        got = len(blocks_for_file("t.rs", "t", src))
        ok = got == want
        failures += 0 if ok else 1
        print(f"{'ok  ' if ok else 'FAIL'} {label}: want {want} block(s), got {got}")
    print(f"\n{len(cases) - failures}/{len(cases)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
