#!/bin/sh
# df1a64cc — gates for card repair-cannot-emit-a-json-refusal-envelope, links seam.
# Run from the worktree root. Every rc is read OUT OF THE LOG (broadcast 0039).
set -u
WT=/Users/Shared/scratch/df1a64cc-links-b84ccf91
LOG=$WT/gates.log
cd "$WT" || exit 99
: > "$LOG"

say() { echo "=== $* ===" | tee -a "$LOG"; }

# ---- ENGINE SELECTORS ----
# RECORD, THEN CLEAR. Not `env -u`, and not a bare refusal either.
#
# all-hands 0067 retracted `env -u` because an unset run is clean whether the variable was there
# or not, so it cannot tell you it just saved you. A bare refusal fixes that — but it also makes
# this arm UNRUNNABLE in a seat where the variable is set by the spawn path, and an unrunnable
# gate is what pushes a seat back to `env -u`. So: READ the variable, WRITE ITS VALUE TO THE LOG
# as evidence of exposure, then clear it for the child commands. The receipt survives; the act is
# safe. MRD_SELECTOR_STRICT=1 restores the pure refusal for a seat that expects a clean
# environment and wants to be told it is wrong.
say "SELECTOR RECORD-AND-CLEAR (all-hands 0066/0067)"
EXPOSED=0
for v in CCC_MRD_BIN MERIDIAN_MRD_BIN MERIDIAN_DAEMON_BIN MRD_BIN MRD_BIN_DIR MRD_PIN; do
  eval "val=\${$v:-}"
  if [ -n "$val" ]; then
    EXPOSED=$((EXPOSED + 1))
    echo "  EXPOSED: $v = '$val'" | tee -a "$LOG"
    if [ "${MRD_SELECTOR_STRICT:-0}" = "1" ]; then
      echo "REFUSED (strict): $v is SET — this arm must run with it empty" | tee -a "$LOG"
      exit 101
    fi
    unset "$v"
    eval "after=\${$v:-}"
    [ -z "$after" ] || { echo "REFUSED: could not clear $v" | tee -a "$LOG"; exit 101; }
    echo "    cleared for this arm" | tee -a "$LOG"
  else
    echo "  $v empty: ok" | tee -a "$LOG"
  fi
done
echo "selectors exposed at entry: $EXPOSED" | tee -a "$LOG"

# ---- ADDRESS ASSERTION: git -C walks UP, so prove this is the repo I meant ----
say "TOPLEVEL ASSERTION (door 445fc704's rider)"
TOP=$(git -C "$WT" rev-parse --show-toplevel 2>&1)
echo "toplevel: $TOP" | tee -a "$LOG"
[ "$TOP" = "$WT" ] || { echo "REFUSED: toplevel is not my worktree" | tee -a "$LOG"; exit 102; }
echo "base commit: $(git -C "$WT" rev-parse HEAD)" | tee -a "$LOG"
echo "base tree:   $(git -C "$WT" rev-parse 'HEAD^{tree}')" | tee -a "$LOG"

if [ "${MRD_GATES_DRYRUN:-0}" = "1" ]; then
  echo "DRYRUN: preflight only, no compilation (needs no slot)" | tee -a "$LOG"
  exit 0
fi

# ---- BUILD ----
say "cargo build -p mrd"
cargo build -p mrd >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

say "RUNG 0 — identity, requiring the paren field to CLOSE"
./target/debug/mrd --version 2>&1 | tee -a "$LOG"

# ---- FIXED WORLD ----
say "FIXED: cargo test -p mrd --test json_refusal_family"
cargo test -p mrd --test json_refusal_family >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

say "FIXED: cargo test -p mrd --test u22_lost_pin_repair"
cargo test -p mrd --test u22_lost_pin_repair >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

say "FIXED: cargo clippy -p mrd --all-targets"
cargo clippy -p mrd --all-targets >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

say "FIXED: cargo fmt -p mrd -- --check"
cargo fmt -p mrd -- --check >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

echo "GATES DONE" | tee -a "$LOG"
