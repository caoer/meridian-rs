export const meta = {
  name: 'comment-cleanup',
  description: 'Minimize comments across all meridian-rs .rs files, batched by estimated effort. Per batch: grok-exec rewrites comments only (no code change, no build, no commit, no worktree), then codex-exec reviews that one batch file-by-file. Key information and review verdicts land in docs/comment-cleanup/ mirror .md files. NO-GO is recorded, never iterated.',
  phases: [
    { title: 'Rewrite', detail: 'grok CLI, one agent per batch, comments only, per-file mechanical invariance self-check' },
    { title: 'Review', detail: 'codex CLI, independent per-file invariance run + info-preservation + style, verdict appended to each mirror md' },
  ],
}

// Contract (fixed before launch):
// - Scope: the BATCHES array below — all 330 tracked .rs files (36,315 comment
//   lines), grouped by crate and bin-packed to ~600 comment lines / max 10
//   files per batch. n = approximate comment-line count. Trim batches to
//   narrow scope.
// - Only allowed md location: docs/comment-cleanup/<rust path>.md (mirror tree,
//   declared here up front so no post-run move is needed).
// - Code-invariance gate: tools/comment-audit/invariance.py (selftested), run
//   per file by the rewriter (self-check) and again by the reviewer.
// - A NO-GO verdict is written into that file's mirror md and collected; the
//   workflow never loops back to the rewriter.

const REPO = '/Users/Shared/repos/meridian-rs'
const MD_ROOT = 'docs/comment-cleanup'
const mdRel = rel => MD_ROOT + '/' + rel.replace(/\.rs$/, '.md')

const FILE_RESULT = {
  type: 'object',
  additionalProperties: false,
  required: ['rel', 'changed', 'mdCreated', 'gate', 'diffstat'],
  properties: {
    rel: { type: 'string' },
    changed: { type: 'boolean' },
    mdCreated: { type: 'boolean' },
    gate: { type: 'string', description: 'verbatim PASS/FAIL line from the invariance run; empty if changed=false' },
    diffstat: { type: 'string', description: "verbatim `git diff --stat HEAD -- <file>` line, run by the WRAPPER itself; 'empty' when no diff" },
  },
}

const REWRITE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['status', 'files', 'summary'],
  properties: {
    status: { type: 'string', enum: ['ok', 'partial', 'failed'], description: 'partial = some files of the batch done' },
    reason: { type: 'string', description: 'on partial/failed: transient | rate-limit | config | task, plus one line' },
    sessionId: { type: 'string', description: 'CLI session id, for manual resume' },
    files: { type: 'array', items: FILE_RESULT, description: 'one entry per batch file you processed' },
    summary: { type: 'string', description: '2-5 sentences for the whole batch: what was deleted, simplified, moved' },
  },
}

const FILE_VERDICT = {
  type: 'object',
  additionalProperties: false,
  required: ['rel', 'verdict', 'detail', 'diffstat'],
  properties: {
    rel: { type: 'string' },
    verdict: { type: 'string', enum: ['GO', 'NO-GO', 'UNREVIEWED'], description: 'UNREVIEWED only when you could not judge this file' },
    gate: { type: 'string', description: 'verbatim PASS/FAIL line from your independent invariance run' },
    detail: { type: 'string', description: 'condensed verdict, 1-2 sentences' },
    diffstat: { type: 'string', description: "verbatim `git diff --stat HEAD -- <file>` line, run by the WRAPPER itself; 'empty' when no diff" },
  },
}

const REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['status', 'files'],
  properties: {
    status: { type: 'string', enum: ['ok', 'partial', 'failed'] },
    reason: { type: 'string', description: 'on partial/failed: transient | rate-limit | config | task, plus one line' },
    sessionId: { type: 'string', description: 'CLI session id, for manual resume' },
    files: { type: 'array', items: FILE_VERDICT, description: 'one entry per batch file — every file, even untouched ones' },
  },
}

const fileList = b => b.files.map(f => '  - ' + f.rel + '   (~' + f.n + ' comment lines)   notes: ' + mdRel(f.rel)).join('\n')

