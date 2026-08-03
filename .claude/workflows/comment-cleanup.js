export const meta = {
  name: 'comment-cleanup',
  description: 'Minimize comments across all meridian-rs .rs files. Per file: grok-exec rewrites comments only (no code change, no build, no commit, no worktree), then codex-exec reviews that one rewrite. Key information and review verdicts land in docs/comment-cleanup/ mirror .md files. NO-GO is recorded, never iterated.',
  phases: [
    { title: 'Rewrite', detail: 'grok CLI, one agent per .rs file, comments only, mechanical invariance self-check' },
    { title: 'Review', detail: 'codex CLI, independent invariance run + info-preservation + style, verdict appended to mirror md' },
  ],
}

// Contract (fixed before launch):
// - Scope: the FILES array below — every tracked .rs file, heaviest comments first.
//   Trim entries to narrow scope; n = approximate comment-line count.
// - Only allowed md location: docs/comment-cleanup/<rust path>.md (mirror tree,
//   declared here up front so no post-run move is needed).
// - Code-invariance gate: tools/comment-audit/invariance.py (selftested).
//   Both the rewriter (self-check) and the reviewer (independent) must run it.
// - A NO-GO verdict is written into the mirror md and collected; the workflow
//   never loops back to the rewriter.

const REPO = '/Users/Shared/repos/meridian-rs'
const MD_ROOT = 'docs/comment-cleanup'
const mdRel = rel => MD_ROOT + '/' + rel.replace(/\.rs$/, '.md')

const REWRITE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['status', 'changed', 'mdCreated', 'summary'],
  properties: {
    status: { type: 'string', enum: ['ok', 'failed'] },
    reason: { type: 'string', description: 'on failure: transient | rate-limit | config | task, plus one line' },
    sessionId: { type: 'string', description: 'CLI session id, for manual resume' },
    changed: { type: 'boolean' },
    mdCreated: { type: 'boolean' },
    gate: { type: 'string', description: 'verbatim PASS/FAIL line from the invariance run; empty if changed=false' },
    summary: { type: 'string', description: '1-3 sentences: what was deleted, simplified, moved' },
  },
}

const REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['status', 'verdict', 'detail'],
  properties: {
    status: { type: 'string', enum: ['ok', 'failed'] },
    reason: { type: 'string', description: 'on failure: transient | rate-limit | config | task, plus one line' },
    sessionId: { type: 'string', description: 'CLI session id, for manual resume' },
    verdict: { type: 'string', enum: ['GO', 'NO-GO', 'UNREVIEWED'], description: 'UNREVIEWED only with status failed' },
    gate: { type: 'string', description: 'verbatim PASS/FAIL line from the independent invariance run' },
    detail: { type: 'string', description: 'condensed verdict, 1-3 sentences' },
  },
}

const rewritePrompt = f => `<grok-exec:dispatch>
model: grok-4-5 (fall back to the CLI default if this id is unknown)
max execution time: 20 minutes
</grok-exec:dispatch>
<grok-exec:payload>
You are one agent in a fleet minimizing comments across a Rust repo. You own exactly ONE file.

Repo: ${REPO}   (cd there first; all paths below are repo-relative)
Your file: ${f.rel}   (~${f.n} comment lines)
Your notes file (create only if needed): ${mdRel(f.rel)}

WRITE ACCESS — exactly two paths: ${f.rel} and ${mdRel(f.rel)}. Other agents work on other files concurrently; never touch anything else. mkdir -p the notes file's parent directory before creating it.

HARD RULES
- Comments only. Every non-comment token must survive byte-identical: no code, string, attribute, or macro changes. Never delete a comment that separates two tokens without leaving a space.
- No cargo, rustc, build, test, fmt, or clippy runs. No worktree. Git is read-only for you: never commit, stash, checkout, branch, or add.
- Before finishing, prove code-invariance mechanically:
    B=$(mktemp -d)/before.rs
    git show HEAD:${f.rel} > "$B"
    python3 tools/comment-audit/invariance.py "$B" ${f.rel}
  It must print PASS. If it prints FAIL, repair your edit until it passes. If you cannot, run git restore ${f.rel} (the one allowed git write, scoped to your own file) and report status failed with reason task.

STYLE TARGET — comments stay minimal; the llm-wiki outside this repo holds the comprehensive docs.
- Delete: comments narrating what the code does, restating names or types, changelog-style remarks ("added for...", "used by..."), commented-out code, decorative banner rules.
- Keep, simplifying when wordy: SAFETY comments on unsafe blocks (must remain), TODO/FIXME markers, license headers, and any WHY a maintainer needs at the cursor — hidden constraints, subtle invariants, workaround-for-bug notes.
- Doc comments (/// and //!): keep a one-line summary plus only load-bearing detail.
- KEY INFORMATION RULE: when a comment paragraph carries real substance too big for one line — design rationale, protocol or contract detail, invariant chains, findings about system state — move that substance into your notes file BEFORE shrinking the comment. Append a section:
    ## <topic> (from ${f.rel}:<line>)
    <the preserved substance, lightly edited>
  Then leave at most one short line in code. On first use create the notes file starting with the line: # ${f.rel} — comment-cleanup notes
  Do not write references to the notes file inside code comments.
- If the file's comments are already minimal, change nothing and report changed=false.

RETURN a single JSON object per the schema you were given: status, reason, sessionId, changed, mdCreated, gate, summary.
</grok-exec:payload>`

