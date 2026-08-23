//! The `mrd` CLI. Owns hand-rolled subcommand parsing (no clap), the per-invocation resolution
//! flow ([`resolve`]), and the client side of the resident daemon ([`engine`]: auto-spawn on
//! first use; reads degrade to an in-process ephemeral engine — decision 0002 §3; writes
//! never degrade — [`write_ipc`]).

use std::path::PathBuf;
use std::process::ExitCode;

mod cache_cmd;
mod check_cmd;
mod config_cmd;
mod corpus_tier;
mod daemon;
mod engine;
mod fingerprint_cmd;
mod gc;
mod history_cmd;
// The commit-fence READ plane. Public because the design tests for `mrd skill hook` hold the
// emitted document to this module's constants. Nothing here writes.
mod arm_cmd;
pub mod hook;
mod init;
mod new_cmd;
mod path_law;
mod pin_cmd;
mod preset_cmd;
mod put_cmd;
mod read_cmd;
mod realise_cmd;
mod reconcile_cmd;
mod repair_cmd;
mod resolve;
mod rm_cmd;
mod rooted;
// The type-2 retirement DSL. Public because the coverage census asserts that the set of reason
// words its fixtures exercise is the set `Reason::ALL` declares.
pub mod retire_cmd;
mod rules_cmd;
/// The rules walk: the disk edge that enumerates the scope ladder's roots and offers every page
/// in their hash domain to tag-indexed registration. `policy` stays I/O-free; this feeds it.
pub mod rules_walk;
mod run_cmd;
/// The script entry's consumer plane (kernel entry #3). Public because the
/// `ScriptTrace` it assembles IS the boundary contract `mrd script --json`
/// emits — the design tests hold the shape, and U4's wire client fills it.
pub mod script;
mod skill_cmd;
mod sql;
mod status_cmd;
mod test_cmd;
mod unfold_cmd;
mod unregister;
mod walk_cmd;
/// Routine CLI writes: authenticated IPC to the workspace authority. No
/// direct-publication fallback.
mod write_ipc;

/// Exit code: a clean success.
const EXIT_OK: u8 = 0;
/// Exit code: findings — a verb ran cleanly but reported failures (e.g. `mrd
/// test` scenarios whose `^expect` did not hold).
const EXIT_FINDINGS: u8 = 1;
/// Exit code: a tool failure — bad usage, a refused deny ceiling, an I/O error,
/// or a structural fault (a malformed scenario, a pairing hard error).
const EXIT_FAIL: u8 = 2;

/// The unserved-member voice, shared by every CLI face that builds the corpus
/// in-process: one line per hash-domain member [`fs::build_corpus`] could not
/// serve (its third slot), on stderr so machine stdout stays clean. The wording
/// mirrors the daemon's per-file `invalid_utf8` teaching (registry
/// `doc_or_refusal`), plus the one fact a SCANNING face owes its operator: this
/// scan never saw inside the member. A face that discards the slot goes
/// silently blind — its scan over a partial corpus reads as a scan of the
/// whole vault. `mrd retire` deliberately does NOT voice through here: a sweep
/// that certifies absence REFUSES on an unserved member instead
/// (`retire_cmd::Reason::MemberUnserved`).
pub(crate) fn voice_unserved(unserved: &std::collections::BTreeMap<String, String>) {
    for (member, condition) in unserved {
        eprintln!(
            "mrd: warning: {member} {condition} — the file serves no spans/nodes; its bytes stay under the root; this scan does not see inside it"
        );
    }
}

/// The domain-excluded voice, shared by every in-process face that ENUMERATES
/// the corpus: one line naming the markdown the domain DECLINES yet the vault
/// still shows — the custom-ignore class, exactly — on stderr so machine
/// stdout stays clean.
///
/// Twin of [`voice_unserved`], and for the same reason: an enumeration's answer
/// is stamped `as_of` a fingerprint that an out-of-domain file's bytes cannot
/// move, so the row is rightly absent — but a scan that reports a partial
/// population as if it were the whole one reads as a scan of the whole vault.
/// An enumerator MAY exclude what its attestation cannot reach; it may never
/// exclude SILENTLY (session decision 0017; `docs/wire-contract.md` §12.1,
/// enumerator clause). Doors asked about ONE named path do not voice here —
/// they serve the path instead (`walk_cmd::admit_named_page`); the `docs`
/// filter below is that discipline's other half, keeping an admitted named
/// page out of its own door's voice.
///
/// The enumeration is [`fs::declined_markdown`] — the projection's own walk
/// law, one shared predicate ([`fs::domain::dot_segment`], §12.1 rule 2), a
/// dot-prefixed segment never entered and never voiced — so this voice can
/// never name a path the record projection refuses to serve (dogfood F11,
/// carried from `mrd rules` to this face; card
/// voice-excluded-walk-consistency). The complete outside-domain enumeration,
/// dot class included, stays on the machine channel: the `excluded` key of
/// bare `mrd links --json` (§4.6).
///
/// ⚠️ **The PROSE sample is capped at [`EXCLUDED_SHOWN`]; the COUNT never is,
/// and the wire's `excluded` key stays complete.** Uncapped, this line is a
/// weapon rather than a courtesy: it is voiced BEFORE the door answers, so a
/// call that ends up FAILING has already emitted it, and a consumer that folds
/// stderr into an error payload hands its caller the whole enumeration on the
/// retry path — measured 2026-08-10 on the newly registered mcp face, where one
/// failed `walk` returned **3,171,117 characters enumerating 28,936 files**,
/// enough to destroy an agent's context on a call that walked nothing. The
/// anti-silence law is satisfied by the COUNT plus a sample plus the pointer to
/// the complete machine-readable list; it never required prose to be unbounded.
/// Cap convention borrowed from [`model::selector::NEAREST_SHOWN`], which this
/// site should have followed from the start.
pub(crate) fn voice_excluded(root: &fs::WorkspaceRoot, docs: &model::Docs) {
    let Ok(declined) = fs::declined_markdown(root) else {
        return;
    };
    let excluded: Vec<String> = declined
        .into_iter()
        .filter(|rel| !docs.contains_key(rel))
        .collect();
    voice_excluded_note(&excluded);
}

/// Print the capped note for an already-enumerated declined-class population:
/// full count, [`EXCLUDED_SHOWN`] sample, remainder clause, machine pointer.
/// One spelling for every face that voices this note — the in-process
/// enumerators above and the `links` face, whose population rides its answer's
/// own `excluded` key rather than a walk (`engine::voice_excluded`). Silent on
/// an empty population.
pub(crate) fn voice_excluded_note(excluded: &[String]) {
    if excluded.is_empty() {
        return;
    }
    eprintln!(
        "{}",
        excluded_note(excluded.len(), &capped_sample(excluded))
    );
}