const rewritePrompt = b => `<grok-exec:dispatch>
model: grok-4-5 (fall back to the CLI default if this id is unknown)
max execution time: 40 minutes
workspace: /tmp/wf-comment-cleanup/rw-${b.idx} — mkdir -p it and keep ALL your scratch state (prompt file, session id, out, pid) inside it. Re-derive every path from this literal on each step. NEVER store state in the session scratchpad or any shared file, and NEVER read another workspace under /tmp/wf-comment-cleanup/ or /tmp/grok-exec.*: sibling wrappers run concurrently and pointer files get clobbered.
verify before returning (pilot lesson — resumed CLI sessions fabricate completion): the CLI's claims are unverified narrative. From /Users/Shared/repos/meridian-rs run git diff --stat HEAD -- <file> YOURSELF for every batch file and fill each diffstat field from your own run. If the CLI claimed changed=true but your diffstat is empty, set that file changed=false, set overall status partial, and say so in reason (task: CLI claimed edits that never landed). If you relaunch or resume the CLI, re-verify from disk — never accept a resumed session's summary as fact.
</grok-exec:dispatch>
<grok-exec:payload>
You are one agent in a fleet minimizing comments across a Rust repo. You own exactly ONE batch of files; work through them one at a time.

Repo: ${REPO}   (cd there first; all paths below are repo-relative)
Your batch (crate ${b.crate}, ~${b.total} comment lines total):
${fileList(b)}

WRITE ACCESS — exactly the .rs files listed above plus their listed notes files. Other agents work on other batches concurrently; never touch anything else. mkdir -p a notes file's parent directory before creating it.

HARD RULES
- Comments only. Every non-comment token must survive byte-identical: no code, string, attribute, or macro changes. Never delete a comment that separates two tokens without leaving a space.
- Edit each repo file IN PLACE with your file-edit tool. Never build a script pipeline that writes a cleaned copy somewhere else, and never edit copies under /tmp — edits that are not in the working tree do not exist.
- No cargo, rustc, build, test, fmt, or clippy runs. No worktree. Git is read-only for you: never commit, stash, checkout, branch, or add.
- After editing each file, prove code-invariance mechanically before moving on:
    B=$(mktemp -d)/before.rs
    git show HEAD:<file> > "$B"
    python3 tools/comment-audit/invariance.py "$B" <file>
  It must print PASS. If it prints FAIL, repair your edit until it passes. If you cannot, run git restore <file> (the one allowed git write, scoped to your own batch) and record that file as changed=false with the FAIL line in its gate field.

STYLE TARGET — comments stay minimal; the llm-wiki outside this repo holds the comprehensive docs.
- Delete: comments narrating what the code does, restating names or types, changelog-style remarks ("added for...", "used by..."), commented-out code, decorative banner rules.
- Keep, simplifying when wordy: SAFETY comments on unsafe blocks (must remain), TODO/FIXME markers, license headers, and any WHY a maintainer needs at the cursor — hidden constraints, subtle invariants, workaround-for-bug notes.
- Doc comments (/// and //!): keep a one-line summary plus only load-bearing detail.
- KEY INFORMATION RULE: when a comment paragraph carries real substance too big for one line — design rationale, protocol or contract detail, invariant chains, findings about system state — move that substance into THAT FILE's notes file BEFORE shrinking the comment. Append a section:
    ## <topic> (from <file>:<line>)
    <the preserved substance, lightly edited>
  Then leave at most one short line in code. On first use create the notes file starting with the line: # <file> — comment-cleanup notes
  Never mix two source files in one notes file. Do not write references to notes files inside code comments.
- If a file's comments are already minimal, leave it untouched and record changed=false.

RETURN a single JSON object per the schema you were given: status (ok | partial | failed), reason, sessionId, files (one entry per file: rel, changed, mdCreated, gate), summary.
</grok-exec:payload>`

const reviewPrompt = (b, rw) => `<codex-exec:dispatch>
model: gpt-5-6-sol, reasoning effort: medium (fall back to the CLI default model at medium reasoning if this id is unknown)
max execution time: 30 minutes
sandbox: launch codex with a workspace-write sandbox rooted at /Users/Shared/repos/meridian-rs (e.g. --sandbox workspace-write with the repo as cwd) so it can append verdict sections under docs/comment-cleanup/. Read-only mode broke verdict writes in the pilot.
workspace: /tmp/wf-comment-cleanup/rv-${b.idx} — mkdir -p it and keep ALL your scratch state inside it. Re-derive every path from this literal on each step. NEVER store state in the session scratchpad or any shared file, and NEVER read another workspace under /tmp/wf-comment-cleanup/ or /tmp/codex-exec.*: sibling wrappers run concurrently and pointer files get clobbered.
verify before returning (pilot lesson — a reviewer CLI rubber-stamped files that were never edited): from /Users/Shared/repos/meridian-rs run git diff --stat HEAD -- <file> YOURSELF for every batch file and fill each diffstat field from your own run; a verdict whose story contradicts your diffstat is fabricated — replace it with NO-GO detail 'reviewer claim contradicts disk'. Then test -f each notes md the verdict says it wrote; if the CLI could not write a verdict section (sandbox), append the returned section to the md YOURSELF with your Write tool before returning.
</codex-exec:dispatch>
<codex-exec:payload>
You review ONE other agent's comment-cleanup batch in a Rust repo, file by file. Review only: you fix nothing, revert nothing, and never iterate with the other agent.

Repo: ${REPO}   (cd there first; paths below are repo-relative)
Batch under review (crate ${b.crate}):
${fileList(b)}
Rewriter's report: ${JSON.stringify(rw)}

The ONLY paths you may write are the listed notes files. Other agents are editing OTHER batches concurrently: ignore every path outside this batch. The rewrite of this batch is finished; judge each working-tree file against git show HEAD:<file>.

FOR EACH FILE in the batch, in order:
0. Disk truth FIRST: git diff --stat HEAD -- <file>. If empty while the rewriter claimed changed=true: NO-GO immediately, detail "no edits on disk", skip the other checks for this file. If empty and the rewriter said changed=false: verify and GO with detail "no changes".
1. Code-invariance (mechanical):
    B=$(mktemp -d)/before.rs
    git show HEAD:<file> > "$B"
    python3 tools/comment-audit/invariance.py "$B" <file>
   FAIL here is an automatic NO-GO for that file regardless of everything else.
2. Information preservation: read git diff HEAD -- <file>. For every deleted or shortened paragraph that carried non-obvious substance — invariants, contracts, SAFETY reasoning, workarounds, findings about system state — verify the substance survives in the remaining comment or in that file's notes file. cat the notes file before describing it; a notes file the rewriter claimed but which does not exist is an info-preservation fail. Every unsafe block must still carry a SAFETY comment.
3. Style: remaining comments are minimal — no narration of what code does, no commented-out code, doc comments short — and reworded comments are still accurate (a simplified comment must not say something false about the code).
VERDICT per file: GO or NO-GO. Append to that file's notes file (mkdir -p the parent; if missing, create it starting with the line: # <file> — comment-cleanup notes ; never edit existing sections):

## Review verdict: <GO|NO-GO>
- reviewer: codex-exec
- code-invariance: pass|fail — <gate line verbatim>
- info-preservation: pass|fail — <one line>
- style: pass|fail — <one line>
- detail: <specifics; for NO-GO name exactly what is wrong and where>

RETURN a single JSON object per the schema you were given: status (ok | partial | failed), reason, sessionId, files (one entry per batch file: rel, verdict GO | NO-GO | UNREVIEWED, gate, detail).
</codex-exec:payload>`