const reviewPrompt = (f, rw) => `<codex-exec:dispatch>
model: gpt-5-6-sol, reasoning effort: medium (fall back to the CLI default model at medium reasoning if this id is unknown)
max execution time: 15 minutes
</codex-exec:dispatch>
<codex-exec:payload>
You review ONE other agent's comment-cleanup edit in a Rust repo. Review only: you fix nothing, revert nothing, and never iterate with the other agent.

Repo: ${REPO}   (cd there first; paths below are repo-relative)
File under review: ${f.rel}
Notes/verdict file (the ONLY path you may write): ${mdRel(f.rel)}
Rewriter's report: ${JSON.stringify(rw)}

Other agents are editing OTHER files concurrently: ignore every path except the two above. The rewrite of ${f.rel} is finished; judge the working-tree version against git show HEAD:${f.rel}.

CHECKS, in order:
1. Code-invariance (mechanical):
    B=$(mktemp -d)/before.rs
    git show HEAD:${f.rel} > "$B"
    python3 tools/comment-audit/invariance.py "$B" ${f.rel}
   FAIL here is an automatic NO-GO regardless of everything else.
2. Information preservation: read git diff HEAD -- ${f.rel}. For every deleted or shortened paragraph that carried non-obvious substance — invariants, contracts, SAFETY reasoning, workarounds, findings about system state — verify the substance survives in the remaining comment or in ${mdRel(f.rel)}. Every unsafe block must still carry a SAFETY comment.
3. Style: remaining comments are minimal — no narration of what code does, no commented-out code, doc comments short — and reworded comments are still accurate (a simplified comment must not say something false about the code).

If the rewriter reported changed=false: verify git diff HEAD -- ${f.rel} really is empty; GO with detail "no changes" when true, NO-GO when the report lied.

VERDICT: GO or NO-GO. Append to ${mdRel(f.rel)} (mkdir -p the parent; if the file is missing, create it starting with the line: # ${f.rel} — comment-cleanup notes ; never edit existing sections):

## Review verdict: <GO|NO-GO>
- reviewer: codex-exec
- code-invariance: pass|fail — <gate line verbatim>
- info-preservation: pass|fail — <one line>
- style: pass|fail — <one line>
- detail: <specifics; for NO-GO name exactly what is wrong and where>

RETURN a single JSON object per the schema you were given: status, reason, sessionId, verdict (GO | NO-GO; UNREVIEWED only with status failed), gate, detail.
</codex-exec:payload>`

