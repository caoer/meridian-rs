#!/usr/bin/env python3
"""Comment-only diff gate for the comment-cleanup sweep.

Answers one question, mechanically: do two versions of a Rust file differ
ONLY inside comments? Whitespace outside literals is ignored; string /
char / raw-string bodies must match byte-for-byte.

Usage:
    invariance.py <before.rs> <after.rs>    # exit 0 = comment-only diff
    invariance.py selftest                  # guard cases

Exit codes: 0 pass, 1 code changed, 2 usage/read error.

The literal-skipping rules mirror sweep.py's lexer; selftest keeps the
mirror honest. Note one deliberate strictness: deleting a comment that
separates two tokens (`foo/*x*/bar`) FAILS, because the token stream
really does change.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

RAW_OPEN = re.compile(r'(?:b?r)(#*)"')
CHAR_LIT = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'])'")


def residue(src: str) -> str:
    """Source minus comments: code whitespace-collapsed, literal bodies verbatim."""
    out: list[str] = []
    code: list[str] = []
    i, n = 0, len(src)

    def flush_code() -> None:
        if not code:
            return
        s = re.sub(r"\s+", " ", "".join(code)).strip()
        out.append(s + " " if s else " ")
        code.clear()

    while i < n:
        c = src[i]

        m = RAW_OPEN.match(src, i)
        if m and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            close = '"' + m.group(1)
            j = src.find(close, m.end())
            j = n if j == -1 else j + len(close)
            flush_code()
            out.append("\x00" + src[i:j] + "\x00")
            i = j
            continue

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
            flush_code()
            out.append("\x00" + src[i:j] + "\x00")
            i = j
            continue

        if c == "'":
            m = CHAR_LIT.match(src, i)
            if m:
                flush_code()
                out.append("\x00" + src[i : m.end()] + "\x00")
                i = m.end()
                continue
            code.append(c)  # a lifetime tick
            i += 1
            continue

        if src.startswith("//", i):
            end = src.find("\n", i)
            i = n if end == -1 else end  # newline survives as code whitespace
            continue

        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            code.append(" ")  # a block comment is a token separator
            i = j
            continue

        code.append(c)
        i += 1

    flush_code()
    return "".join(out)


def compare(before: str, after: str) -> tuple[bool, str]:
    a, b = residue(before), residue(after)
    if a == b:
        return True, ""
    k = next((idx for idx, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    lo = max(0, k - 40)
    show = lambda s: s[lo : k + 40].replace("\x00", "|").replace("\n", "\\n")
    return False, f"first divergence at residue offset {k}:\n  before: …{show(a)}…\n  after:  …{show(b)}…"


SELF_CASES = [
    # (name, before, after, expect_pass)
    ("delete line comment", "let x = 1; // why\nfoo();\n", "let x = 1;\nfoo();\n", True),
    ("rewrite doc comment", "/// long essay\nfn f() {}\n", "/// short\nfn f() {}\n", True),
    ("delete nested block", "a(); /* x /* y */ z */ b();\n", "a(); b();\n", True),
    ("reflow blank lines", "a();\n\n\n// gone\n\nb();\n", "a();\nb();\n", True),
    ("code edit", "let x = 1;\n", "let x = 2;\n", False),
    ("string edit", 'let u = "http://a"; // c\n', 'let u = "http://b";\n', False),
    ("whitespace inside string", 'let s = "a b";\n', 'let s = "a  b";\n', False),
    ("raw string with // kept", 'let r = r#"// not a comment"#;\n', 'let r = r#"// not a comment"#;\n', True),
    ("raw string edited", 'let r = r#"// not a comment"#;\n', 'let r = r#"// edited"#;\n', False),
    ("slashes in string not comment", 'let u = "http://x"; // trailing\n', 'let u = "http://x";\n', True),
    ("lifetime survives", "fn f<'a>(x: &'a str) {} // c\n", "fn f<'a>(x: &'a str) {}\n", True),
    ("char literal quote", "let c = '\"'; // c\n", "let c = '\"';\n", True),
    ("token-separating comment deleted", "foo/*x*/bar\n", "foobar\n", False),
    ("commented-out code deleted", "a();\n// b();\n// c();\nd();\n", "a();\nd();\n", True),
]


def selftest() -> int:
    bad = 0
    for name, before, after, expect in SELF_CASES:
        ok, why = compare(before, after)
        if ok != expect:
            bad += 1
            print(f"FAIL {name}: expected {'pass' if expect else 'fail'}, got {'pass' if ok else 'fail'}\n{why}")
    print(f"selftest: {len(SELF_CASES) - bad}/{len(SELF_CASES)} ok")
    return 1 if bad else 0


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "selftest":
        return selftest()
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    try:
        before = Path(sys.argv[1]).read_text(encoding="utf-8")
        after = Path(sys.argv[2]).read_text(encoding="utf-8")
    except OSError as e:
        print(f"read error: {e}")
        return 2
    ok, why = compare(before, after)
    if ok:
        print("PASS: comment-only diff")
        return 0
    print(f"FAIL: code changed\n{why}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
