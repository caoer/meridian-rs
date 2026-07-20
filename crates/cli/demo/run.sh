#!/usr/bin/env bash
# mrd demo harness: exercise every subcommand against a fresh S0 fixture
# vault and save the RAW results (argv, stdout, stderr, exit code) as a
# machine-readable manifest.
#
# Usage: crates/cli/demo/run.sh [out-dir]     (default: target/mrd-demo)
#
# Output:
#   <out-dir>/results.json   raw results manifest (array of case objects)
#   <out-dir>/results.js     the same data as window.MRD_RESULTS (so
#                            report.html works from file:// without a server)
#   <out-dir>/report.html    copy of the Alpine report page beside its data
#   <out-dir>/raw/NN-slug.{out,err,code}  per-case raw streams
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
out="${1:-$repo/target/mrd-demo}"
raw="$out/raw"
rm -rf "$raw"          # idempotent: stale cases from a prior run never linger
mkdir -p "$raw"

cargo build -p mrd-cli --quiet
mrd="$repo/target/debug/mrd"

# --- the §0.3 S0 fixture vault, byte-exact -------------------------------
vault="$(mktemp -d)"
mkdir -p "$vault/notes" "$vault/receipts"
printf -- '---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n' > "$vault/notes/plan.md"
printf -- '# Receipts — 2026-07-18\n' > "$vault/receipts/2026-07-18.md"

R0="b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"

n=0
manifest="$out/results.json"
printf '[\n' > "$manifest"

# run <slug> <title> <expected-exit> <mrd args...>
run() {
  local slug="$1" title="$2" want="$3"; shift 3
  n=$((n + 1))
  local id
  id=$(printf '%02d-%s' "$n" "$slug")
  local code=0
  "$mrd" --root "$vault" "$@" > "$raw/$id.out" 2> "$raw/$id.err" || code=$?
  printf '%s' "$code" > "$raw/$id.code"
  if [ "$code" -ne "$want" ]; then
    echo "FAIL $id: exit $code, wanted $want" >&2
    exit 1
  fi
  [ "$n" -gt 1 ] && printf ',\n' >> "$manifest"
  # one JSON case object, streams JSON-encoded via python (portable quoting)
  python3 - "$id" "$slug" "$title" "$code" "$want" "$raw/$id.out" "$raw/$id.err" "mrd $*" >> "$manifest" <<'PY'
import json, sys
id_, slug, title, code, want, outf, errf, argv = sys.argv[1:9]
print(json.dumps({
    "id": id_, "slug": slug, "title": title,
    "argv": argv,
    "exit": int(code), "expected_exit": int(want),
    "stdout": open(outf, encoding="utf-8", errors="replace").read(),
    "stderr": open(errf, encoding="utf-8", errors="replace").read(),
}, ensure_ascii=False, indent=2), end="")
PY
}

# --- every subcommand: reads first (vault at S0/R0), then the write path --
run hello        "hello — handshake, complete caps"                    0 hello
run toc          "toc — the map with revs riding free"                 0 toc notes/plan.md
run toc-json     "toc --json — the raw wire frame"                     0 --json toc notes/plan.md
run cat-file     "cat — whole file roundtrip"                          0 cat notes/plan.md
run cat-sec      "cat --sec — one section, full span bytes"            0 cat notes/plan.md --sec Goals --sec Q3
run extract      "extract — full node inventory"                       0 extract notes/plan.md
run extract-kind "extract --kinds wikilink"                            0 extract notes/plan.md --kinds wikilink
run resolve      "resolve — the walk plane"                            0 resolve notes/plan.md 'plan#Goals#Q3'
run resolve-dang "resolve — dangling ref fails loud (stage 1)"         1 resolve notes/plan.md roadmap
run links        "links — resolved + unresolved edges"                 0 links notes/plan.md
run links-corpus "links — whole-corpus edge map"                       0 links
run root         "root — the frozen ambient root R0"                   0 root
run diff         "diff — replay between identical roots (at R0)"       0 --json diff "$R0" "$R0"
run sub          "sub — one-shot ack at seq 0 (honest limit)"          0 sub
run sub-beyond   "sub — beyond the epoch refuses root_unknown"         1 sub 5
run err-missing  "cat — file_not_found, recovery env, exit 1"          1 cat missing.md
run splice-dry   "splice --dry — armed facts, disk untouched"          0 splice notes/plan.md --dry --edits '[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by September"}}}]'
run splice       "splice — the §4.4 worked write, receipt included"    0 splice notes/plan.md --actor agent:demo --now 2026-07-18T20:31:04Z --receipt-path receipts/2026-07-18.md --receipt-anchor r-000042 --edits '[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by September"}},"if_node_rev":"33d5b0e1b27cb48b"}]'
run cat-after    "cat — a second process sees the committed write"     0 cat notes/plan.md --sec Goals --sec Q3
run splice-cas   "splice — stale rev refuses cas_mismatch"             1 splice notes/plan.md --edits '[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by September","new":"x"}},"if_node_rev":"33d5b0e1b27cb48b"}]'

printf '\n]\n' >> "$manifest"

# validate + emit the file://-friendly JS twin
python3 - "$manifest" "$out/results.js" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
with open(sys.argv[2], "w", encoding="utf-8") as f:
    f.write("window.MRD_RESULTS = ")
    json.dump(data, f, ensure_ascii=False)
    f.write(";\n")
print(f"{len(data)} cases captured")
PY

cp "$(dirname "$0")/report.html" "$out/report.html"
rm -rf "$vault"
echo "demo results: $out (open report.html)"
