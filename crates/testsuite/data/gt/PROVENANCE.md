# GT pack provenance

Copied verbatim from the frozen parser-bench fixture pack:
`home-wiki-codespace/parser-bench/fixtures/` (frozen 2026-07-17T23:48:52Z by
lane0-corpus, agent 282a39ec; manifest sha256
`a4fe7a4040de32384fce3be2d14e28ff65e57806c00a97e58d3138400aaaeae5`).

Contents: the 10 ground-truth `*.expected.json` + `ground-truth/README.md`
(the GT law — span convention, kind vocabulary) + the 10 source markdown files
each expected.json's `file` field names, at their pack-relative paths
(8 `corpus/…`, 2 `adversarial/…`).

The full corpus (206 files) and the adversarial pack (13 files incl. the 10 MB
monster) stay in parser-bench — they are bench/sweep fixtures, not ground truth.

Frozen means frozen: do not add, edit, or re-sample. Disagreements between a
parser and this ground truth are findings for lane 0 (agent 5bb58f0e gates
corrections), never local edits.