// Every tracked .rs file, sorted by approximate comment-line count, heaviest first.
const FILES = [
  { n: 1358, rel: 'crates/wire-serve/src/write.rs' },
  { n: 985, rel: 'crates/model/src/lib.rs' },
  { n: 874, rel: 'crates/wire/src/lib.rs' },
  { n: 603, rel: 'crates/fs/src/lib.rs' },
  { n: 601, rel: 'crates/check/src/layer0.rs' },
  { n: 566, rel: 'crates/mrd/src/check_cmd.rs' },
  { n: 517, rel: 'crates/view/src/read_face.rs' },
  { n: 455, rel: 'crates/mrd/src/status_cmd.rs' },
  { n: 447, rel: 'crates/policy/src/armed.rs' },
  { n: 436, rel: 'crates/view/src/walk.rs' },
  { n: 419, rel: 'crates/run/src/executor.rs' },
  { n: 412, rel: 'crates/policy/src/registration.rs' },
  { n: 402, rel: 'crates/mrd/src/corpus_tier.rs' },
  { n: 392, rel: 'crates/model/src/selector.rs' },
  { n: 375, rel: 'crates/config/src/lib.rs' },
  { n: 373, rel: 'crates/effects/src/lib.rs' },
  { n: 362, rel: 'crates/registry/src/registry.rs' },
  { n: 358, rel: 'crates/git/src/lib.rs' },
  { n: 356, rel: 'crates/policy/src/hook.rs' },
  { n: 356, rel: 'crates/mrd/tests/s4r21_fence_line.rs' },
  { n: 351, rel: 'crates/mrd/tests/s2fix_cross_surface.rs' },
  { n: 343, rel: 'crates/addr/src/lib.rs' },
  { n: 341, rel: 'crates/preset/src/lib.rs' },
  { n: 334, rel: 'crates/mrd/tests/u15_hook_fence_e2e.rs' },
  { n: 332, rel: 'crates/config/src/mount.rs' },
  { n: 325, rel: 'crates/receipt/src/anchor.rs' },
  { n: 309, rel: 'crates/wire-serve/tests/u12_door_enumeration.rs' },
  { n: 297, rel: 'crates/syntax/src/lib.rs' },
  { n: 286, rel: 'crates/policy/src/rule.rs' },
  { n: 280, rel: 'crates/wire-serve/src/positions.rs' },
  { n: 273, rel: 'crates/fs/src/domain.rs' },
  { n: 272, rel: 'crates/policy/src/pack.rs' },
  { n: 271, rel: 'crates/registry/src/server.rs' },
  { n: 265, rel: 'crates/wire-serve/src/read.rs' },
  { n: 260, rel: 'crates/mrd/src/hook.rs' },
  { n: 257, rel: 'crates/effects/src/kernel.rs' },
  { n: 251, rel: 'crates/run/src/dispatch_bash.rs' },
  { n: 246, rel: 'crates/lock/src/lib.rs' },
  { n: 238, rel: 'crates/policy/src/lib.rs' },
  { n: 232, rel: 'crates/mrd/tests/s3_cross_surface.rs' },
  { n: 223, rel: 'crates/model/src/fingerprint.rs' },
  { n: 215, rel: 'crates/mrd/src/sql.rs' },
  { n: 215, rel: 'crates/fs/src/guard.rs' },
  { n: 210, rel: 'crates/mrd/src/history_cmd.rs' },
  { n: 208, rel: 'crates/mrd/tests/rules_cli.rs' },
  { n: 207, rel: 'crates/realise/src/lib.rs' },
  { n: 206, rel: 'crates/policy/src/change.rs' },
  { n: 198, rel: 'crates/config/src/bridge.rs' },
  { n: 193, rel: 'crates/policy/src/check_eval.rs' },
  { n: 189, rel: 'crates/wire-serve/src/gate.rs' },
  { n: 183, rel: 'crates/workspace/src/lib.rs' },
  { n: 183, rel: 'crates/mrd/tests/u14_check_pin_plane.rs' },
  { n: 183, rel: 'crates/mrd/tests/s4r19_commit_gate.rs' },
  { n: 180, rel: 'crates/policy/src/armed_law.rs' },
  { n: 180, rel: 'crates/mrd/tests/check_e2e.rs' },
  { n: 173, rel: 'crates/testsuite/tests/pf_frozen_sweep.rs' },
  { n: 172, rel: 'crates/cache/tests/drawer_lock_release.rs' },
  { n: 169, rel: 'crates/mrd/src/rules_cmd.rs' },
  { n: 168, rel: 'crates/config/src/mount/tests.rs' },
  { n: 166, rel: 'crates/mrd/tests/config_e2e.rs' },
  { n: 162, rel: 'crates/mrd/src/config_cmd.rs' },
  { n: 157, rel: 'crates/run/src/caps.rs' },
  { n: 154, rel: 'crates/wire-serve/src/plan.rs' },
  { n: 153, rel: 'crates/policy/src/gate.rs' },
  { n: 151, rel: 'crates/wire-serve/tests/s7_pin.rs' },
  { n: 150, rel: 'crates/addr/src/stored.rs' },
  { n: 149, rel: 'crates/receipt/src/journal.rs' },
  { n: 149, rel: 'crates/check/src/lib.rs' },
  { n: 146, rel: 'crates/sidecar/tests/dispatch_v2.rs' },
  { n: 145, rel: 'crates/mrd/tests/skill_hook_emit.rs' },
  { n: 145, rel: 'crates/mrd/tests/hook_plane_fence.rs' },
  { n: 142, rel: 'crates/transport-proto/tests/wire_agreement.rs' },
  { n: 141, rel: 'crates/sidecar/src/lib.rs' },
  { n: 140, rel: 'crates/view/src/lib.rs' },
  { n: 139, rel: 'crates/wire/tests/contract_v2.rs' },
  { n: 139, rel: 'crates/wire-serve/src/decode.rs' },
  { n: 137, rel: 'crates/render/src/lib.rs' },
  { n: 131, rel: 'crates/transcript/src/lib.rs' },
  { n: 130, rel: 'crates/wire-serve/src/rev.rs' },
  { n: 129, rel: 'crates/run/src/runner.rs' },
  { n: 129, rel: 'crates/config/src/bridge/tests.rs' },
  { n: 127, rel: 'crates/mrd/tests/e2e.rs' },
  { n: 126, rel: 'crates/check/src/layer1.rs' },
  { n: 125, rel: 'crates/run/src/record.rs' },
  { n: 123, rel: 'crates/sidecar/src/watch.rs' },
  { n: 123, rel: 'crates/mrd/src/run_cmd.rs' },
  { n: 123, rel: 'crates/model/src/delta.rs' },
  { n: 122, rel: 'crates/wire-serve/src/reaction.rs' },
  { n: 121, rel: 'crates/view/tests/board_view_gates.rs' },
  { n: 120, rel: 'crates/run/src/gate.rs' },
  { n: 118, rel: 'crates/mrd/tests/floors.rs' },
  { n: 118, rel: 'crates/mrd/src/walk_cmd.rs' },
  { n: 115, rel: 'crates/wire-serve/tests/s10_fp_decorate.rs' },
  { n: 114, rel: 'crates/run/src/exec.rs' },
  { n: 114, rel: 'crates/query/src/lib.rs' },
  { n: 114, rel: 'crates/policy/src/defs/rebuild.rs' },
  { n: 113, rel: 'crates/mrd/tests/u32_governed_journal.rs' },
  { n: 112, rel: 'crates/mrd/tests/u35_journaled_doors.rs' },
  { n: 110, rel: 'crates/wire-map/src/facts.rs' },
  { n: 110, rel: 'crates/testsuite/tests/meridian_md.rs' },
  { n: 109, rel: 'crates/run/src/report.rs' },
  { n: 109, rel: 'crates/mrd/tests/status_e2e.rs' },
  { n: 109, rel: 'crates/fs/tests/exec_guard.rs' },
  { n: 108, rel: 'crates/wire-serve/src/lib.rs' },
  { n: 104, rel: 'crates/mrd/tests/u13_per_root_anchoring.rs' },
  { n: 103, rel: 'crates/mrd/tests/corpus_suite.rs' },
  { n: 102, rel: 'crates/registry/tests/hello.rs' },
  { n: 99, rel: 'crates/receipt/src/lib.rs' },
  { n: 99, rel: 'crates/policy/src/binding.rs' },
  { n: 99, rel: 'crates/cache/src/sentinel.rs' },
  { n: 98, rel: 'crates/run/src/shim.rs' },
  { n: 97, rel: 'crates/sidecar/src/arms.rs' },
  { n: 97, rel: 'crates/mrd/tests/color_planes_e2e.rs' },
  { n: 97, rel: 'crates/model/tests/s2fix_empty_span.rs' },
  { n: 96, rel: 'crates/sidecar/tests/splice_e2e.rs' },
  { n: 96, rel: 'crates/mrd/tests/f4_declared_unbound_root.rs' },
  { n: 94, rel: 'crates/run/src/fp.rs' },
  { n: 94, rel: 'crates/policy/src/reaction.rs' },
  { n: 93, rel: 'crates/transport/src/lib.rs' },
  { n: 93, rel: 'crates/run/tests/s2fix_run_plane_fp.rs' },
  { n: 93, rel: 'crates/check/tests/end_to_end.rs' },
  { n: 92, rel: 'crates/wire-map/src/lib.rs' },
  { n: 92, rel: 'crates/mrd/src/engine.rs' },
  { n: 91, rel: 'crates/sidecar/tests/reconcile_demand.rs' },
  { n: 91, rel: 'crates/receipt/src/read_mint.rs' },
  { n: 91, rel: 'crates/mrd/src/rules_walk.rs' },
  { n: 89, rel: 'crates/view/tests/board_pin_verdict_gates.rs' },
  { n: 89, rel: 'crates/mrd/src/journal_cmd.rs' },
  { n: 89, rel: 'crates/model/src/walk.rs' },
  { n: 88, rel: 'crates/run/tests/dispatch_bash.rs' },
  { n: 88, rel: 'crates/mrd/tests/history.rs' },
  { n: 88, rel: 'crates/mrd/tests/f6_check_sees_the_mount_table.rs' },
  { n: 87, rel: 'crates/run/src/snapshot.rs' },
  { n: 86, rel: 'crates/run/src/address.rs' },
  { n: 86, rel: 'crates/mrd/tests/u11r2_declared_but_unreadable.rs' },
  { n: 86, rel: 'crates/fs/tests/write_lock_release.rs' },
  { n: 85, rel: 'crates/git/tests/plumbing.rs' },
  { n: 84, rel: 'crates/testsuite/tests/u36_per_language_elision.rs' },
  { n: 83, rel: 'crates/preset/tests/gates.rs' },
  { n: 83, rel: 'crates/mrd/tests/u12_stored_form_e2e.rs' },
  { n: 82, rel: 'crates/wire-serve/tests/s2fix_rule_id_journal.rs' },
  { n: 80, rel: 'crates/mrd/src/realise_cmd.rs' },
  { n: 79, rel: 'crates/mrd/tests/u11_cross_root_walk_e2e.rs' },
  { n: 79, rel: 'crates/cache/src/lib.rs' },
  { n: 79, rel: 'crates/addr/tests/ingress_enumeration.rs' },
  { n: 77, rel: 'crates/wire-serve/tests/u5_4_substrate.rs' },
  { n: 77, rel: 'crates/testsuite/tests/s1_authz_facts.rs' },
  { n: 77, rel: 'crates/sidecar/tests/contract_v3.rs' },
  { n: 76, rel: 'crates/wire-serve/tests/s2fix_promotion.rs' },
  { n: 75, rel: 'crates/registry/src/protocol.rs' },
  { n: 75, rel: 'crates/mrd/src/init.rs' },
  { n: 75, rel: 'crates/check/src/orphan.rs' },
  { n: 74, rel: 'crates/mrd/src/skill_cmd.rs' },
  { n: 73, rel: 'crates/policy/src/declaration.rs' },
  { n: 72, rel: 'crates/policy/tests/compile_gate.rs' },
  { n: 72, rel: 'crates/config/src/tests.rs' },
  { n: 71, rel: 'crates/wire-serve/tests/s2fix_empty_span_mint.rs' },
  { n: 71, rel: 'crates/transcript/tests/gates.rs' },
  { n: 71, rel: 'crates/run/tests/workspace_lock_release.rs' },
  { n: 70, rel: 'crates/wire-serve/tests/s2fix_fp_document_grain.rs' },
  { n: 68, rel: 'crates/mrd/tests/sql_e2e.rs' },
  { n: 67, rel: 'crates/wire-serve/tests/s2fix_artifact_guard.rs' },
  { n: 66, rel: 'crates/sidecar/tests/create_e2e.rs' },
  { n: 65, rel: 'crates/mrd/src/pin_cmd.rs' },
  { n: 65, rel: 'crates/model/tests/norm_v2_fixtures.rs' },
  { n: 64, rel: 'crates/cache/src/sweep.rs' },
  { n: 63, rel: 'crates/view/src/facts.rs' },
  { n: 62, rel: 'crates/mrd/tests/mount_e2e.rs' },
  { n: 61, rel: 'crates/registry/tests/wire_vocab_rev.rs' },
  { n: 60, rel: 'crates/view/tests/read_face_gates.rs' },
  { n: 60, rel: 'crates/run/src/lib.rs' },
  { n: 60, rel: 'crates/registry/src/lib.rs' },
  { n: 60, rel: 'crates/policy/tests/eval_gate.rs' },
  { n: 60, rel: 'crates/cache/src/locking.rs' },
  { n: 59, rel: 'crates/wire-serve/src/check_write.rs' },
  { n: 59, rel: 'crates/view/tests/gates.rs' },
  { n: 59, rel: 'crates/registry/tests/read_mint.rs' },
  { n: 59, rel: 'crates/policy/tests/defs_rebuild.rs' },
  { n: 59, rel: 'crates/perfsuite/benches/policy.rs' },
  { n: 58, rel: 'crates/registry/tests/view_path_e2e.rs' },
  { n: 58, rel: 'crates/mrd/src/read_cmd.rs' },
  { n: 57, rel: 'crates/sidecar/tests/verdicts_e2e.rs' },
  { n: 57, rel: 'crates/registry/tests/write_e2e.rs' },
  { n: 57, rel: 'crates/mrd/tests/address_owner.rs' },
  { n: 55, rel: 'crates/sidecar/tests/replay_live.rs' },
  { n: 55, rel: 'crates/sidecar/src/ring.rs' },
  { n: 53, rel: 'crates/wire-map/src/gotext.rs' },
  { n: 53, rel: 'crates/transport-proto/src/lib.rs' },
  { n: 53, rel: 'crates/registry/src/client.rs' },
  { n: 52, rel: 'crates/render/src/walk.rs' },
  { n: 52, rel: 'crates/realise/tests/scenarios.rs' },
  { n: 51, rel: 'crates/testsuite/tests/yaml_confinement.rs' },
  { n: 51, rel: 'crates/run/tests/executor.rs' },
  { n: 51, rel: 'crates/registry/tests/engine_rpc.rs' },
  { n: 51, rel: 'crates/receipt/tests/s2fix_claim_token.rs' },
  { n: 51, rel: 'crates/mrd/src/daemon.rs' },
  { n: 50, rel: 'crates/wire-serve/tests/s4a_conformance_toctou.rs' },
  { n: 50, rel: 'crates/run/tests/s2fix_run_receipt_fields.rs' },
  { n: 50, rel: 'crates/fs/tests/hash_domain.rs' },
  { n: 49, rel: 'crates/wire-serve/tests/u8b_plan_edits.rs' },
  { n: 49, rel: 'crates/run/tests/record.rs' },
  { n: 49, rel: 'crates/run/src/dispatch_starlark.rs' },
  { n: 49, rel: 'crates/policy/src/defs/fm.rs' },
  { n: 48, rel: 'crates/mrd/src/test_cmd.rs' },
  { n: 48, rel: 'crates/model/tests/u11r2_two_root_greys.rs' },
  { n: 47, rel: 'crates/testsuite/tests/u4a2_composed_read.rs' },
  { n: 47, rel: 'crates/model/src/gotext.rs' },
  { n: 45, rel: 'crates/registry/src/refresh.rs' },
  { n: 44, rel: 'crates/run/tests/s2fix_task_name_charset.rs' },
  { n: 44, rel: 'crates/perfsuite/src/generator.rs' },
  { n: 44, rel: 'crates/mrd/src/lib.rs' },
  { n: 43, rel: 'crates/sidecar/tests/sub_push.rs' },
  { n: 43, rel: 'crates/render/tests/u4b_elision.rs' },
  { n: 43, rel: 'crates/policy/src/defs/mod.rs' },
  { n: 43, rel: 'crates/mrd/tests/read_put_cli.rs' },
  { n: 43, rel: 'crates/mrd/tests/board_three_layers.rs' },
  { n: 42, rel: 'crates/wire-serve/src/armed_disk.rs' },
  { n: 42, rel: 'crates/mrd/tests/run_lifecycle.rs' },
  { n: 42, rel: 'crates/mrd/src/resolve.rs' },
  { n: 41, rel: 'crates/wire-serve/tests/u6_write_flock.rs' },
  { n: 41, rel: 'crates/registry/src/engine.rs' },
  { n: 41, rel: 'crates/perfsuite/src/profile.rs' },
  { n: 41, rel: 'crates/perfsuite/benches/roundtrip.rs' },
  { n: 41, rel: 'crates/mrd/tests/retired_verbs.rs' },
  { n: 40, rel: 'crates/run/src/fence.rs' },
  { n: 40, rel: 'crates/policy/src/defs/load.rs' },
  { n: 40, rel: 'crates/mrd/src/put_cmd.rs' },
  { n: 39, rel: 'crates/sidecar/tests/watch_external.rs' },
  { n: 39, rel: 'crates/registry/src/state.rs' },
  { n: 38, rel: 'crates/testsuite/tests/armed_disk_agreement.rs' },
  { n: 37, rel: 'crates/wire-serve/tests/u11_cross_root_confinement.rs' },
  { n: 37, rel: 'crates/run/tests/caps_home.rs' },
  { n: 37, rel: 'crates/mrd/tests/journal_genesis.rs' },
  { n: 36, rel: 'crates/testsuite/tests/walk_resolve.rs' },
  { n: 36, rel: 'crates/model/tests/s2fix_root_qualified_linkpath.rs' },
  { n: 35, rel: 'crates/wire-serve/tests/u11_lock_write.rs' },
  { n: 35, rel: 'crates/sidecar/tests/p4_probes.rs' },
  { n: 34, rel: 'crates/policy/tests/defs_conformance.rs' },
  { n: 33, rel: 'crates/mrd/tests/status_walltime.rs' },
  { n: 33, rel: 'crates/mrd/src/view_status.rs' },
  { n: 33, rel: 'crates/fs/tests/overlay_interval.rs' },
  { n: 32, rel: 'crates/sidecar/src/policy_bridge.rs' },
  { n: 32, rel: 'crates/run/tests/snapshot.rs' },
  { n: 32, rel: 'crates/perfsuite/src/corpus.rs' },
  { n: 32, rel: 'crates/perfsuite/benches/cold_ingest.rs' },
  { n: 32, rel: 'crates/mrd/tests/run_cli.rs' },
  { n: 31, rel: 'crates/wire-serve/tests/u5_write_conflict.rs' },
  { n: 31, rel: 'crates/cache/src/layout.rs' },
  { n: 30, rel: 'crates/run/src/contracts.rs' },
  { n: 29, rel: 'crates/sidecar/tests/selector_ambiguity.rs' },
  { n: 29, rel: 'crates/policy/src/defs/i4.rs' },
  { n: 29, rel: 'crates/mrd/src/reconcile_cmd.rs' },
  { n: 29, rel: 'crates/check/tests/orphan_production_fixture.rs' },
  { n: 28, rel: 'crates/workspace/tests/discovery.rs' },
  { n: 28, rel: 'crates/wire-serve/tests/reserved_paths.rs' },
  { n: 28, rel: 'crates/policy/src/defs/go_fmt.rs' },
  { n: 28, rel: 'crates/policy/src/defs/check.rs' },
  { n: 28, rel: 'crates/perfsuite/src/lib.rs' },
  { n: 28, rel: 'crates/effects/tests/determinism.rs' },
  { n: 27, rel: 'crates/wire-map/tests/b1_superset.rs' },
  { n: 27, rel: 'crates/testsuite/tests/delta_e3e4.rs' },
  { n: 27, rel: 'crates/registry/tests/rpc.rs' },
  { n: 27, rel: 'crates/policy/tests/purity.rs' },
  { n: 27, rel: 'crates/cache/tests/integration.rs' },
  { n: 26, rel: 'crates/run/tests/runner.rs' },
  { n: 26, rel: 'crates/perfsuite/src/report.rs' },
  { n: 25, rel: 'crates/testsuite/tests/charset_guard.rs' },
  { n: 25, rel: 'crates/testsuite/src/lib.rs' },
  { n: 25, rel: 'crates/model/tests/u11_c3_linkpath_peels_and_refuses.rs' },
  { n: 23, rel: 'crates/mrd/tests/walk_e2e.rs' },
  { n: 23, rel: 'crates/effects/tests/fuel.rs' },
  { n: 22, rel: 'crates/sidecar/tests/hook_notify.rs' },
  { n: 22, rel: 'crates/perfsuite/benches/assemble.rs' },
  { n: 21, rel: 'crates/perfsuite/tests/results_freshness.rs' },
  { n: 20, rel: 'crates/testsuite/tests/wsfix_oracle.rs' },
  { n: 20, rel: 'crates/testsuite/tests/u0_read_parity.rs' },
  { n: 20, rel: 'crates/run/tests/exec.rs' },
  { n: 20, rel: 'crates/policy/src/defs/cascade.rs' },
  { n: 20, rel: 'crates/perfsuite/benches/project.rs' },
  { n: 20, rel: 'crates/mrd/tests/pin_cause.rs' },
  { n: 20, rel: 'crates/mrd/src/unfold_cmd.rs' },
  { n: 20, rel: 'crates/mrd/src/new_cmd.rs' },
  { n: 20, rel: 'crates/effects/tests/purity.rs' },
  { n: 19, rel: 'crates/sidecar/tests/v2_compat_e2e.rs' },
  { n: 19, rel: 'crates/run/tests/caps.rs' },
  { n: 18, rel: 'crates/wire/tests/create_shape.rs' },
  { n: 18, rel: 'crates/perfsuite/src/claims.rs' },
  { n: 18, rel: 'crates/perfsuite/src/bin/corpusgen.rs' },
  { n: 18, rel: 'crates/mrd/src/gc.rs' },
  { n: 17, rel: 'crates/policy/src/defs/shape.rs' },
  { n: 17, rel: 'crates/perfsuite/src/measure.rs' },
  { n: 17, rel: 'crates/mrd/src/cache_cmd.rs' },
  { n: 17, rel: 'crates/effects/tests/hostility.rs' },
  { n: 16, rel: 'crates/view/src/schema.rs' },
  { n: 16, rel: 'crates/receipt/tests/frozen_receipts.rs' },
  { n: 16, rel: 'crates/perfsuite/benches/parse.rs' },
  { n: 16, rel: 'crates/effects/tests/robustness.rs' },
  { n: 15, rel: 'crates/run/tests/report.rs' },
  { n: 15, rel: 'crates/effects/tests/run_entry.rs' },
  { n: 15, rel: 'crates/effects/tests/golden.rs' },
  { n: 14, rel: 'crates/testsuite/tests/u4b_real_lock.rs' },
  { n: 12, rel: 'crates/testsuite/tests/u4a1_render_parity.rs' },
  { n: 12, rel: 'crates/mrd/examples/board_colors.rs' },
  { n: 11, rel: 'crates/mrd/src/preset_cmd.rs' },
  { n: 10, rel: 'crates/testsuite/tests/gt_parse.rs' },
  { n: 10, rel: 'crates/perfsuite/tests/harness.rs' },
  { n: 10, rel: 'crates/mrd/src/unregister.rs' },
  { n: 10, rel: 'crates/effects/tests/cascade.rs' },
  { n: 9, rel: 'crates/view/tests/topology_assert.rs' },
  { n: 9, rel: 'crates/sidecar/src/main.rs' },
  { n: 9, rel: 'crates/run/tests/shim.rs' },
  { n: 9, rel: 'crates/mrd/tests/preset_e2e.rs' },
  { n: 8, rel: 'crates/testsuite/tests/wire_vocab.rs' },
  { n: 8, rel: 'crates/perfsuite/tests/inventory_assert.rs' },
  { n: 7, rel: 'crates/perfsuite/src/inventory.rs' },
  { n: 7, rel: 'crates/effects/tests/capability.rs' },
  { n: 6, rel: 'crates/sidecar/tests/view_path_daemon_only.rs' },
  { n: 6, rel: 'crates/run/tests/support/mod.rs' },
  { n: 6, rel: 'crates/effects/tests/support/mod.rs' },
  { n: 5, rel: 'crates/run/tests/dispatch_starlark.rs' },
  { n: 4, rel: 'crates/run/tests/address.rs' },
  { n: 4, rel: 'crates/perfsuite/src/bin/bench-report.rs' },
  { n: 4, rel: 'crates/effects/fuzz/fuzz_targets/eval_source.rs' },
  { n: 3, rel: 'crates/testsuite/tests/main.rs' },
  { n: 3, rel: 'crates/run/tests/contracts.rs' },
  { n: 3, rel: 'crates/effects/tests/validate.rs' },
  { n: 3, rel: 'crates/effects/fuzz/fuzz_targets/eval_input.rs' },
  { n: 2, rel: 'crates/transport-proto/build.rs' },
  { n: 2, rel: 'crates/testsuite/tests/gt_pack_smoke.rs' },
  { n: 2, rel: 'crates/mrd/src/main.rs' },
]