/// The house cap spelling for a prose list of paths: at most [`EXCLUDED_SHOWN`]
/// names, then the remainder clause. One spelling for every human face that
/// names a population it also counts — the domain-excluded note above and the
/// `mrd retire` human render, whose enumeration is lawfully COMPLETE on its
/// `--json` (`files_excluded`, the certify-absence contract) and therefore
/// needs the cap on the prose only (card retire-cmd-cap-join).
///
/// The remainder clause exists so the sample can never READ as the whole list:
/// "a, b, c" and "a, b, c and 28933 more" are the same three names and opposite
/// claims about the population. The COUNT is the caller's to print, never
/// capped — that is the half that keeps exclusion non-silent (decision 0017).
pub(crate) fn capped_sample(names: &[String]) -> String {
    let shown: Vec<&str> = names
        .iter()
        .take(EXCLUDED_SHOWN)
        .map(String::as_str)
        .collect();
    let rest = names.len().saturating_sub(shown.len());
    if rest == 0 {
        shown.join(", ")
    } else {
        format!("{} and {rest} more", shown.join(", "))
    }
}

/// The one-line domain-excluded note [`voice_excluded`] prints. Extracted so a
/// test can hold the note's promise against the machine answers it speaks for:
/// none of the in-process faces that voice it (sql, walk, check, repair) carry
/// an `excluded` key in their own `--json` output, so the pointer names the one
/// carrier that really serves the complete list — the bare `mrd links --json`
/// enumeration (§4.6, §12.1) — never "the machine answer" of the calling verb.
/// The `links` face itself voices this same note (card walk-law-audit), where
/// the pointer is self-referential by design: the complete list is that verb's
/// own `--json` answer.
pub(crate) fn excluded_note(count: usize, sample: &str) -> String {
    format!(
        "mrd: note: {count} markdown file(s) under this root are outside the hash domain and are NOT in this listing — {sample}. The complete list is the `excluded` key of `mrd links --json` (§12.1). They are addressable by explicit path (`mrd read`, `mrd links <PATH>`); their bytes do not move the fingerprint this answer is stamped with."
    )
}

/// How many excluded paths the PROSE note names before it defers to the wire.
///
/// Three, matching [`model::selector::NEAREST_SHOWN`] — the tree's existing
/// answer to "how many is enough for a human to recognise the KIND of thing
/// being excluded". A reader who needs the population reads the count; a reader
/// who needs the members reads the machine answer.
const EXCLUDED_SHOWN: usize = 3;

/// The title line and the gutter legend. Held apart from [`LISTING`] so the legend, which is
/// prose and not a verb block, can never be lexed as one.
const HEADER: &str = "\
mrd — the meridian workspace CLI
  `!` in the gutter marks a verb that CHANGES something — files or the drawer.
  Every unmarked verb is a read.
  MRD_TIMING=1 (or a file path) adds `mrd-timing` phase lines on stderr, for
  ANY verb: time cost only, nothing else — stdout and the exit code are
  unchanged. Off, no clock is read. `docs/status.md` § The timing mode.
";

/// The verb listing and the options block: the single source for `mrd --help` and every
/// `<verb> --help`, which is lexed back out of this text.
const LISTING: &str = "\
usage:
! mrd init [PATH] [--name NAME]
                           declare the root: write PATH's MERIDIAN.md
                           (type: meridian-root; name = dir unless --name),
                           register the drawer, reconcile shadowed descendant
                           drawers. Declaration does NOT anchor the resolution
                           ladder — the report also names the tier/root PATH
                           resolves to. Valid existing declaration left
                           byte-for-byte; present-but-invalid MERIDIAN.md
                           refuses (exit 2).
