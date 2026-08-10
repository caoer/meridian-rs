#!/bin/sh
# df1a64cc — THE NEGATIVE CONTROL: same tree, ONE STATEMENT as the only variable.
#
# The links seam's frame emission is swapped for a no-op that keeps both params used, so the
# build stays clean and the ONLY difference is whether the envelope is emitted. A control that
# changes more than one thing cannot tell you which one the test is measuring.
#
# ⚠️ cmp is run against the fixed file, because a "corrupting" edit whose pattern matched nothing
# is the vacuous-control failure this card already ate once (55e4442c, caught by cmp).
set -u
WT=/Users/Shared/scratch/df1a64cc-links-b84ccf91
LOG=$WT/control.log
cd "$WT" || exit 99
: > "$LOG"
say() { echo "=== $* ===" | tee -a "$LOG"; }

SRC=crates/mrd/src/engine.rs
cp "$SRC" /tmp/df1a64cc-engine-fixed.rs

say "SWAP: frame emission -> no-op"
/usr/bin/sed -i '' \
  's|^        json_error_frame(format, workspace, \&e);$|        let _ = (format, workspace);|' \
  "$SRC"

say "CONTROL IS DIFFERENT FROM SUBJECT (cmp must report a difference)"
if cmp -s /tmp/df1a64cc-engine-fixed.rs "$SRC"; then
  echo "REFUSED: control is BYTE-IDENTICAL to the subject — the sed matched nothing." | tee -a "$LOG"
  cp /tmp/df1a64cc-engine-fixed.rs "$SRC"
  exit 103
fi
echo "cmp: files differ (good)" | tee -a "$LOG"
diff /tmp/df1a64cc-engine-fixed.rs "$SRC" | tee -a "$LOG"

say "UNFIXED: cargo test -p mrd --test json_refusal_family"
cargo test -p mrd --test json_refusal_family >>"$LOG" 2>&1; echo "rc=$?" | tee -a "$LOG"

say "RESTORE"
cp /tmp/df1a64cc-engine-fixed.rs "$SRC"
cmp -s /tmp/df1a64cc-engine-fixed.rs "$SRC" && echo "restored: byte-identical" | tee -a "$LOG"
echo "CONTROL DONE" | tee -a "$LOG"