log('comment-cleanup: ' + FILES.length + ' files, rewrite=grok-exec review=codex-exec, md root ' + MD_ROOT)

const results = await pipeline(
  FILES,
  f => agent(rewritePrompt(f), {
    agentType: 'grok-exec', model: 'sonnet', effort: 'low',
    label: 'rewrite:' + f.rel, phase: 'Rewrite', schema: REWRITE_SCHEMA,
  }).then(rw => rw ?? { status: 'failed', reason: 'rewrite agent lost (skipped or terminal API error)', changed: false, mdCreated: false, summary: '' }),
  (rw, f) => {
    if (rw.status !== 'ok') {
      return { rel: f.rel, n: f.n, rewrite: rw, review: null, verdict: 'SKIPPED' }
    }
    return agent(reviewPrompt(f, rw), {
      agentType: 'codex-exec', model: 'sonnet', effort: 'low',
      label: 'review:' + f.rel, phase: 'Review', schema: REVIEW_SCHEMA,
    }).then(rv => ({
      rel: f.rel, n: f.n, rewrite: rw,
      review: rv ?? { status: 'failed', reason: 'review agent lost', verdict: 'UNREVIEWED', detail: '' },
      verdict: rv?.verdict ?? 'UNREVIEWED',
    }))
  },
)