// All 330 tracked .rs files: grouped by crate, bin-packed to ~600 comment
// lines / max 10 files per batch, heaviest first within each crate.
const BATCHES = [
  { crate: 'wire-serve', total: 1358, files: [
    { n: 1358, rel: 'crates/wire-serve/src/write.rs' },
  ] },
  { crate: 'wire-serve', total: 589, files: [
    { n: 309, rel: 'crates/wire-serve/tests/u12_door_enumeration.rs' },
    { n: 280, rel: 'crates/wire-serve/src/positions.rs' },
  ] },
  { crate: 'wire-serve', total: 454, files: [
    { n: 265, rel: 'crates/wire-serve/src/read.rs' },
    { n: 189, rel: 'crates/wire-serve/src/gate.rs' },
  ] },
  { crate: 'wire-serve', total: 574, files: [
    { n: 154, rel: 'crates/wire-serve/src/plan.rs' },
    { n: 151, rel: 'crates/wire-serve/tests/s7_pin.rs' },
    { n: 139, rel: 'crates/wire-serve/src/decode.rs' },
    { n: 130, rel: 'crates/wire-serve/src/rev.rs' },
  ] },
  { crate: 'wire-serve', total: 580, files: [
    { n: 122, rel: 'crates/wire-serve/src/reaction.rs' },
    { n: 115, rel: 'crates/wire-serve/tests/s10_fp_decorate.rs' },
    { n: 108, rel: 'crates/wire-serve/src/lib.rs' },
    { n: 82, rel: 'crates/wire-serve/tests/s2fix_rule_id_journal.rs' },
    { n: 77, rel: 'crates/wire-serve/tests/u5_4_substrate.rs' },
    { n: 76, rel: 'crates/wire-serve/tests/s2fix_promotion.rs' },
  ] },
  { crate: 'wire-serve', total: 521, files: [
    { n: 71, rel: 'crates/wire-serve/tests/s2fix_empty_span_mint.rs' },
    { n: 70, rel: 'crates/wire-serve/tests/s2fix_fp_document_grain.rs' },
    { n: 67, rel: 'crates/wire-serve/tests/s2fix_artifact_guard.rs' },
    { n: 59, rel: 'crates/wire-serve/src/check_write.rs' },
    { n: 50, rel: 'crates/wire-serve/tests/s4a_conformance_toctou.rs' },
    { n: 49, rel: 'crates/wire-serve/tests/u8b_plan_edits.rs' },
    { n: 42, rel: 'crates/wire-serve/src/armed_disk.rs' },
    { n: 41, rel: 'crates/wire-serve/tests/u6_write_flock.rs' },
    { n: 37, rel: 'crates/wire-serve/tests/u11_cross_root_confinement.rs' },
    { n: 35, rel: 'crates/wire-serve/tests/u11_lock_write.rs' },
  ] },
  { crate: 'wire-serve', total: 59, files: [
    { n: 31, rel: 'crates/wire-serve/tests/u5_write_conflict.rs' },
    { n: 28, rel: 'crates/wire-serve/tests/reserved_paths.rs' },
  ] },
  { crate: 'model', total: 985, files: [
    { n: 985, rel: 'crates/model/src/lib.rs' },
  ] },
  { crate: 'model', total: 392, files: [
    { n: 392, rel: 'crates/model/src/selector.rs' },
  ] },
  { crate: 'model', total: 597, files: [
    { n: 223, rel: 'crates/model/src/fingerprint.rs' },
    { n: 123, rel: 'crates/model/src/delta.rs' },
    { n: 97, rel: 'crates/model/tests/s2fix_empty_span.rs' },
    { n: 89, rel: 'crates/model/src/walk.rs' },
    { n: 65, rel: 'crates/model/tests/norm_v2_fixtures.rs' },
  ] },
  { crate: 'model', total: 156, files: [
    { n: 48, rel: 'crates/model/tests/u11r2_two_root_greys.rs' },
    { n: 47, rel: 'crates/model/src/gotext.rs' },
    { n: 36, rel: 'crates/model/tests/s2fix_root_qualified_linkpath.rs' },
    { n: 25, rel: 'crates/model/tests/u11_c3_linkpath_peels_and_refuses.rs' },
  ] },
  { crate: 'wire', total: 874, files: [
    { n: 874, rel: 'crates/wire/src/lib.rs' },
  ] },
  { crate: 'wire', total: 157, files: [
    { n: 139, rel: 'crates/wire/tests/contract_v2.rs' },
    { n: 18, rel: 'crates/wire/tests/create_shape.rs' },
  ] },
  { crate: 'fs', total: 603, files: [
    { n: 603, rel: 'crates/fs/src/lib.rs' },
  ] },
  { crate: 'fs', total: 597, files: [
    { n: 273, rel: 'crates/fs/src/domain.rs' },
    { n: 215, rel: 'crates/fs/src/guard.rs' },
    { n: 109, rel: 'crates/fs/tests/exec_guard.rs' },
  ] },
  { crate: 'fs', total: 169, files: [
    { n: 86, rel: 'crates/fs/tests/write_lock_release.rs' },
    { n: 50, rel: 'crates/fs/tests/hash_domain.rs' },
    { n: 33, rel: 'crates/fs/tests/overlay_interval.rs' },
  ] },
  { crate: 'check', total: 601, files: [
    { n: 601, rel: 'crates/check/src/layer0.rs' },
  ] },
  { crate: 'check', total: 472, files: [
    { n: 149, rel: 'crates/check/src/lib.rs' },
    { n: 126, rel: 'crates/check/src/layer1.rs' },
    { n: 93, rel: 'crates/check/tests/end_to_end.rs' },
    { n: 75, rel: 'crates/check/src/orphan.rs' },
    { n: 29, rel: 'crates/check/tests/orphan_production_fixture.rs' },
  ] },
  { crate: 'mrd', total: 566, files: [
    { n: 566, rel: 'crates/mrd/src/check_cmd.rs' },
  ] },
  { crate: 'mrd', total: 455, files: [
    { n: 455, rel: 'crates/mrd/src/status_cmd.rs' },
  ] },
  { crate: 'mrd', total: 402, files: [
    { n: 402, rel: 'crates/mrd/src/corpus_tier.rs' },
  ] },
  { crate: 'mrd', total: 356, files: [
    { n: 356, rel: 'crates/mrd/tests/s4r21_fence_line.rs' },
  ] },
  { crate: 'mrd', total: 351, files: [
    { n: 351, rel: 'crates/mrd/tests/s2fix_cross_surface.rs' },
  ] },
  { crate: 'mrd', total: 594, files: [
    { n: 334, rel: 'crates/mrd/tests/u15_hook_fence_e2e.rs' },
    { n: 260, rel: 'crates/mrd/src/hook.rs' },
  ] },
  { crate: 'mrd', total: 447, files: [
    { n: 232, rel: 'crates/mrd/tests/s3_cross_surface.rs' },
    { n: 215, rel: 'crates/mrd/src/sql.rs' },
  ] },
  { crate: 'mrd', total: 418, files: [
    { n: 210, rel: 'crates/mrd/src/history_cmd.rs' },
    { n: 208, rel: 'crates/mrd/tests/rules_cli.rs' },
  ] },
  { crate: 'mrd', total: 546, files: [
    { n: 183, rel: 'crates/mrd/tests/u14_check_pin_plane.rs' },
    { n: 183, rel: 'crates/mrd/tests/s4r19_commit_gate.rs' },
    { n: 180, rel: 'crates/mrd/tests/check_e2e.rs' },
  ] },
  { crate: 'mrd', total: 497, files: [
    { n: 169, rel: 'crates/mrd/src/rules_cmd.rs' },
    { n: 166, rel: 'crates/mrd/tests/config_e2e.rs' },
    { n: 162, rel: 'crates/mrd/src/config_cmd.rs' },
  ] },
  { crate: 'mrd', total: 540, files: [
    { n: 145, rel: 'crates/mrd/tests/skill_hook_emit.rs' },
    { n: 145, rel: 'crates/mrd/tests/hook_plane_fence.rs' },
    { n: 127, rel: 'crates/mrd/tests/e2e.rs' },
    { n: 123, rel: 'crates/mrd/src/run_cmd.rs' },
  ] },
  { crate: 'mrd', total: 570, files: [
    { n: 118, rel: 'crates/mrd/tests/floors.rs' },
    { n: 118, rel: 'crates/mrd/src/walk_cmd.rs' },
    { n: 113, rel: 'crates/mrd/tests/u32_governed_journal.rs' },
    { n: 112, rel: 'crates/mrd/tests/u35_journaled_doors.rs' },
    { n: 109, rel: 'crates/mrd/tests/status_e2e.rs' },
  ] },
  { crate: 'mrd', total: 583, files: [
    { n: 104, rel: 'crates/mrd/tests/u13_per_root_anchoring.rs' },
    { n: 103, rel: 'crates/mrd/tests/corpus_suite.rs' },
    { n: 97, rel: 'crates/mrd/tests/color_planes_e2e.rs' },
    { n: 96, rel: 'crates/mrd/tests/f4_declared_unbound_root.rs' },
    { n: 92, rel: 'crates/mrd/src/engine.rs' },
    { n: 91, rel: 'crates/mrd/src/rules_walk.rs' },
  ] },
  { crate: 'mrd', total: 593, files: [
    { n: 89, rel: 'crates/mrd/src/journal_cmd.rs' },
    { n: 88, rel: 'crates/mrd/tests/history.rs' },
    { n: 88, rel: 'crates/mrd/tests/f6_check_sees_the_mount_table.rs' },
    { n: 86, rel: 'crates/mrd/tests/u11r2_declared_but_unreadable.rs' },
    { n: 83, rel: 'crates/mrd/tests/u12_stored_form_e2e.rs' },
    { n: 80, rel: 'crates/mrd/src/realise_cmd.rs' },
    { n: 79, rel: 'crates/mrd/tests/u11_cross_root_walk_e2e.rs' },
  ] },
  { crate: 'mrd', total: 558, files: [
    { n: 75, rel: 'crates/mrd/src/init.rs' },
    { n: 74, rel: 'crates/mrd/src/skill_cmd.rs' },
    { n: 68, rel: 'crates/mrd/tests/sql_e2e.rs' },
    { n: 65, rel: 'crates/mrd/src/pin_cmd.rs' },
    { n: 62, rel: 'crates/mrd/tests/mount_e2e.rs' },
    { n: 58, rel: 'crates/mrd/src/read_cmd.rs' },
    { n: 57, rel: 'crates/mrd/tests/address_owner.rs' },
    { n: 51, rel: 'crates/mrd/src/daemon.rs' },
    { n: 48, rel: 'crates/mrd/src/test_cmd.rs' },
  ] },
  { crate: 'mrd', total: 398, files: [
    { n: 44, rel: 'crates/mrd/src/lib.rs' },
    { n: 43, rel: 'crates/mrd/tests/read_put_cli.rs' },
    { n: 43, rel: 'crates/mrd/tests/board_three_layers.rs' },
    { n: 42, rel: 'crates/mrd/tests/run_lifecycle.rs' },
    { n: 42, rel: 'crates/mrd/src/resolve.rs' },
    { n: 41, rel: 'crates/mrd/tests/retired_verbs.rs' },
    { n: 40, rel: 'crates/mrd/src/put_cmd.rs' },
    { n: 37, rel: 'crates/mrd/tests/journal_genesis.rs' },
    { n: 33, rel: 'crates/mrd/tests/status_walltime.rs' },
  ] },
  { crate: 'mrd', total: 202, files: [
    { n: 32, rel: 'crates/mrd/tests/run_cli.rs' },
    { n: 29, rel: 'crates/mrd/src/reconcile_cmd.rs' },
    { n: 23, rel: 'crates/mrd/tests/walk_e2e.rs' },
    { n: 20, rel: 'crates/mrd/tests/pin_cause.rs' },
    { n: 20, rel: 'crates/mrd/src/unfold_cmd.rs' },
    { n: 20, rel: 'crates/mrd/src/new_cmd.rs' },
    { n: 18, rel: 'crates/mrd/src/gc.rs' },
    { n: 17, rel: 'crates/mrd/src/cache_cmd.rs' },
    { n: 12, rel: 'crates/mrd/examples/board_colors.rs' },
    { n: 11, rel: 'crates/mrd/src/preset_cmd.rs' },
  ] },
  { crate: 'mrd', total: 21, files: [
    { n: 10, rel: 'crates/mrd/src/unregister.rs' },
    { n: 9, rel: 'crates/mrd/tests/preset_e2e.rs' },
    { n: 2, rel: 'crates/mrd/src/main.rs' },
  ] },
  { crate: 'view', total: 517, files: [
    { n: 517, rel: 'crates/view/src/read_face.rs' },
  ] },
  { crate: 'view', total: 576, files: [
    { n: 436, rel: 'crates/view/src/walk.rs' },
    { n: 140, rel: 'crates/view/src/lib.rs' },
  ] },
  { crate: 'view', total: 417, files: [
    { n: 121, rel: 'crates/view/tests/board_view_gates.rs' },
    { n: 89, rel: 'crates/view/tests/board_pin_verdict_gates.rs' },
    { n: 63, rel: 'crates/view/src/facts.rs' },
    { n: 60, rel: 'crates/view/tests/read_face_gates.rs' },
    { n: 59, rel: 'crates/view/tests/gates.rs' },
    { n: 16, rel: 'crates/view/src/schema.rs' },
    { n: 9, rel: 'crates/view/tests/topology_assert.rs' },
  ] },
  { crate: 'policy', total: 447, files: [
    { n: 447, rel: 'crates/policy/src/armed.rs' },
  ] },
  { crate: 'policy', total: 412, files: [
    { n: 412, rel: 'crates/policy/src/registration.rs' },
  ] },
  { crate: 'policy', total: 356, files: [
    { n: 356, rel: 'crates/policy/src/hook.rs' },
  ] },
  { crate: 'policy', total: 558, files: [
    { n: 286, rel: 'crates/policy/src/rule.rs' },
    { n: 272, rel: 'crates/policy/src/pack.rs' },
  ] },
  { crate: 'policy', total: 444, files: [
    { n: 238, rel: 'crates/policy/src/lib.rs' },
    { n: 206, rel: 'crates/policy/src/change.rs' },
  ] },
  { crate: 'policy', total: 526, files: [
    { n: 193, rel: 'crates/policy/src/check_eval.rs' },
    { n: 180, rel: 'crates/policy/src/armed_law.rs' },
    { n: 153, rel: 'crates/policy/src/gate.rs' },
  ] },
  { crate: 'policy', total: 571, files: [
    { n: 114, rel: 'crates/policy/src/defs/rebuild.rs' },
    { n: 99, rel: 'crates/policy/src/binding.rs' },
    { n: 94, rel: 'crates/policy/src/reaction.rs' },
    { n: 73, rel: 'crates/policy/src/declaration.rs' },
    { n: 72, rel: 'crates/policy/tests/compile_gate.rs' },
    { n: 60, rel: 'crates/policy/tests/eval_gate.rs' },
    { n: 59, rel: 'crates/policy/tests/defs_rebuild.rs' },
  ] },
  { crate: 'policy', total: 315, files: [
    { n: 49, rel: 'crates/policy/src/defs/fm.rs' },
    { n: 43, rel: 'crates/policy/src/defs/mod.rs' },
    { n: 40, rel: 'crates/policy/src/defs/load.rs' },
    { n: 34, rel: 'crates/policy/tests/defs_conformance.rs' },
    { n: 29, rel: 'crates/policy/src/defs/i4.rs' },
    { n: 28, rel: 'crates/policy/src/defs/go_fmt.rs' },
    { n: 28, rel: 'crates/policy/src/defs/check.rs' },
    { n: 27, rel: 'crates/policy/tests/purity.rs' },
    { n: 20, rel: 'crates/policy/src/defs/cascade.rs' },
    { n: 17, rel: 'crates/policy/src/defs/shape.rs' },
  ] },
  { crate: 'run', total: 419, files: [
    { n: 419, rel: 'crates/run/src/executor.rs' },
  ] },
  { crate: 'run', total: 537, files: [
    { n: 251, rel: 'crates/run/src/dispatch_bash.rs' },
    { n: 157, rel: 'crates/run/src/caps.rs' },
    { n: 129, rel: 'crates/run/src/runner.rs' },
  ] },
  { crate: 'run', total: 566, files: [
    { n: 125, rel: 'crates/run/src/record.rs' },
    { n: 120, rel: 'crates/run/src/gate.rs' },
    { n: 114, rel: 'crates/run/src/exec.rs' },
    { n: 109, rel: 'crates/run/src/report.rs' },
    { n: 98, rel: 'crates/run/src/shim.rs' },
  ] },
  { crate: 'run', total: 579, files: [
    { n: 94, rel: 'crates/run/src/fp.rs' },
    { n: 93, rel: 'crates/run/tests/s2fix_run_plane_fp.rs' },
    { n: 88, rel: 'crates/run/tests/dispatch_bash.rs' },
    { n: 87, rel: 'crates/run/src/snapshot.rs' },
    { n: 86, rel: 'crates/run/src/address.rs' },
    { n: 71, rel: 'crates/run/tests/workspace_lock_release.rs' },
    { n: 60, rel: 'crates/run/src/lib.rs' },
  ] },
  { crate: 'run', total: 408, files: [
    { n: 51, rel: 'crates/run/tests/executor.rs' },
    { n: 50, rel: 'crates/run/tests/s2fix_run_receipt_fields.rs' },
    { n: 49, rel: 'crates/run/tests/record.rs' },
    { n: 49, rel: 'crates/run/src/dispatch_starlark.rs' },
    { n: 44, rel: 'crates/run/tests/s2fix_task_name_charset.rs' },
    { n: 40, rel: 'crates/run/src/fence.rs' },
    { n: 37, rel: 'crates/run/tests/caps_home.rs' },
    { n: 32, rel: 'crates/run/tests/snapshot.rs' },
    { n: 30, rel: 'crates/run/src/contracts.rs' },
    { n: 26, rel: 'crates/run/tests/runner.rs' },
  ] },
  { crate: 'run', total: 81, files: [
    { n: 20, rel: 'crates/run/tests/exec.rs' },
    { n: 19, rel: 'crates/run/tests/caps.rs' },
    { n: 15, rel: 'crates/run/tests/report.rs' },
    { n: 9, rel: 'crates/run/tests/shim.rs' },
    { n: 6, rel: 'crates/run/tests/support/mod.rs' },
    { n: 5, rel: 'crates/run/tests/dispatch_starlark.rs' },
    { n: 4, rel: 'crates/run/tests/address.rs' },
    { n: 3, rel: 'crates/run/tests/contracts.rs' },
  ] },
  { crate: 'config', total: 375, files: [
    { n: 375, rel: 'crates/config/src/lib.rs' },
  ] },
  { crate: 'config', total: 530, files: [
    { n: 332, rel: 'crates/config/src/mount.rs' },
    { n: 198, rel: 'crates/config/src/bridge.rs' },
  ] },
  { crate: 'config', total: 369, files: [
    { n: 168, rel: 'crates/config/src/mount/tests.rs' },
    { n: 129, rel: 'crates/config/src/bridge/tests.rs' },
    { n: 72, rel: 'crates/config/src/tests.rs' },
  ] },
  { crate: 'effects', total: 373, files: [
    { n: 373, rel: 'crates/effects/src/lib.rs' },
  ] },
  { crate: 'effects', total: 408, files: [
    { n: 257, rel: 'crates/effects/src/kernel.rs' },
    { n: 28, rel: 'crates/effects/tests/determinism.rs' },
    { n: 23, rel: 'crates/effects/tests/fuel.rs' },
    { n: 20, rel: 'crates/effects/tests/purity.rs' },
    { n: 17, rel: 'crates/effects/tests/hostility.rs' },
    { n: 16, rel: 'crates/effects/tests/robustness.rs' },
    { n: 15, rel: 'crates/effects/tests/run_entry.rs' },
    { n: 15, rel: 'crates/effects/tests/golden.rs' },
    { n: 10, rel: 'crates/effects/tests/cascade.rs' },
    { n: 7, rel: 'crates/effects/tests/capability.rs' },
  ] },
  { crate: 'effects', total: 16, files: [
    { n: 6, rel: 'crates/effects/tests/support/mod.rs' },
    { n: 4, rel: 'crates/effects/fuzz/fuzz_targets/eval_source.rs' },
    { n: 3, rel: 'crates/effects/tests/validate.rs' },
    { n: 3, rel: 'crates/effects/fuzz/fuzz_targets/eval_input.rs' },
  ] },
  { crate: 'registry', total: 362, files: [
    { n: 362, rel: 'crates/registry/src/registry.rs' },
  ] },
  { crate: 'registry', total: 569, files: [
    { n: 271, rel: 'crates/registry/src/server.rs' },
    { n: 102, rel: 'crates/registry/tests/hello.rs' },
    { n: 75, rel: 'crates/registry/src/protocol.rs' },
    { n: 61, rel: 'crates/registry/tests/wire_vocab_rev.rs' },
    { n: 60, rel: 'crates/registry/src/lib.rs' },
  ] },
  { crate: 'registry', total: 430, files: [
    { n: 59, rel: 'crates/registry/tests/read_mint.rs' },
    { n: 57, rel: 'crates/registry/tests/write_e2e.rs' },
    { n: 53, rel: 'crates/registry/src/client.rs' },
    { n: 51, rel: 'crates/registry/tests/engine_rpc.rs' },
    { n: 41, rel: 'crates/registry/src/engine.rs' },
    { n: 39, rel: 'crates/registry/src/state.rs' },
    { n: 27, rel: 'crates/registry/tests/rpc.rs' },
  ] },
  { crate: 'git', total: 443, files: [
    { n: 358, rel: 'crates/git/src/lib.rs' },
    { n: 85, rel: 'crates/git/tests/plumbing.rs' },
  ] },
  { crate: 'addr', total: 572, files: [
    { n: 343, rel: 'crates/addr/src/lib.rs' },
    { n: 150, rel: 'crates/addr/src/stored.rs' },
    { n: 79, rel: 'crates/addr/tests/ingress_enumeration.rs' },
  ] },
  { crate: 'preset', total: 424, files: [
    { n: 341, rel: 'crates/preset/src/lib.rs' },
    { n: 83, rel: 'crates/preset/tests/gates.rs' },
  ] },
  { crate: 'receipt', total: 573, files: [
    { n: 325, rel: 'crates/receipt/src/anchor.rs' },
    { n: 149, rel: 'crates/receipt/src/journal.rs' },
    { n: 99, rel: 'crates/receipt/src/lib.rs' },
  ] },
  { crate: 'receipt', total: 158, files: [
    { n: 91, rel: 'crates/receipt/src/read_mint.rs' },
    { n: 51, rel: 'crates/receipt/tests/s2fix_claim_token.rs' },
    { n: 16, rel: 'crates/receipt/tests/frozen_receipts.rs' },
  ] },
  { crate: 'syntax', total: 297, files: [
    { n: 297, rel: 'crates/syntax/src/lib.rs' },
  ] },
  { crate: 'lock', total: 246, files: [
    { n: 246, rel: 'crates/lock/src/lib.rs' },
  ] },
  { crate: 'realise', total: 259, files: [
    { n: 207, rel: 'crates/realise/src/lib.rs' },
    { n: 52, rel: 'crates/realise/tests/scenarios.rs' },
  ] },
  { crate: 'workspace', total: 211, files: [
    { n: 183, rel: 'crates/workspace/src/lib.rs' },
    { n: 28, rel: 'crates/workspace/tests/discovery.rs' },
  ] },
  { crate: 'testsuite', total: 580, files: [
    { n: 173, rel: 'crates/testsuite/tests/pf_frozen_sweep.rs' },
    { n: 110, rel: 'crates/testsuite/tests/meridian_md.rs' },
    { n: 84, rel: 'crates/testsuite/tests/u36_per_language_elision.rs' },
    { n: 77, rel: 'crates/testsuite/tests/s1_authz_facts.rs' },
    { n: 51, rel: 'crates/testsuite/tests/yaml_confinement.rs' },
    { n: 47, rel: 'crates/testsuite/tests/u4a2_composed_read.rs' },
    { n: 38, rel: 'crates/testsuite/tests/armed_disk_agreement.rs' },
  ] },
  { crate: 'testsuite', total: 197, files: [
    { n: 36, rel: 'crates/testsuite/tests/walk_resolve.rs' },
    { n: 27, rel: 'crates/testsuite/tests/delta_e3e4.rs' },
    { n: 25, rel: 'crates/testsuite/tests/charset_guard.rs' },
    { n: 25, rel: 'crates/testsuite/src/lib.rs' },
    { n: 20, rel: 'crates/testsuite/tests/wsfix_oracle.rs' },
    { n: 20, rel: 'crates/testsuite/tests/u0_read_parity.rs' },
    { n: 14, rel: 'crates/testsuite/tests/u4b_real_lock.rs' },
    { n: 12, rel: 'crates/testsuite/tests/u4a1_render_parity.rs' },
    { n: 10, rel: 'crates/testsuite/tests/gt_parse.rs' },
    { n: 8, rel: 'crates/testsuite/tests/wire_vocab.rs' },
  ] },
  { crate: 'testsuite', total: 5, files: [
    { n: 3, rel: 'crates/testsuite/tests/main.rs' },
    { n: 2, rel: 'crates/testsuite/tests/gt_pack_smoke.rs' },
  ] },
  { crate: 'cache', total: 532, files: [
    { n: 172, rel: 'crates/cache/tests/drawer_lock_release.rs' },
    { n: 99, rel: 'crates/cache/src/sentinel.rs' },
    { n: 79, rel: 'crates/cache/src/lib.rs' },
    { n: 64, rel: 'crates/cache/src/sweep.rs' },
    { n: 60, rel: 'crates/cache/src/locking.rs' },
    { n: 31, rel: 'crates/cache/src/layout.rs' },
    { n: 27, rel: 'crates/cache/tests/integration.rs' },
  ] },
  { crate: 'sidecar', total: 507, files: [
    { n: 146, rel: 'crates/sidecar/tests/dispatch_v2.rs' },
    { n: 141, rel: 'crates/sidecar/src/lib.rs' },
    { n: 123, rel: 'crates/sidecar/src/watch.rs' },
    { n: 97, rel: 'crates/sidecar/src/arms.rs' },
  ] },
  { crate: 'sidecar', total: 579, files: [
    { n: 96, rel: 'crates/sidecar/tests/splice_e2e.rs' },
    { n: 91, rel: 'crates/sidecar/tests/reconcile_demand.rs' },
    { n: 77, rel: 'crates/sidecar/tests/contract_v3.rs' },
    { n: 66, rel: 'crates/sidecar/tests/create_e2e.rs' },
    { n: 57, rel: 'crates/sidecar/tests/verdicts_e2e.rs' },
    { n: 55, rel: 'crates/sidecar/tests/replay_live.rs' },
    { n: 55, rel: 'crates/sidecar/src/ring.rs' },
    { n: 43, rel: 'crates/sidecar/tests/sub_push.rs' },
    { n: 39, rel: 'crates/sidecar/tests/watch_external.rs' },
  ] },
  { crate: 'sidecar', total: 152, files: [
    { n: 35, rel: 'crates/sidecar/tests/p4_probes.rs' },
    { n: 32, rel: 'crates/sidecar/src/policy_bridge.rs' },
    { n: 29, rel: 'crates/sidecar/tests/selector_ambiguity.rs' },
    { n: 22, rel: 'crates/sidecar/tests/hook_notify.rs' },
    { n: 19, rel: 'crates/sidecar/tests/v2_compat_e2e.rs' },
    { n: 9, rel: 'crates/sidecar/src/main.rs' },
  ] },
  { crate: 'transport-proto', total: 197, files: [
    { n: 142, rel: 'crates/transport-proto/tests/wire_agreement.rs' },
    { n: 53, rel: 'crates/transport-proto/src/lib.rs' },
    { n: 2, rel: 'crates/transport-proto/build.rs' },
  ] },
  { crate: 'render', total: 232, files: [
    { n: 137, rel: 'crates/render/src/lib.rs' },
    { n: 52, rel: 'crates/render/src/walk.rs' },
    { n: 43, rel: 'crates/render/tests/u4b_elision.rs' },
  ] },
  { crate: 'transcript', total: 202, files: [
    { n: 131, rel: 'crates/transcript/src/lib.rs' },
    { n: 71, rel: 'crates/transcript/tests/gates.rs' },
  ] },
  { crate: 'query', total: 114, files: [
    { n: 114, rel: 'crates/query/src/lib.rs' },
  ] },
  { crate: 'wire-map', total: 282, files: [
    { n: 110, rel: 'crates/wire-map/src/facts.rs' },
    { n: 92, rel: 'crates/wire-map/src/lib.rs' },
    { n: 53, rel: 'crates/wire-map/src/gotext.rs' },
    { n: 27, rel: 'crates/wire-map/tests/b1_superset.rs' },
  ] },
  { crate: 'transport', total: 93, files: [
    { n: 93, rel: 'crates/transport/src/lib.rs' },
  ] },
  { crate: 'perfsuite', total: 346, files: [
    { n: 59, rel: 'crates/perfsuite/benches/policy.rs' },
    { n: 44, rel: 'crates/perfsuite/src/generator.rs' },
    { n: 41, rel: 'crates/perfsuite/src/profile.rs' },
    { n: 41, rel: 'crates/perfsuite/benches/roundtrip.rs' },
    { n: 32, rel: 'crates/perfsuite/src/corpus.rs' },
    { n: 32, rel: 'crates/perfsuite/benches/cold_ingest.rs' },
    { n: 28, rel: 'crates/perfsuite/src/lib.rs' },
    { n: 26, rel: 'crates/perfsuite/src/report.rs' },
    { n: 22, rel: 'crates/perfsuite/benches/assemble.rs' },
    { n: 21, rel: 'crates/perfsuite/tests/results_freshness.rs' },
  ] },
  { crate: 'perfsuite', total: 118, files: [
    { n: 20, rel: 'crates/perfsuite/benches/project.rs' },
    { n: 18, rel: 'crates/perfsuite/src/claims.rs' },
    { n: 18, rel: 'crates/perfsuite/src/bin/corpusgen.rs' },
    { n: 17, rel: 'crates/perfsuite/src/measure.rs' },
    { n: 16, rel: 'crates/perfsuite/benches/parse.rs' },
    { n: 10, rel: 'crates/perfsuite/tests/harness.rs' },
    { n: 8, rel: 'crates/perfsuite/tests/inventory_assert.rs' },
    { n: 7, rel: 'crates/perfsuite/src/inventory.rs' },
    { n: 4, rel: 'crates/perfsuite/src/bin/bench-report.rs' },
  ] },
]

