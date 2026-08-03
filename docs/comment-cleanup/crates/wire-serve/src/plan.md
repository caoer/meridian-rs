# crates/wire-serve/src/plan.rs — comment-cleanup notes

## Module charter / fidelity law (from plan.rs:1)

M1 U8b: lower `splice.plan_edits` (the put-plan vocabulary) to NATIVE wire edits at the splice intake — the Go daemon's `buildSpliceEdit` + `buildPropertyEdits` emulation (`putsidecar.go` / `puttoc.go`), moved behind the wire and deleted host-side.

**The one law: byte-faithful to the deleted Go arms.** Every lowered native edit is byte-identical to the edit the host used to build from its `toc`+`cat` view, so everything DOWNSTREAM (model validate, CAS, armed facts, reparse gate, `would_corrupt`) behaves identically by construction. Fidelity points, each deliberate:

- **Address resolution = the host `tocIndex` MAP**, not the read face and not the `check_write` rebuild: exact sanitized-hpath-chain lookup, LAST occurrence wins on duplicates (Go map overwrite), NO raw-title fallback. (Duplicate/ambiguous targets never reach here through the put face — `check_write`'s rebuild refuses `E_AMBIGUOUS` first, host-sequenced.)
- **Append discipline** reads the PRE-batch bytes only (`fileBytes[at-1]`), with NO in-batch pending-newline tracking — the host emulation had none (`policy::defs::rebuild`'s `pending_nl` is the `check_write` candidate's lib-faithful twin; two appends to one section refuse as overlap anyway).
- **`create` = PARENT-APPEND** (`"\n" + "#"*level + " " + title + "\n\n" + body + "\n"`, `Put{end}` on the parent) — emulation-faithful. The `check_write` candidate places creates at EOF (`rebuild::plan_create_section`, lib-faithful): the NAMED U8b residual (G-class, C rider 3) — invisible while no def governs created sections; stage-2 unifies if one ever does.
- **`set_property` = the property-group dance**, NOT native `at:upsert`: model's upsert inserts an absent key at FIRST-key position (`plan_fm_upsert`), the Go dance inserts after the LAST key — divergent bytes. BOTH halves of the composed line pass the ONE shared owner — keys through `policy::defs::yaml_safe_key`, values through `policy::defs::yaml_safe_value` — so the `check_write` candidate and the written bytes cannot drift, and neither door can forge frontmatter.
- **Refusals**: `bad_request` + `message` = the exact Go-face teaching MINUS the `put: ` verb prefix (the host renders the prefix; op-owner ruling 2026-07-24 — engine owns the sentence, host owns the verb). `%q` spellings mint through `policy::defs::go_quote`.

## lower / edit order (from plan.rs:106)

Properties FIRST as one group, then body ops in request order (the host's edit order, verbatim). The returned batch feeds the unchanged native splice path; armed facts align 1:1 with the LOWERED edits. Go map semantics on the props group: last value per key wins, keys sorted (BTreeMap).

## lower_append (from plan.rs:173)

`buildSpliceEdit` append arm (`putsidecar.go:202-226`): block targets refuse; the payload gets `ensureTrailingNL` + a leading `\n` when the pre-batch byte before the insertion point is not one. Lowered to `Put{end}` on the RAW hpath. (Dedupe is the HOST's — a deduped plan is never sent.)

## lower_match / S10 needle peel (from plan.rs:213)

`buildSpliceEdit` replace arm (`putsidecar.go:244-275`): unique-anchor `Match{old,new}`, or the `all:true` read-modify-write — every occurrence replaced over the heading-EXCLUDED content, written back `Put{content}`. (`foreign_changes` is journal-derived and stays with the HOST.)

S10's ADDRESS half (advisor R25): `old` is a NEEDLE searched in STORED bytes, which never carry an `@fp` token, so an agent's needle copied from the decorated render face must be peeled before the search — the same rule `read::to_model_ref` applies to a `SecRef::Anchor`. It is peeled HERE because the search happens here; `new` is a PAYLOAD and rides verbatim into the candidate, where the document-grain strip owns it.

Go stripHeading: content = span bytes from the content-span offset (defensive fallbacks verbatim: no content span, or a malformed offset, serve the FULL span bytes).

## lower_replace_section (from plan.rs:283)

`buildSpliceEdit` `replace_section` arm (`putsidecar.go:228-242`): requires a rev (destructive), payload `ensureTrailingNL` (empty stays empty), lowered to `Put{content}` + `if_node_rev`. The empty-rev refusal is DEAD through the put face (`check_write`'s ECAS fires first, host-sequenced) — minted arm-faithful for direct wire callers.

## lower_create (from plan.rs:324)

`createTarget` + the create arm (`puttoc.go:125-139`, `putsidecar.go:277-286`): PARENT-APPEND emulation — `"\n" + "#"*(parent.level+1) + " " + title + "\n\n" + body + "\n"` as `Put{end}` on the parent. Top-level (`parent_hpath` empty) and parent-miss refuse with the ONE host teaching, naming the full target as the caller spelled it. The named EOF-vs-parent residual lives in `check_write`'s candidate, not here.

## lower_property_group (from plan.rs:366)

`buildPropertyEdits` (`puttoc.go:147-209`), byte-faithful: each existing key being set is a whole-line `Put{all}` on its `fm_key`; absent keys land as `"\n{k}: {v}"` lines AFTER the last existing key — folded into that key's `Put{all}` when it is itself being set (the carrier), else a `Put{end}` after it. No frontmatter to anchor on refuses the host teaching.

BOTH halves of the composed `{key}: {value}` line pass their ONE shared fallible owner before any byte is built: the key through `yaml_safe_key` (charset — the pre-flight's own refusal, `rebuild::plan_set_property`), the value through the conditional-quote predicate, which REFUSES a multi-line value (D11).

The KEY owner runs first, in the pre-flight's ordering: an unvalidated key is composed into the frontmatter line exactly as an unvalidated value is, so this door must refuse what `rebuild::plan_set_property` refuses.

D11: the composed line is `{key}: {value}`, so a newline in the value forges frontmatter keys — and a single-quoted YAML scalar cannot escape one. Refuse, never sanitize; the shared predicate owns the law.

pkg/body inserts a new key immediately before the closing `---` — the END of frontmatter, after the LAST key. If the last key is itself being set, FOLD the inserts into its Put{all} (one disjoint edit); otherwise Put{end} after it.

Empty value composes the host's `"k: "` line (trailing space) — the dance's `line(k)` verbatim, never model-upsert's `"k:"`.

## Test pins (from plan.rs tests)

- Append discipline (`putsidecar.go:219-223`); U8b MUST-CARRY pins against goldens (`p-replace-on-block`, `p-create-top`).
- Block-append and empty-rev replace_section arms are dead through the put face (`check_write` fires first) but arm-faithful for direct wire callers.
- match all:true content span starts at the byte AFTER the heading line, so the blank separator line rides the content — exactly Go stripHeading.
- Property dance: existing alone → Put{all}; absent last-not-set → Put{end}; absent+last-set → carrier fold (one Put{all} + other existing Put{all}).
- No-frontmatter refuse is dead through put face (`E_NO_MATCH` first).
- Conditional quote rides the ONE shared predicate (`": "`-bearing value lands single-quoted as check_write candidate).
- Duplicate sanitized chains: LAST wins (unreachable through put face: E_AMBIGUOUS first). Raw-title addresses MISS (host map keys sanitized only — no title fallback, unlike check_write rebuild resolve).

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Deleted protocol invariants, refusal ordering, byte-shape rules, and test rationale survive in the notes or concise comments.
- style: pass — Remaining comments are concise and accurate; no narration, commented-out code, or unsafe blocks remain.
- detail: Host-map last-wins behavior, pre-batch append discipline, S10 peel/strip rules, create residual, D11 safety rule, property carrier folding, and direct-caller dead-arm context are preserved.