const done = results.filter(Boolean)
const count = v => done.filter(r => r.verdict === v).length
const summary = {
  total: FILES.length,
  go: count('GO'),
  nogo: count('NO-GO'),
  rewriteFailed: count('SKIPPED'),
  reviewFailed: count('UNREVIEWED'),
  lost: results.length - done.length,
  mdRoot: MD_ROOT,
  nogoFiles: done.filter(r => r.verdict === 'NO-GO')
    .map(r => ({ rel: r.rel, detail: r.review?.detail ?? '', md: mdRel(r.rel) })),
  failedFiles: done.filter(r => r.verdict === 'SKIPPED' || r.verdict === 'UNREVIEWED')
    .map(r => ({
      rel: r.rel,
      stage: r.verdict === 'SKIPPED' ? 'rewrite' : 'review',
      reason: (r.verdict === 'SKIPPED' ? r.rewrite?.reason : r.review?.reason) ?? '',
      sessionId: (r.verdict === 'SKIPPED' ? r.rewrite?.sessionId : r.review?.sessionId) ?? '',
    })),
  files: done.map(r => ({
    rel: r.rel, verdict: r.verdict,
    changed: r.rewrite?.changed ?? false,
    mdCreated: r.rewrite?.mdCreated ?? false,
    summary: r.rewrite?.summary ?? '',
    reviewDetail: r.review?.detail ?? '',
  })),
}

log('comment-cleanup done: ' + summary.go + ' GO / ' + summary.nogo + ' NO-GO / '
  + summary.rewriteFailed + ' rewrite-failed / ' + summary.reviewFailed + ' review-failed / '
  + summary.lost + ' lost, of ' + summary.total)

return summary
