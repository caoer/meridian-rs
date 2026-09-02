#!/usr/bin/env python3
"""parser-bench lane0: reference markdown extractor (ground-truth generator).

Byte-span extraction of the CONTRACT checklist constructs from raw bytes.
Span convention (uniform): [start, end) byte offsets into the raw file;
block spans run from the first byte of the construct to the last byte of
its final line EXCLUDING the line terminator. Inline spans include their
delimiters (backticks, [[ ]], the leading ! of an embed, the ^ of an anchor).

Extraction rules (documented decisions — ground-truth law for this bench):
- frontmatter: only when bytes 0..3 are exactly b"---\\n" (BOM files: none in
  the ground-truth set; a BOM-prefixed "---" is NOT frontmatter here).
- fences: CommonMark-ish. Opener: <=3 spaces indent, run of >=3 backticks or
  tildes; backtick info strings may not contain backticks. Closer: same char,
  run >= opener length, <=3 indent, nothing else on the line. Unterminated
  fence runs to EOF. 4+ space indent is never a fence.
- headings: ATX only (# .. ######, <=3 indent, space after hashes). Setext
  headings are NOT extracted (documented limitation; rare in agent-written
  corpus). hpath = "/".join of heading texts root..self.
- anchors: Obsidian block ids — ^[A-Za-z0-9-]+ at line tail only (whitespace
  before ^ or line start; only trailing whitespace after). Unicode ids are
  NOT valid anchors. Not inside fences/frontmatter/inline code.
- wikilinks/embeds: [[...]] / ![[...]], no newline inside, not in code.
- callouts: top-level quote block whose FIRST line matches > [!type] with a
  CLOSED bracket ("[!x" unclosed -> plain quote, not extracted; type may be
  any non-space token; ">[!x]" without space after > counts; a "-" or "+"
  fold marker after "]" allowed). Block = contiguous lines starting with
  <=3 spaces then ">". Nested callouts (quote-within-quote) not extracted.
- tasks: list item line ^\\s*[-*+] \\[[ xX]\\] ; span from list marker byte.
- tables: >=2 consecutive lines containing "|" where line 2 is a GFM
  delimiter row; span = whole contiguous pipe-block.
- comments: %%...%% (non-greedy, may span lines), not inside fences.
- inline code: `code` / ``code`` on one line, not inside fences.
"""
import re

FENCE_OPEN = re.compile(rb"^( {0,3})(`{3,}|~{3,})[ \t]*([^\n]*)$")
ATX = re.compile(rb"^ {0,3}(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$")
ANCHOR = re.compile(rb"(?:(?<=\s)|^)(\^[A-Za-z0-9-]+)[ \t\r]*$")
WIKILINK = re.compile(rb"(!?)\[\[([^\[\]\n]+)\]\]")
CALLOUT_HEAD = re.compile(rb"^ {0,3}>[ \t]?\[!([^\s\]]+)\][+-]?")
QUOTE_LINE = re.compile(rb"^ {0,3}>")
TASK = re.compile(rb"^[ \t]*([-*+] \[[ xX]\])[ \t]")
TABLE_DELIM = re.compile(rb"^ {0,3}\|?[ \t:|-]*-[ \t:|-]*\|?[ \t\r]*$")
INLINE_CODE = re.compile(rb"(?<!`)(`{1,2})(?!`)(.+?)(?<!`)\1(?!`)")
COMMENT = re.compile(rb"%%(.+?)%%", re.S)


def _lines(raw):
    """[(start, end_excl_newline, end_incl_newline)] per line."""
    out, pos = [], 0
    n = len(raw)
    while pos < n:
        nl = raw.find(b"\n", pos)
        if nl == -1:
            out.append((pos, n, n))
            break
        end = nl
        if end > pos and raw[end - 1:end] == b"\r":
            end -= 1
        out.append((pos, end, nl + 1))
        pos = nl + 1
    return out