// args: {limit: N} runs only the first N in-scope batches (pilot);
// {skip: [i, ...]} excludes batches by their index in BATCHES (already-done runs).
const skipSet = new Set((args && args.skip) || [])
const withIdx = BATCHES.map((b, i) => ({ ...b, idx: i }))
const SCOPE = withIdx.filter(b => !skipSet.has(b.idx)).slice(0, (args && args.limit) || BATCHES.length)
const nFiles = SCOPE.reduce((a, b) => a + b.files.length, 0)
log('comment-cleanup: ' + nFiles + ' files in ' + SCOPE.length + ' of ' + BATCHES.length + ' batches, rewrite=grok-exec review=codex-exec, md root ' + MD_ROOT)

const results = await pipeline(
  SCOPE,
  (b, _, i) => agent(rewritePrompt(b), {
    agentType: 'grok-exec', model: 'sonnet', effort: 'low',
    label: 'rewrite:' + b.crate + '#' + b.idx + ' (' + b.files.length + ' files)',
    phase: 'Rewrite', schema: REWRITE_SCHEMA,
  }).then(rw => rw ?? { status: 'failed', reason: 'rewrite agent lost (skipped or terminal API error)', files: [], summary: '' }),
  (rw, b, i) => {
    if (rw.status === 'failed' && rw.files.length === 0) {
      return { batch: b, rewrite: rw, review: null }
    }
    return agent(reviewPrompt(b, rw), {
      agentType: 'codex-exec', model: 'sonnet', effort: 'low',
      label: 'review:' + b.crate + '#' + b.idx + ' (' + b.files.length + ' files)',
      phase: 'Review', schema: REVIEW_SCHEMA,
    }).then(rv => ({ batch: b, rewrite: rw, review: rv }))
  },
)

