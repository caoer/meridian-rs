#!/usr/bin/env bash
# r15 ALARM DEMONSTRATION for the absence family's FACE axis.
#
#   tools/absence-family-alarm.sh
#
# ⛔ RE-RUN AFTER ANY EDIT TO `edge_color` OR TO
# `crates/mrd/tests/absence_domain_family_e2e.rs`. Exit 0 = demonstrated.
#
# WHY IT EXISTS. Charter 02 r15: a clean read from a check is not evidence the
# check works — vary one condition, make the subject genuinely fail, and watch
# the detector say so. The four face tests assert that `check`, `walk`, `status`
# and the commit gate all name a MEASURED ABSENCE (session decisions 0049/0054).
# Passing tests do not prove those assertions can REFUSE.
#
# WHY IT IS A FILE AND NOT A TRANSCRIPT. A demonstration is a PROPERTY, not an
# event, and a detector edited after its demonstration is UNDEMONSTRATED AGAIN.
# The seat who most needs this is whoever edits `edge_color` months from now,
# who cannot re-run a demonstration that lived in someone's session.
#
# WHAT IT PROVES — both halves, because the second is what keeps a guard alive:
#   1. ALL FOUR faces REFUSE, each named independently, on a broken engine
#   2. all four STAY QUIET on a correct one
#
# ⛔ THE MUTATION ASSERTS ITS OWN TARGET IS PRESENT EXACTLY ONCE BEFORE MUTATING.
# A mutation that silently matched nothing would make this control void.
#
# NOTE ON `set -e`: deliberately NOT used, unlike this repo's other tools. This
# script REQUIRES a non-zero cargo run in its middle — under `set -e` it would
# abort at the very step it exists to observe.

set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
GREP=/usr/bin/grep

fail() { echo "⛔ DEMONSTRATION INVALID: $*"; exit 2; }

# `git -C` walks UP and answers confidently about a parent, so the root is
# ASSERTED rather than assumed: this file must live inside the tree it mutates.
WT="$(git -C "$here" rev-parse --show-toplevel)" || fail "not inside a git worktree"
[ -f "$WT/crates/view/src/walk.rs" ] || fail "toplevel $WT does not look like meridian-rs"
cd "$WT" || fail "cannot enter $WT"

FILE=crates/view/src/walk.rs
TEST=absence_domain_family_e2e
LOG_DIR="$(mktemp -d)"   # cargo logs go outside the tree; see BUILD OUTPUT below

# ⛔ BUILD OUTPUT. This script COMPILES, so it writes a cargo target dir — the
# first run here cost 318 crates and 7.1 GB. It honours CARGO_TARGET_DIR when the
# caller sets one, so a seat with a warm private arena reuses it instead of
# paying a cold build; unset, cargo falls back to the repo's own `target/`.
# ⚠️ THE FIRST VERSION OF THIS SCRIPT CLAIMED "leaves no residue" AND THAT WAS
# FALSE — it silently created a 7.1 GB `target/` in the worktree. `git status`
# read CLEAN throughout, because `target/` is gitignored: THE NATURAL CHECK FOR
# RESIDUE CANNOT SEE GITIGNORED RESIDUE. The claim is corrected rather than
# scrubbed, because the next person to write "no residue" will verify it the
# same blind way.
echo "target   : ${CARGO_TARGET_DIR:-$WT/target (cargo default)}"

TARGET='if corpus.on_ambient_disk(edge.to_root.as_ref(), &edge.to_path) == Some(false) {'
MUTANT='if corpus.in_hash_domain(edge.to_root.as_ref(), &edge.to_path) == Some(false) && corpus.on_ambient_disk(edge.to_root.as_ref(), &edge.to_path) == Some(false) {'

sha="$(git rev-parse HEAD)"
echo "worktree : $WT"
echo "HEAD     : ${sha}"
echo "logs     : $LOG_DIR"

