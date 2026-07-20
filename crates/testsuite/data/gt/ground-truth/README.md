# ground-truth law

10 files: 8 corpus (each `*.expected.json`'s `file` field names its fixture
under `corpus/`) + 2 adversarial (`anchor-edge-cases.md`, `callout-vs-quote.md`).
Structure: `{"file": <fixture-relpath>, "nodes": [{kind, hpath?, span:[start,end), text_prefix_16b}]}`.

## Span convention (uniform, bytes)

- Offsets are **byte** offsets into the raw file, `[start, end)`.
- Block nodes (frontmatter, heading, fence, callout, task, table, comment):
  first byte of the construct → last byte of its final line **excluding** the
  line terminator.
- Inline nodes include their delimiters: backticks, `[[ ]]`, the `!` of an
  embed, the `^` of an anchor.
- `text_prefix_16b` = `raw[start:start+16].decode("utf-8", "backslashreplace")`.

## Kind vocabulary + decisions

| kind | rule |
|---|---|
| frontmatter | only when bytes 0..3 are `---\n` (BOM-prefixed `---` is NOT frontmatter) |
| heading | ATX only, ≤3 indent; setext NOT extracted (documented limitation, rare in corpus); `hpath` = `/`-joined heading texts root→self, text as written (incl. backticks/anchors) |
| fence | CommonMark-ish; closer run ≥ opener, same char; unterminated → EOF + `"unterminated": true`; 4+ space indent is never a fence; backtick info string may not contain backticks |
| inline-code | `` ` ``/`` `` `` on one line, not in fences/frontmatter |
| anchor | Obsidian block id `^[A-Za-z0-9-]+` at **line tail** only; mid-line and unicode ids are NOT anchors; not in fences/inline code |
| wikilink / embed | `[[...]]` / `![[...]]`, no newline inside, not in fences/frontmatter/inline code |
| callout | top-level quote block whose first line is `> [!type]` with **closed** bracket; `>[!x]` (no space) counts; fold marker `-`/`+` allowed; unclosed `[!x` → plain quote (not extracted); nested (quote-in-quote) not extracted; plain blockquotes are not a kind |
| task | `- [ ]` / `- [x]` / `- [X]` list line (also `*`/`+` markers); span from marker byte to line end |
| table | ≥2 consecutive `\|`-leading lines where line 2 is a GFM delimiter row; span = whole contiguous pipe block |
| comment | `%%...%%` non-greedy, may span lines; none appear in the 10 files (near-absent in corpus — see survey) |

## Overlap is intended

Inline nodes are also extracted inside block nodes (inline code in headings,
wikilinks in callout bodies, anchors on heading/task lines). Nodes are ordered
by span start; overlapping spans are correct, not a bug.

Disagreements are findings: flag to 5bb58f0e — lane 0 gates corrections; do
not silently adjust your parser or this ground truth.