// ---- flatten to per-file records ------------------------------------------
const perFile = []
const lostBatches = []
results.forEach((r, i) => {
  if (!r) { lostBatches.push(SCOPE[i].idx); SCOPE[i].files.forEach(f => perFile.push({ rel: f.rel, verdict: 'LOST', changed: false, mdCreated: false, detail: 'batch pipeline item lost', batch: SCOPE[i].idx })); return }
  const fallback = r.review === null ? 'SKIPPED' : 'UNREVIEWED'
  for (const f of r.batch.files) {
    const rw = r.rewrite?.files?.find(x => x.rel === f.rel)
    const rv = r.review?.files?.find(x => x.rel === f.rel)
    perFile.push({
      rel: f.rel,
      verdict: rv?.verdict ?? fallback,
      changed: rw?.changed ?? false,
      mdCreated: rw?.mdCreated ?? false,
      detail: rv?.detail ?? (r.review === null ? (r.rewrite?.reason ?? 'rewrite failed') : (r.review?.reason ?? 'no per-file verdict returned')),
      batch: r.batch.idx,
    })
  }
})

const count = v => perFile.filter(f => f.verdict === v).length
const summary = {
  totalFiles: nFiles,
  totalBatches: SCOPE.length,
  go: count('GO'),
  nogo: count('NO-GO'),
  skipped: count('SKIPPED'),
  unreviewed: count('UNREVIEWED'),
  lost: count('LOST'),
  mdRoot: MD_ROOT,
  nogoFiles: perFile.filter(f => f.verdict === 'NO-GO').map(f => ({ rel: f.rel, detail: f.detail, md: mdRel(f.rel) })),
  failedBatches: results.map((r, i) => ({ r, i }))
    .filter(({ r }) => !r || r.rewrite?.status !== 'ok' || (r.review && r.review.status !== 'ok'))
    .map(({ r, i }) => ({
      batch: SCOPE[i].idx, crate: SCOPE[i].crate,
      rewrite: r?.rewrite ? { status: r.rewrite.status, reason: r.rewrite.reason ?? '', sessionId: r.rewrite.sessionId ?? '' } : null,
      review: r?.review ? { status: r.review.status, reason: r.review.reason ?? '', sessionId: r.review.sessionId ?? '' } : null,
    })),
  files: perFile,
}

log('comment-cleanup done: ' + summary.go + ' GO / ' + summary.nogo + ' NO-GO / '
  + summary.skipped + ' skipped / ' + summary.unreviewed + ' unreviewed / '
  + summary.lost + ' lost, of ' + nFiles + ' files in ' + SCOPE.length + ' batches')

return summary