def extract(raw):
    """Return ordered node list: dicts {kind, hpath?, span:[s,e]}."""
    nodes = []
    lines = _lines(raw)
    nlines = len(lines)

    # --- frontmatter ---
    fm_end_line = -1
    if raw.startswith(b"---\n") or raw.startswith(b"---\r\n"):
        for i in range(1, nlines):
            s, e, _ = lines[i]
            if raw[s:e] in (b"---", b"..."):
                fm_end_line = i
                nodes.append({"kind": "frontmatter", "span": [0, e]})
                break

    # --- line scan: fences, headings, callouts, tasks, tables ---
    in_fence = False
    fence_char = b""
    fence_len = 0
    fence_start = None
    fenced_line = [False] * nlines  # True = line is inside (or part of) a fence
    hstack = []  # (level, text)
    i = fm_end_line + 1
    while i < nlines:
        s, e, _ = lines[i]
        line = raw[s:e]
        if in_fence:
            fenced_line[i] = True
            m = FENCE_OPEN.match(line)
            if (m and m.group(2)[0:1] == fence_char and len(m.group(2)) >= fence_len
                    and not m.group(3).strip()):
                nodes.append({"kind": "fence", "span": [fence_start, e]})
                in_fence = False
            i += 1
            continue
        m = FENCE_OPEN.match(line)
        if m and not (m.group(2)[0:1] == b"`" and b"`" in m.group(3)):
            in_fence = True
            fence_char = m.group(2)[0:1]
            fence_len = len(m.group(2))
            fence_start = s + len(m.group(1))
            fenced_line[i] = True
            i += 1
            continue
        m = ATX.match(line)
        if m:
            level = len(m.group(1))
            text = m.group(2).decode("utf-8", "backslashreplace")
            while hstack and hstack[-1][0] >= level:
                hstack.pop()
            hstack.append((level, text))
            start = s + len(line) - len(line.lstrip(b" "))
            nodes.append({"kind": "heading",
                          "hpath": "/".join(t for _, t in hstack),
                          "span": [start, e]})
            i += 1
            continue
        if CALLOUT_HEAD.match(line):
            j = i
            while j + 1 < nlines and QUOTE_LINE.match(
                    raw[lines[j + 1][0]:lines[j + 1][1]]):
                j += 1
            start = s + len(line) - len(line.lstrip(b" "))
            nodes.append({"kind": "callout", "span": [start, lines[j][1]]})
            i = j + 1
            continue
        m = TASK.match(line)
        if m:
            start = s + line.index(m.group(1))
            nodes.append({"kind": "task", "span": [start, e]})
            i += 1
            continue
        if b"|" in line and line.strip().startswith(b"|") and i + 1 < nlines:
            nxt = raw[lines[i + 1][0]:lines[i + 1][1]]
            if b"|" in nxt and TABLE_DELIM.match(nxt):
                j = i + 1
                while j + 1 < nlines:
                    cand = raw[lines[j + 1][0]:lines[j + 1][1]]
                    if b"|" in cand and cand.strip().startswith(b"|"):
                        j += 1
                    else:
                        break
                start = s + len(line) - len(line.lstrip(b" "))
                nodes.append({"kind": "table", "span": [start, lines[j][1]]})
                i = j + 1
                continue
        i += 1

    if in_fence:  # unterminated: runs to EOF
        nodes.append({"kind": "fence", "span": [fence_start, len(raw)],
                      "unterminated": True})

    # --- inline pass (skip fenced lines + frontmatter) ---
    fence_spans = [n["span"] for n in nodes if n["kind"] == "fence"]
    fm_spans = [n["span"] for n in nodes if n["kind"] == "frontmatter"]

    def in_spans(pos, spans):
        return any(a <= pos < b for a, b in spans)

    icode_spans = []
    for m in COMMENT.finditer(raw):
        if not in_spans(m.start(), fence_spans) and not in_spans(m.start(), fm_spans):
            nodes.append({"kind": "comment", "span": [m.start(), m.end()]})
    for i in range(fm_end_line + 1, nlines):
        if fenced_line[i]:
            continue
        s, e, _ = lines[i]
        line = raw[s:e]
        for m in INLINE_CODE.finditer(line):
            icode_spans.append((s + m.start(), s + m.end()))
            nodes.append({"kind": "inline-code",
                          "span": [s + m.start(), s + m.end()]})
        for m in WIKILINK.finditer(line):
            if in_spans(s + m.start(), icode_spans):
                continue
            kind = "embed" if m.group(1) else "wikilink"
            nodes.append({"kind": kind, "span": [s + m.start(), s + m.end()]})
        m = ANCHOR.search(line)
        if m and not in_spans(s + m.start(1), icode_spans):
            nodes.append({"kind": "anchor",
                          "span": [s + m.start(1), s + m.end(1)]})

    nodes.sort(key=lambda n: (n["span"][0], n["span"][1]))
    for n in nodes:
        a, b = n["span"]
        n["text_prefix_16b"] = raw[a:a + 16].decode("utf-8", "backslashreplace")
    return nodes