# Engine selectors: RECORD by value. A check that cannot report is not a check.
exposed=0
for v in CCC_MRD_BIN MERIDIAN_MRD_BIN MRD_BIN MERIDIAN_DAEMON_BIN; do
  val="$(printenv "$v" || true)"
  if [ -n "$val" ]; then echo "EXPOSED  : $v = $val"; exposed=$((exposed + 1)); fi
done
echo "selectors exposed at entry: $exposed (of 4 probed)"

dirty="$(git status --porcelain -- "$FILE" | wc -l | tr -d ' ')"
[ "$dirty" = "0" ] || fail "$FILE is already modified; refusing to mutate a dirty file"

# ── POSITIVE CONTROL ON THE MUTATION ITSELF ─────────────────────────────────
hits="$($GREP -cF "$TARGET" "$FILE")"
[ "$hits" = "1" ] || fail "mutation target matched $hits times, expected exactly 1 — a mutation that matches nothing (or twice) proves nothing"
echo "mutation target present exactly once ✅"

restore() {
  git checkout -- "$FILE" 2>/dev/null
  if cmp -s <(git show "HEAD:$FILE") "$FILE"; then
    echo "restored byte-identical to HEAD ✅"
  else
    echo "⛔ RESTORE FAILED — $FILE differs from HEAD. FIX BY HAND."
    exit 3
  fi
}
trap restore EXIT

# ── VARY ONE CONDITION: nest the existence read back inside the domain branch,
#    which is exactly the pre-0054 arm order decision 0054 overturned. ────────
python3 - "$FILE" "$TARGET" "$MUTANT" <<'PY'
import sys
path, target, mutant = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
assert s.count(target) == 1, "target count moved between check and edit"
open(path, 'w').write(s.replace(target, mutant))
PY
cmp -s <(git show "HEAD:$FILE") "$FILE" && fail "mutation did not change the file"
echo "mutation applied (pre-0054 arm order restored) ✅"

# ── PROPERTY 1: every face must REFUSE, and each must NAME ITSELF ───────────
# ⛔ ONE TEST PER FACE IS LOAD-BEARING. An earlier cut asserted four faces inside
# ONE test; under mutation it refused at its FIRST assertion and the other three
# never executed — an aggregate masking a member (charter 02 r9) inside the very
# demonstration meant to expose one. Asserting "the test refused" would
# reintroduce it, so each face is required BY NAME.
cargo test -p mrd --test "$TEST" > "$LOG_DIR/mutated.log" 2>&1
rc=$?
echo "mutated run rc=$rc"
[ "$rc" != "0" ] || fail "THE FACE AXIS STAYED GREEN ON A BROKEN ENGINE — it is a line that prints, not a detector"

missing=""
for f in face_check_json_names_the_measured_absence \
         face_walk_names_the_measured_absence \
         face_status_rolls_the_absence_up_under_its_own_word \
         face_commit_gate_refuses_over_the_measured_absence; do
  if $GREP -q "^test $f \.\.\. FAILED" "$LOG_DIR/mutated.log"; then
    echo "    REFUSED: $f"
  else
    missing="$missing $f"
  fi
done
[ -z "$missing" ] || fail "these faces did NOT refuse on a broken engine —$missing"
echo "PROPERTY 1 ✅ ALL FOUR FACES refused independently, each naming itself"

# ── PROPERTY 2: quiet on correct input ──────────────────────────────────────
restore
trap - EXIT
cargo test -p mrd --test "$TEST" > "$LOG_DIR/restored.log" 2>&1
rc2=$?
echo "restored run rc=$rc2"
[ "$rc2" = "0" ] || fail "the face axis does NOT pass on correct input — it refuses everything, which is equally useless"
echo "PROPERTY 2 ✅ face axis GREEN on correct input"

# ${sha} BRACED: zsh applies a history modifier to an unbraced expansion followed
# by `:<letter>` — 17 of 52 letters fire. Unbraced, the trap would corrupt the
# commit id inside the receipt that attests this run: a wrong RECORD, not a wrong
# command, with no failing exit to notice it.
echo
echo "════ DEMONSTRATED at ${sha}: the face axis fires on a broken engine and stays quiet on a correct one."