! mrd unregister [PATH]    drop the daemon entry (if a daemon answers) and the
                           workspace drawer. A PATH whose directory is already
                           gone is matched as given — the stale entry a sweep
                           leaves behind. Keyed by nothing, it refuses (exit 2)
                           instead of reporting a removal that did not happen.
  mrd resolve [PATH]       how PATH resolves: the tier that answered and the
                           root it named.
  mrd links [PATH]         corpus edge map (whole corpus, or one file). Daemon
                           (auto-spawned) or in-process.
  mrd read <PATH>[#FRAG] [--section SEL]
                           composed read at ONE engine snapshot (daemon or
                           in-process). PATH is the agent-plane `[root:]path`
                           spelling (address-grammar §4.1): a rooted ref reads
                           inside the named root's workspace from the machine
                           mount table (~/MERIDIAN.md, fresh per call) — the
                           same name→workspace binding MCP serves. The root
                           reading wins on a head colon, never a literal path:
                           a typo'd or unbound root refuses (exit 1) naming
                           the bound roots, and never falls back to an ambient
                           lookup. The rooted lane spans every door that names
                           a page (address-grammar §4.6): read, fingerprint,
                           resolve, links, walk, repair, realise, run, rules,
                           put (TARGET and --scope), rm, pin (PAGE; TARGET was
                           already cross-root), script --files — one
                           resolution seam. A rooted op runs under the NAMED
                           root: conventions and receipts follow the page
                           tree, never the cwd. unfold/reconcile/new refuse a
                           rooted ref for now (their writes run in-process,
                           where the named root's armed gates would not fire).
                           No --section = section map alone (dewey
                           n, depth, title, words, sec_rev) under the read's
                           fingerprint (the fp put --if-fingerprint takes).
                           --section (repeatable: heading path, dewey, or
                           ^anchor) serves bodies; each body opens with == n ==
                           and the head declares its byte length. WHOLE-FILE
                           READ IS `--section 1`: the root selector serves its
                           descendants too, so it is the one whole-document
                           form. There is no --all/--full, and naming every
                           section instead DUPLICATES the nested bodies. Human:
                           rendered text. --json toc: structured toc[] only, no
                           rendered_text. READ BUDGET: one section read serves
                           at most 20000 WORDS and names at most 64 DISTINCT
                           --section selectors. Over either the read REFUSES,
                           never truncates. The word ceiling is priced in the
                           map you already have: a bare read lists every section
                           with its own words, so pick what fits before asking.
                           Repeated identical selectors are served ONCE and the
                           collapse is stated. The section map itself is never
                           word-bounded — it is the way back in. Under the
                           scoped-guards cap a --json read also captures the
                           file's scoped token (fingerprint {scope} on the
                           same connection) as the frame's mint key; the
                           body's fingerprint stays the world token. Exits: 0 served
                           / 1 engine refused / 2 bad invocation.
  mrd fingerprint [PATH | --scope-bytes B64] [--json]
                           the standalone §4.7 mint door. Bare: the §5.1
                           world token (v2-identical, no cap needed). PATH:
                           the named node's scoped token — workspace root,
                           folder, or file leaf (nodes `mrd read` cannot
                           serve mint here). A `#` fragment on PATH refuses
                           at path grain (exit 1), rooted and ambient alike:
                           a mint binds a node, never a section — a name
                           carrying a literal `#` mints via --scope-bytes.
                           --scope-bytes B64 = base64url
                           over the raw path bytes, for names the UTF-8
                           Path noun cannot carry (§5.4). Exactly one
                           spelling — a mint names ONE node; both refuse at
                           parse. Scoped arms ride only when hello serves
                           scoped-guards (taught refusal otherwise, nothing
                           sent). Answers {fingerprint, seq,
                           scope|scope_bytes}, the request's spelling echoed
                           beside the token; a lawful path with no node
                           answers the reserved token `absent` (§5.6).
                           --json: {workspace, mint} served, {workspace,
                           error} refused. Exits: 0 minted / 1 engine
                           refused / 2 bad invocation.
! mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
          [--if-fingerprint FP] [--scope PATH | --scope-bytes B64]
          [--receipt PATH#ANCHOR] [--field K=V]... [--json]
                           batch write. STDIN = BARE JSON array
                           [{target, edit, if_node_rev?}] — the VALUE of
                           wire §4.4 edits, NEVER the full request object
                           (no id/op/path; those are argv).
                           --field K=V (repeatable) = the § A.2.1 opaque
                           middleware passthrough (ctx.fields); needs the
                           daemon's splice.fields cap, refused client-side
                           before any write when absent.
                           target: {\"hpath\":[{\"h\":\"Raw Title\"},...]} — the raw
                           heading path mrd read publishes ({\"n\":2} only on a
                           duplicate) / {\"anchor\":\"id\"} / {\"fm_key\":\"key\"}.
                           edit nested, never a bare string:
                           {\"match\":{\"old\":\"…\",\"new\":\"…\"}} (one occurrence)
                           or {\"put\":{\"at\":\"end|all|content|upsert\",
                           \"text\":\"…\"}}. A working batch, whole:
                           [{\"target\":{\"hpath\":[{\"h\":\"Title\"}]},
                             \"edit\":{\"match\":{\"old\":\"a\",\"new\":\"b\"}}}]
                           Routed to the running daemon over authenticated
                           IPC (CAS + armed gate). No direct-write
                           fallback — the daemon must come up (auto-spawn,
                           or `mrd daemon`). A guardless put is a wire
                           client: fingerprint-or-force applies (pass
                           --force or if_node_rev). --dry and --validate =
                           one daemon rehearsal (no disk): --dry prints
                           the rehearsal summary; --validate is exit-only.
                           --force escapes armed binding-break/block and
                           the wire guard (skip shown in verdict).
                           --if-fingerprint = world-grain guard.
                           --scope PATH pairs it to that node (sent only
                           when hello serves scoped-guards; taught
                           refusal otherwise, nothing sent). --scope takes
                           [root:]path (§4.1): a rooted scope is accepted
                           when the named root binds the workspace this
                           put writes — a rooted mint's echo pastes beside
                           its token; the wire carries the rel half. A
                           root binding elsewhere, or an unbound root,
                           refuses naming the fault (exit 1), never as
                           premise coverage.
                           --scope-bytes B64 = the same premise for a
                           raw-byte node name (base64url over the raw
                           path bytes, §5.4); rides the wire as one
                           guards[] entry carrying the token — never a
                           top-level splice field. Exactly one of
                           --scope/--scope-bytes. --json
                           machine face on both legs: commit
                           {workspace,put}; refusal {workspace,error} on
                           stdout. Exits: 0 committed|rehearsal-ok / 1
                           refused / 2 bad invocation.
! mrd rm <PAGE> --rev <FILE_REV> [--if-fingerprint FP] [--dry] [--actor A]
         [--now T] [--json]
                           guarded file death (§ A.3 remove door) — the write
                           model's third mutation beside new (birth) and put
                           (edit). --rev = the page's whole-file rev from a
                           prior read (remove-what-you-read; REQUIRED — the
                           engine demands it from every origin). Refuses while
                           anything still references the page: inbound
                           wikilinks, embeds, and ambient meridian-lock pins,
                           checked by the daemon; the refusal names
                           every referring file, edge kind, and count. NO
                           --force exists on this door. --if-fingerprint =
                           optional world-grain guard. --dry rehearses
                           everything except disk. Routed over IPC like
                           put — no direct-write fallback. Exits: 0
                           removed|dry / 1 refused / 2 bad invocation.
! mrd pin <PAGE> <TARGET>#<SELECTOR> [--fingerprint TOKEN] [--vibe] [--dry]
          [--json]
                           attest: record in PAGE's meridian-lock that it draws
                           from TARGET#SELECTOR at that section's content
                           fingerprint, and mint a stable ^block-id on the
                           target. PAGE draws (A pins B); SELECTOR is a
                           sanitized heading path or ^id (same grammar as mrd
                           read). Lock write rides the daemon splice (IPC,
                           no direct-write fallback) with the page content
                           (one commit). --vibe also writes the target blob
                           into git's object store so the pin is retrievable
                           before any commit references it. --fingerprint =
                           the § A.3 proof of read; a supplied token is
                           always verified (trust excuses absence, never a
                           wrong token). Exits: 0 pinned|dry / 1 refused /
                           2 bad invocation.
! mrd repair [PAGE] [--dry] [--json]
                           lost-pin repair via git history. LOST = live target
                           no longer verifies the fingerprint AND git no longer
                           holds the recorded blob (red pin with blob still
                           held = ordinary drift, not touched). A version whose
                           content matches the pin's fingerprint is restored by
                           rewriting only the pin's hash to that blob —
                           object/selector/fingerprint never rewritten, so true
                           drift stays red. No matching version = TRUE LOSS
                           (reported, never invented). Pins on another root:
                           outside jurisdiction (count stated). --dry runs the
                           walk, skips the lock write. Exits: 0
                           none-lost|all-repaired|dry / 1 TRUE LOSS / 2 bad
                           invocation.
! mrd retire <report|mark> [--id ID] [--dry-run] [--expect-root ROOT]
                           type-2 retirement DSL: sweep
                           `~~term~~ replacer (retired: ID)` markers over terms in
                           meridian-retire blocks, then report. Marker carries
                           an opaque KEY, never an address; the array-hpath
                           link lives once in the block. mark is idempotent
                           (second run writes nothing, fp byte-identical, still
                           prints count). mark
                           REQUIRES --expect-root (file-set world guard) unless
                           --dry-run; quiesce fleet + commit vault first.
                           report
                           labels measured vs declared, never inspects a test.
                           Exits: 0 clean / 1 refusal or open retirement / 2
                           bad
                           invocation.
  mrd walk <PAGE> [--down] [--depth N]
                           pin-graph context assembly. up (default) = what PAGE
                           draws from; --down = who pins PAGE + blast radius
                           (--depth 1 = direct). Every answer cites the revs it
                           read. Exits: 0 clean / 1 red edge / 2 bad invocation
                           or in-snapshot cycle.
  mrd rules [PATH] [--workspace | --user] [--json]
                           effective rules at PATH (default cwd) after id-based
                           override. Per id: winning page (rev + scope), then
                           pages it SHADOWS (winner first; never collapsed).
                           Ladder: user
                           space (MERIDIAN.md-anchored) → workspace root →
                           folder/session; narrowed to PATH's chain (sibling
                           same-id is no conflict). armed= is a separate column
                           from the
                           attested armed set (not recomputed): armed=- |
                           armed=<mode> | armed=<mode>@<page> (freeze: arming
                           pins resolution); (drifted)/(missing) when the
                           pinned page no
                           longer stands. Collision at one scope on one chain =
                           REFUSED naming every tied page. --workspace / --user
                           print that layer alone. Exits: 0 clean /
                           1 finding (collision | refused rule page | red armed row)
                           / 2 bad invocation or PATH outside workspace.
! mrd arm <ID> --mode <off|warn|block|armed> --rev <16HEX> [--at DIR] [--json]
                           the ARM act (the attest path): resolve ID at the arm
                           root (--at, a workspace-relative DIRECTORY, default
                           `.` = the workspace root — never an absolute path,
                           never the resolver's layer:depth spelling), admit
                           the attestation only if the live page rev equals
                           --rev, and pin the winner into
                           meridian/armed-rules.md — creating the once-armed
                           marker on the first arm. --rev is the attestation:
                           the rev the reviewer READ (`mrd rules` winner rev=;
                           `mrd read` serves the page); no live-rev default. A
                           check arms off|warn|block, a hook off|armed. Re-arm
                           on the same (id, arm root): identical row = no-op
                           \"unchanged\"; differing row REPLACED (the re-arm
                           every drift refusal commands). Corrupt existing
                           artifact refuses, byte-untouched. Exits: 0 armed |
                           unchanged / 1 refused (arm fault | drift | corrupt
                           artifact | busy lock) / 2 bad invocation.
  mrd config               MERIDIAN.md config plane: resolve bootstrap
                           (MERIDIAN_CONFIG, then $HOME/MERIDIAN.md) and print
                           path, state, origin, rev/fingerprint, BOUND mount
                           table (canonical/vault/path + root state), and
                           declared tools. This verb PUBLISHES the mount table
                           (render face elides meridian-* blocks, so mrd read
                           shows prose only). Exits: 0 every root bound / 1
                           config or root refused / 2 bad invocation.
  mrd check [--core] [--staged] [--commit-gate [--require-pins]] [--json]
                           pure READ validity (what lies?). Layer-0 core: claim
                           plane (pinned content drift) + retrieval plane
                           (pinned blob
                           durably anchored). WRITE HISTORY is NOT assessed
                           (NOT CHECKED, never grey) — engine keeps no memory;
                           history is git at lock. Green means the world still
                           matches the pins, not how it got there.
                           --commit-gate --require-pins refuses a pinless
                           corpus (opt-in; default pinless PASSES as vacuously
                           true). Grey pin / unaskable object store fails
                           CLOSED either way. Exits: 0 green / 1
                           finding|grey|no-pin-coverage under --require-pins /
                           2 bad invocation.
  mrd skill hook           EMIT the commit-fence contract to stdout
                           only: the markdown IS the contract (doors;
                           fence body runs mrd check --commit-gate;
                           MRD_HOOK_FORCE; generation; when to REFUSE to
                           place; how to verify). The READER places it —
                           this verb writes no file, reads no git dir,
                           resolves no workspace. mrd check reports what
                           a checkout is actually fenced by, on its
                           fence: line. There is no --json face — the document is markdown.
                           Exits: 0 document on stdout / 2 bad invocation.
  mrd cache ls             list registered drawers.
! mrd cache clean [--all]  reap stale / orphaned / retired drawers (--all:
                           every drawer).
  mrd sql <query> [--fresh] [--json] [--rebuild] [--cwd PATH | --root NAME]
                           SQL over the corpus projection (honest-tense
                           freshness frame), served from the drawer's
                           append-only sql.duckdb cache when a cache root
                           resolves, else an ephemeral in-memory build.
                           --rebuild recreates the cache file (repair verb).
                           --root NAME selects the projection workspace by
                           canonical root name from the machine mount table
                           (§4.6 addendum — the cwd plays no part); an unbound
                           name refuses naming the bound roots. One of
                           --root/--cwd.
  mrd status [--cwd PATH]  pure-local drift + freshness: rules line (N
                           armed · M drifted · forced-since-realise:
                           not-tracked — the engine keeps no memory), then the
                           five-axis line (pin · lock · anchor · armed ·
                           vibe-debt). O(armed), fetch-less. Exits: 0 clean /
                           1 finding / 2 bad invocation.
! mrd daemon               run the registry daemon in the foreground.
  mrd test --corpus <SPEC> tier-2 corpus runner: drive CHECK/HOOK rules over
                           SYNTHETIC changes; report fire-where-expected, zero
                           dead rules, fuel+heap p50/p99, FIX/HOOK quiescence.
                           Exits: 0 clean / 1 mismatch|dead|budget|quiescence /
                           2 bad spec.
  mrd test --history WORKSPACE --rule PAGE [--spec PAGE]
                           history tier — History is git: replay WORKSPACE git
                           history (<commit>:<path> rows from commit vs first
                           parent), rebuild docs, compare PAGE CHECK refusals
                           to --spec's ```golden fence (HOOK-only page refuses
                           zero). --spec's rule: must resolve to PAGE; omit =
                           declare nothing. Undeclared would-refuse fails;
                           unreconstructable rows are grey. Exits: 0 clean / 1
                           undeclared would-refuse / 2 tool failure.
! mrd run <PAGE> [TASK] [-- ARGS]
                           run a task block from page frontmatter (task.<name>;
                           PAGE workspace-relative). TASK omitted: one declared
                           task runs; several run the one named default, else
                           list and exit 2. Exits: 0 clean / 1 refused|failed /
                           2 bad invocation.
! mrd script [--files PATH]... [--args JSON] [--dry] [--actor A] [--now T]
          [--if-fingerprint FP] [--expect-armed DIGEST] [--receipt PATH#ANCHOR]
                           evaluate inline Starlark from STDIN as the caller
                           and commit what it arms. Top level IS the program:
                           read(PATH[, section=]), put(PATH,
                           props=|section=,append=) arms wire plan edits; ONE
                           guarded splice applies them. THE COMMIT'S AUTHORITY
                           IS THE TOUCH SET — the nodes this attempt actually
                           touched (what it read, what a pattern expanded to,
                           what it armed), verified entry-vs-live at exactly
                           those nodes. A foreign write OUTSIDE that set does
                           NOT refuse; one INSIDE it refuses
                           fingerprint_mismatch naming the moved scope, and
                           nothing lands. NEEDS A DAEMON: this door writes AS
                           you through the one socket, so there is no
                           daemonless leg — with none running it auto-spawns
                           one and waits for it to bind; if that never happens
                           it refuses by name and nothing is evaluated. READ
                           BUDGET: 64 read() CALLS per attempt — NOT 64 files.
                           A section read spends one like any other, so a file
                           taken as toc+N sections spends 1+N and three files
                           can exhaust it. Over the budget the run REFUSES,
                           never truncates. The pinned fingerprint is
                           guaranteed to that budget; above it, compose runs —
                           each run's commit answers for its OWN touch set, so
                           unrelated corpus churn between them is not a reason
                           to re-run; only a move inside a run's own touch set
                           is. Single attempt (retry is the
                           caller's). --dry rehearses; --json emits the trace —
                           and THE TRACE CARRIES WHAT YOU READ: each row is
                           {kind, line, path, face}, so a read-only script is
                           read THERE. There is no print(); a script that only
                           reads exits `no_effect`, which reports that it armed
                           nothing, NOT that it did nothing. BY DESIGN (D-04)
                           this door has no --scope/--scope-bytes: the
                           caller's --if-fingerprint is the world-grain entry
                           token and stays a WIDENING guard (strictest wins),
                           never a requirement (R3); the finer grain is the
                           engine's own automatic touch-set premises, which
                           need no flag.
                           Exits: 0 committed|nothing-armed / 1
                           conflict|fault|refusal / 2 bad invocation.
! mrd new <KIND> <ID> [--dry] [--actor A] [--now T]
                           file birth: resolve def (presets/<KIND>.md or page
                           path), fill ^template, validate against ^properties,
                           birth first rev via guarded create (inline birth
                           receipt). Invalid def → def_invalid; occupied →
                           cas_mismatch. Exits: 0 born|dry / 1 refused / 2 bad
                           invocation.
! mrd unfold <PRESET> [--dry] [--actor A] [--now T]
                           materialize preset scaffold: each # Unfold file via
                           guarded create (birth receipt); existing path
                           refuses if_absent CAS, byte-untouched. Exits: 0 all
                           born|dry / 1 path existed / 2 bad invocation.
! mrd reconcile <PRESET> [--prune] [--dry] [--actor A] [--now T]
                           reconcile tree to preset scaffold: materialize all
                           missing declared paths (guarded create). --prune
                           removes only declared-ephemeral files + empty
                           undeclared dirs; undeclared content = findings,
                           never pruned. Exits: 0 converged|dry / 1 finding / 2
                           bad invocation.
! mrd realise <PAGE> [--dry]
                           reconciliation loop: observe → check → apply (only
                           on drift, once) → re-check over the page's declared
                           claim (realise.field/expected + realise.apply).
                           Apply rides mrd run. Terminal: converged /
                           drifted-fixed / non-convergent / pending-agent.
                           Exits: 0 converged|drifted-fixed|dry / 1
                           non-convergent|pending-agent / 2 bad invocation.

options:
  --json                   emit JSON instead of a human table.
  --env KEY=VALUE          (run) supply one declared env entry (repeatable).
  --dry                    (run) starlark: evaluate hermetically, print full
                           effect set, apply nothing; bash: show block + caps,
                           refuse to exec.
  --list                   (run) list the page's tasks with contracts and caps.
  --files PATH             (script) one host-enumerated path, bound inert as
                           files (repeatable). Paths only — content enters
                           through read() alone. A member containing * is a
                           pattern: the attempt forwards through the daemon's
                           script op and the ENGINE expands it against the
                           entry world (recorded in the trace; zero matches
                           is data, not a refusal).
  --args JSON              (script) JSON object of strings, bound inert as the
                           args dict.
  --fingerprint TOKEN      (pin) proof of read (§ A.3): the pinned section's
                           own fp1.… token, from a sections read. Optional on
                           the trusted CLI door — a supplied token is still
                           verified against the live bytes; a wrong one
                           refuses pin_proof_required, nothing written.
  --if-fingerprint FP      (script, put, pin) world-grain guard: refuse unless
                           the workspace still stands at FP.
  --scope PATH             (put) narrows --if-fingerprint to PATH; pair
                           required. Takes [root:]path (§4.1): a rooted scope
                           is accepted when the named root binds the workspace
                           the put writes. Sent only when hello serves
                           scoped-guards; otherwise taught refusal, nothing
                           sent.
  --scope-bytes B64        (put, fingerprint) the raw-byte node spelling:
                           base64url over the raw path bytes, for names the
                           UTF-8 Path noun cannot carry (§5.4). On put it
                           pairs with --if-fingerprint and rides as one
                           guards[] entry; on fingerprint it is the mint's
                           node. Exactly one of --scope/--scope-bytes (put),
                           PATH/--scope-bytes (fingerprint). Cap-gated like
                           --scope.
  --expect-armed DIGEST    (script) refuse BEFORE splice unless this run's
                           armed set hashes to DIGEST. Hosts that gate write
                           sets run --dry first and pass the trace's
                           armed_digest. Receipt is not an armed row.
  --history                (test) the history tier over WORKSPACE (a git repo).
  --rule PAGE              (test --history) workspace-relative rule PAGE to
                           run.
  --spec PAGE              (test --history) workspace-relative SPEC page whose
                           ```golden fence declares exceptions; its rule: must
                           resolve to --rule's PAGE. Omitted: nothing declared.
  --fresh                  (sql) re-ask a STALE answer once, bounded; a run
                           that still cannot reach as_of == live reports RACED
                           rather than a fresh-looking result.
  --rebuild                (sql) delete the drawer's sql.duckdb cache and
                           cold-build it at the live corpus (repair verb). May
                           be given without a query.
  --cwd PATH               (sql, status) resolve the workspace from PATH
                           instead of the process working directory.
  -V, --version            build identity: package version + the tree this
                           binary was built from — a bare commit where that
                           tree was clean, `<commit>-dirty` where tracked
                           content diverged from it, `unknown` where neither
                           could be read.
  -h, --help               print this help.
";

/// A command failure: the process exit code plus a diagnostic for stderr.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: u8,
    pub(crate) message: String,
    /// Print the whole verb surface beneath the diagnostic. Set only by [`Fail::usage`].
    pub(crate) usage: bool,
}

impl Fail {
    /// A tool failure (exit 2).
    pub(crate) fn tool(message: String) -> Self {
        Fail {
            code: EXIT_FAIL,
            message,
            usage: false,
        }
    }

    /// A findings failure (exit 1): the verb ran, but reported failures.
    pub(crate) fn findings(message: String) -> Self {
        Fail {
            code: EXIT_FINDINGS,
            message,
            usage: false,
        }
    }

    /// A tool failure (exit 2) whose answer is the verb surface itself: no verb was named, or the
    /// name given is not one.
    pub(crate) fn usage(message: String) -> Self {
        Fail {
            code: EXIT_FAIL,
            message,
            usage: true,
        }
    }

    /// A failure carrying an explicit exit code: a verb whose own ladder names the leg
    /// (`EXIT_FINDING`, `EXIT_RUN`), and the `128 + signal` legs of `mrd run`.
    pub(crate) fn with_code(code: u8, message: String) -> Self {
        Fail {
            code,
            message,
            usage: false,
        }
    }
}

/// Output shape: a human table by default, JSON under `--json`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Format {
    Human,
    Json,
}

/// Did the caller spell "the whole corpus" at a door that admits one page?
///
/// The recovery half of the face-honesty law (laws.md § the face-honesty law,
/// clause 3) turns on this and NOTHING WIDER. `mrd read .` and `mrd walk .`
/// refuse correctly and point nowhere, while the verb the caller evidently
/// wanted — `mrd links --json` — enumerates. That pointer is only ever right
/// for the enumeration gesture: a caller who mistyped a real filename wants the
/// respelling they already get, and clause 3 rules that **a wrong pointer is
/// worse than none.** So this stays a closed set, never a heuristic.
pub(crate) fn names_the_whole_corpus(path: &str) -> bool {
    matches!(path.trim_end_matches('/'), "." | "" | "*" | "**")
}

/// The current working directory, as a tool failure when it cannot be read.
pub(crate) fn current_dir() -> Result<PathBuf, Fail> {
    std::env::current_dir()
        .map_err(|e| Fail::tool(format!("cannot read the current directory: {e}")))
}

/// The whole help text: the header and its legend, a blank line, then the listing.
fn usage() -> String {
    format!("{HEADER}\n{LISTING}")
}

/// The build identity, one line: the package version and the tree `build.rs` read at compile
/// time. The version alone cannot identify a binary — every crate carries the one workspace
/// stamp, so it names the release, never the build. The commit token carries a `-dirty` marker
/// where tracked content diverged from it, so a bare sha ASSERTS a whole commit rather than
/// merely naming one (`docs/release.md` §5.1).
fn version() -> String {
    format!(
        "mrd {} (git {})",
        env!("CARGO_PKG_VERSION"),
        env!("MRD_BUILD_SHA")
    )
}

/// Parse `args` (argv without the program name) and run the selected verb.
#[must_use]
pub fn run(args: &[String]) -> ExitCode {
    // The middleware door's ctx.sql backend (armed-plane Part A2): every mrd
    // process can host an in-process write, so the projection seam is
    // installed unconditionally at entry (idempotent).
    registry::mw_sql::install();
    // The one phase every verb has: the whole process. It is stopped on BOTH
    // arms, and deliberately AFTER the refusal is printed — unlike every phase
    // below it, `total` completes whether the verb succeeded or refused,
    // because the thing it measures is the process, and the process finished
    // either way (docs/status.md § The timing mode).
    let total = timing::phase("total");
    let exit = match dispatch(args) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(fail) => {
            // The diagnostic leads, so a refusal is never buried under the help listing.
            eprintln!("mrd: {}", fail.message);
            if fail.usage {
                eprint!("{}", usage());
            }
            ExitCode::from(fail.code)
        }
    };
    total.stop();
    exit
}

fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(verb) = args.first() else {
        return Err(Fail::usage("no subcommand given".to_owned()));
    };
    // The `cmd=` every timing line carries. One call covers every verb: the
    // instrument never learns a verb name of its own.
    timing::label(verb);
    if let Some(page) = help::for_invocation(args) {
        print!("{page}");
        return Ok(());
    }
    match verb.as_str() {
        "help" | "-h" | "--help" => {
            print!("{}", usage());
            Ok(())
        }
        "version" | "-V" | "--version" => {
            println!("{}", version());
            Ok(())
        }
        "init" => init::dispatch(&args[1..]),
        "unregister" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            unregister::run(p.positional.as_deref(), p.format())
        }
        "resolve" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            resolve::run_command(p.positional.as_deref(), p.format())
        }
        "links" => {
            let p = Parsed::parse(&args[1..], ALLOW_PATH, NO_ALL)?;
            engine::run_command(p.positional.as_deref(), p.format())
        }
        "read" => read_cmd::dispatch(&args[1..]),
        "fingerprint" => fingerprint_cmd::dispatch(&args[1..]),
        "put" => put_cmd::dispatch(&args[1..]),
        "rm" => rm_cmd::dispatch(&args[1..]),
        "pin" => pin_cmd::dispatch(&args[1..]),
        "repair" => repair_cmd::dispatch(&args[1..]),
        "walk" => walk_cmd::dispatch(&args[1..]),
        "rules" => rules_cmd::dispatch(&args[1..]),
        "arm" => arm_cmd::dispatch(&args[1..]),
        "check" => check_cmd::dispatch(&args[1..]),
        "skill" => skill_cmd::dispatch(&args[1..]),
        "config" => {
            let p = Parsed::parse(&args[1..], NO_PATH, NO_ALL)?;
            config_cmd::run(p.format())
        }
        "retire" => retire_cmd::dispatch(&args[1..]),
        "cache" => dispatch_cache(&args[1..]),
        "sql" => sql::run(&args[1..]),
        "status" => status_cmd::run(&args[1..]),
        "test" => test_cmd::dispatch(&args[1..]),
        "run" => run_cmd::dispatch(&args[1..]),
        "script" => script::cmd::dispatch(&args[1..]),
        "new" => new_cmd::run(&args[1..]),
        "unfold" => unfold_cmd::run(&args[1..]),
        "reconcile" => reconcile_cmd::run(&args[1..]),
        "realise" => realise_cmd::run(&args[1..]),
        "daemon" => {
            reject_extra(&args[1..])?;
            daemon::run()
        }
        other => Err(Fail::usage(format!("unknown subcommand: {other}"))),
    }
}

fn dispatch_cache(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::usage(
            "cache needs a subcommand (ls | clean)".to_owned(),
        ));
    };
    match sub.as_str() {
        "ls" => {
            let p = Parsed::parse(&args[1..], NO_PATH, NO_ALL)?;
            cache_cmd::ls(p.format())
        }
        "clean" => {
            let p = Parsed::parse(&args[1..], NO_PATH, ALLOW_ALL)?;
            cache_cmd::clean(p.all, p.format())
        }
        other => Err(Fail::usage(format!("unknown cache subcommand: {other}"))),
    }
}

/// Refuse any argument to a verb that takes none.
fn reject_extra(args: &[String]) -> Result<(), Fail> {
    match args.first() {
        None => Ok(()),
        Some(a) => Err(Fail::tool(format!("unexpected argument: {a}"))),
    }
}

// Named booleans for the shared parser: a verb states exactly what it accepts, and anything
// else is a loud exit-2 rather than ignored.
const ALLOW_PATH: bool = true;
const NO_PATH: bool = false;
const ALLOW_ALL: bool = true;
const NO_ALL: bool = false;

/// The parsed tail of a verb: an optional positional, `--json`, `--all`.
struct Parsed {
    positional: Option<String>,
    json: bool,
    all: bool,
}

impl Parsed {
    fn parse(tail: &[String], allow_path: bool, allow_all: bool) -> Result<Self, Fail> {
        let mut parsed = Parsed {
            positional: None,
            json: false,
            all: false,
        };
        for arg in tail {
            match arg.as_str() {
                "--json" => parsed.json = true,
                "--all" if allow_all => parsed.all = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if allow_path && parsed.positional.is_none() => {
                    parsed.positional = Some(value.to_owned());
                }
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        Ok(parsed)
    }

    fn format(&self) -> Format {
        if self.json {
            Format::Json
        } else {
            Format::Human
        }
    }
}

/// `<verb> --help`: the per-verb face of [`LISTING`], lexed back out of the same text
/// `mrd --help` prints. A block is found by its addressing words and reprinted verbatim — this
/// module holds no second copy of the help.
mod help {
    use super::{HEADER, LISTING};

    /// Where a description begins, on a block's own line and on every continuation line.
    const DESC_COL: usize = 27;

    /// One verb block: the words that address it, and its lines as they stand.
    struct Block {
        words: Vec<String>,
        lines: Vec<String>,
    }

    /// One options entry: the flags it defines, the verb named by the `(...)` tag its description
    /// opens with, and its lines. An untagged entry names no owner and reaches a page only
    /// through a synopsis that offers its flag.
    struct Opt {
        flags: Vec<String>,
        owner: Option<String>,
        lines: Vec<String>,
    }

    /// Does this line carry a gutter — the two bytes that open or continue a block? `! ` marks a
    /// verb that writes, `  ` one that does not. A line with no gutter closes the open block.
    fn has_gutter(line: &str) -> bool {
        matches!(line.get(..2), Some("! " | "  "))
    }

    /// The part of a line before its description begins. A synopsis long enough to overflow the
    /// description column carries no description on its own line, and keeps its whole tail.
    fn head_of(line: &str) -> &str {
        if line.as_bytes().get(DESC_COL - 1) == Some(&b' ') {
            // Byte 26 is a space, so byte 27 is a char boundary; the fallback is for a caller
            // that changes DESC_COL, not for this listing.
            line.get(..DESC_COL).unwrap_or(line)
        } else {
            line
        }
    }

    /// Strip the notation a listing wraps its flags in: `[--dry]` is `--dry`.
    fn bare(token: &str) -> &str {
        token.trim_matches(|c: char| matches!(c, '[' | ']' | ',' | '|' | '(' | ')'))
    }

    /// The addressing words of a synopsis: `! mrd cache clean [--all]` gives `["cache", "clean"]`.
    /// They stop at the first token that is not a bare lowercase name.
    fn words_of(line: &str) -> Vec<String> {
        head_of(line)
            .get(2..)
            .and_then(|synopsis| synopsis.strip_prefix("mrd "))
            .unwrap_or_default()
            .split_whitespace()
            .take_while(|word| word.chars().all(|c| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect()
    }

    /// The flags an options entry defines — `-h, --help` defines two.
    fn flags_of(line: &str) -> Vec<String> {
        head_of(line)
            .split_whitespace()
            .map(bare)
            .filter(|token| token.starts_with('-') && token.len() > 1)
            .map(str::to_owned)
            .collect()
    }

    /// A block's synopsis: its operands and flags, description left out. A synopsis can wrap, and
    /// the line it wraps onto is the one that does not begin at the description column.
    fn synopsis_of(block: &Block) -> String {
        let mut synopsis = String::new();
        for (n, line) in block.lines.iter().enumerate() {
            let is_description = line.bytes().take(DESC_COL).all(|b| b == b' ');
            if n == 0 || !is_description {
                synopsis.push_str(head_of(line));
                synopsis.push(' ');
            }
        }
        synopsis
    }

    /// Does a verb's own synopsis offer this flag?
    fn offers(synopsis: &str, flag: &str) -> bool {
        synopsis
            .split_whitespace()
            .map(bare)
            .any(|token| token == flag)
    }

    /// The verb an option belongs to: the first word inside the `(...)` its description opens
    /// with, so `--rule PAGE (test --history) ...` belongs to `test`.
    fn owner_of(line: &str) -> Option<String> {
        let owner: String = line
            .get(DESC_COL..)?
            .trim_start()
            .strip_prefix('(')?
            .chars()
            .take_while(char::is_ascii_lowercase)
            .collect();
        (!owner.is_empty()).then_some(owner)
    }

    /// Lex the listing's verb blocks. A block opens on a gutter followed by
    /// `mrd ` and runs to the first line with no gutter at all.
    fn blocks() -> Vec<Block> {
        let mut blocks: Vec<Block> = Vec::new();
        let mut open: Option<usize> = None;
        for line in LISTING.lines() {
            if !has_gutter(line) {
                open = None;
            } else if line[2..].starts_with("mrd ") {
                open = Some(blocks.len());
                blocks.push(Block {
                    words: words_of(line),
                    lines: vec![line.to_owned()],
                });
            } else if let Some(block) = open {
                blocks[block].lines.push(line.to_owned());
            }
        }
        blocks
    }

    /// Lex the options block the same way: an entry opens on a gutter followed
    /// by a `-`.
    fn options() -> Vec<Opt> {
        let mut opts: Vec<Opt> = Vec::new();
        let mut open: Option<usize> = None;
        for line in LISTING.lines() {
            if !has_gutter(line) {
                open = None;
            } else if line[2..].starts_with('-') {
                open = Some(opts.len());
                opts.push(Opt {
                    flags: flags_of(line),
                    owner: owner_of(line),
                    lines: vec![line.to_owned()],
                });
            } else if let Some(opt) = open {
                opts[opt].lines.push(line.to_owned());
            }
        }
        opts
    }

    /// Is this invocation asking for help? Only up to a `--` separator: in
    /// `mrd run PAGE TASK -- --help` the flag is the task's argument, not this CLI's.
    fn asks(args: &[String]) -> bool {
        args.iter()
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg.as_str() == "-h" || arg.as_str() == "--help")
    }

    /// The verb words an invocation names: its leading bare-lowercase arguments. The first
    /// operand or flag ends them, so `mrd read notes.md --help` asks about `read`.
    fn query_of(args: &[String]) -> Vec<String> {
        args.iter()
            .take_while(|arg| !arg.is_empty() && arg.chars().all(|c| c.is_ascii_lowercase()))
            .cloned()
            .collect()
    }

    /// One word list is a prefix of the other. So `mrd cache --help` answers with both cache
    /// subcommands, `mrd cache clean --help` with just that one, and `mrd read notes.md --help`
    /// with `read`.
    fn addresses(query: &[String], words: &[String]) -> bool {
        !query.is_empty() && !words.is_empty() && query.iter().zip(words).all(|(q, w)| q == w)
    }

    /// The page: the header and its legend, the matched block(s) under `usage:`, then the options
    /// those verbs take. An option reaches a page two ways: the `(...)` tag names the verb, or the
    /// verb's own synopsis offers the flag.
    fn render(matched: &[Block]) -> String {
        let owners: Vec<&str> = matched
            .iter()
            .filter_map(|block| block.words.first())
            .map(String::as_str)
            .collect();
        let synopses: Vec<String> = matched.iter().map(synopsis_of).collect();
        let owned: Vec<Opt> = options()
            .into_iter()
            .filter(|opt| {
                opt.owner.as_deref().is_some_and(|o| owners.contains(&o))
                    || opt
                        .flags
                        .iter()
                        .any(|flag| synopses.iter().any(|synopsis| offers(synopsis, flag)))
            })
            .collect();

        let mut page = String::from(HEADER);
        page.push_str("\nusage:\n");
        for line in matched.iter().flat_map(|block| &block.lines) {
            page.push_str(line);
            page.push('\n');
        }
        if !owned.is_empty() {
            page.push_str("\noptions:\n");
            for line in owned.iter().flat_map(|opt| &opt.lines) {
                page.push_str(line);
                page.push('\n');
            }
        }
        page.push_str("\nsee `mrd --help` for every verb.\n");
        page
    }

    /// The help page for the verb an invocation addresses — or `None` when it asks for no help,
    /// or names nothing this CLI knows. Falling through leaves the refusal to dispatch, which is
    /// what keeps `mrd nope --help` an `unknown subcommand`.
    pub(super) fn for_invocation(args: &[String]) -> Option<String> {
        if !asks(args) {
            return None;
        }
        let query = query_of(args);
        let matched: Vec<Block> = blocks()
            .into_iter()
            .filter(|block| addresses(&query, &block.words))
            .collect();
        if matched.is_empty() {
            return None;
        }
        Some(render(&matched))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Every block the lexer finds is addressable; empty words would silently have no page.
        #[test]
        fn every_block_is_addressable() {
            for block in blocks() {
                assert!(
                    !block.words.is_empty(),
                    "a block no --help can ever address:\n{}",
                    block.lines.join("\n")
                );
            }
        }

        /// The two-word verbs resolve to both of their words.
        #[test]
        fn multi_word_verbs_keep_both_words() {
            let found: Vec<Vec<String>> = blocks()
                .into_iter()
                .map(|block| block.words)
                .filter(|words| words.len() > 1)
                .collect();
            assert_eq!(
                found,
                vec![
                    vec!["skill", "hook"],
                    vec!["cache", "ls"],
                    vec!["cache", "clean"],
                ],
                "the two-word verbs of the listing"
            );
        }

        /// Operands stop the words: the two `mrd test` blocks share one verb name, so
        /// `mrd test --help` prints both.
        #[test]
        fn operands_and_flags_end_the_words() {
            let test_blocks: Vec<Vec<String>> = blocks()
                .into_iter()
                .map(|block| block.words)
                .filter(|words| words.first().is_some_and(|word| word == "test"))
                .collect();
            assert_eq!(test_blocks, vec![vec!["test"], vec!["test"]]);
        }

        /// A synopsis that overflows the description column still lexes: the guard is that byte 26
        /// is a space, not that the line is long enough to slice.
        #[test]
        fn an_overflowing_synopsis_still_lexes() {
            let overflowing = "  mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry] [--json]";
            assert_ne!(
                overflowing.as_bytes()[DESC_COL - 1],
                b' ',
                "the case this test is for: no description column on this line"
            );
            assert_eq!(words_of(overflowing), vec!["pin"]);
        }

        /// The write mark is the gutter: 16 verbs write, the rest read.
        #[test]
        fn sixteen_verbs_are_marked_as_writers() {
            let marked: Vec<&str> = LISTING
                .lines()
                .filter(|line| line.starts_with("! "))
                .collect();
            assert_eq!(
                marked.len(),
                16,
                "marked as writers:\n{}",
                marked.join("\n")
            );
            assert_eq!(blocks().len(), 30, "verb blocks in the listing");
        }

        /// Every option that names an owner names a verb that exists.
        #[test]
        fn every_tagged_option_names_a_real_verb() {
            let verbs: Vec<String> = blocks()
                .into_iter()
                .filter_map(|block| block.words.first().cloned())
                .collect();
            for opt in options() {
                if let Some(owner) = opt.owner {
                    assert!(
                        verbs.contains(&owner),
                        "option tagged ({owner}), a verb the listing does not have:\n{}",
                        opt.lines.join("\n")
                    );
                }
            }
        }

        /// The query stops where the verb does — an operand is not a verb word.
        #[test]
        fn a_query_is_the_leading_verb_words_only() {
            assert_eq!(
                query_of(&["read", "notes.md", "--help"].map(String::from)),
                ["read"]
            );
            assert_eq!(query_of(&["cache", "--help"].map(String::from)), ["cache"]);
            assert_eq!(
                query_of(&["cache", "clean", "--help"].map(String::from)),
                ["cache", "clean"]
            );
            assert!(query_of(&["--help"].map(String::from)).is_empty());
        }

        /// A `--` separator ends this CLI's own arguments.
        #[test]
        fn a_separator_hides_the_flag_from_this_cli() {
            assert!(asks(&["run", "page.md", "--help"].map(String::from)));
            assert!(!asks(
                &["run", "page.md", "task", "--", "--help"].map(String::from)
            ));
            assert!(
                for_invocation(&["run", "p.md", "t", "--", "--help"].map(String::from)).is_none()
            );
        }
    }
}
