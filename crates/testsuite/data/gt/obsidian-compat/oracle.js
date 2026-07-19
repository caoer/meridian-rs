// obsidian-compat resolution GT oracle — B3-APP-GT.
//
// Generates the obsidian-compat resolution pack from the LIVE Obsidian
// resolver (never hand-written). Runs inside the app via:
//
//   obsidian eval code="$(cat oracle.js)"
//
// Requires the `probe-bridge` plugin enabled so window.__ob === require("obsidian")
// (exposes parseLinktext + resolveSubpath). Stage 1 (getFirstLinkpathDest) and
// the metadata cache are always on app.
//
// Probe set (FIXED by gate-1 ruling 4 + decision 013): ZT's six walk-law probes
// (WL-1..WL-6, the WL-6a/WL-6b hpath-join pair is one probe) + the `_`-bearing
// anchor probe (WX-6). Corpus: fixtures/walkvault/ (walk.md + blocks.md),
// byte-identical to gate-3 adversarial-harness. Spans are the app's UTF-8 byte
// offsets (the corpus is ASCII, so UTF-16 Loc.offset == UTF-8 offset).
(function () {
  // Live-vault path to the pack corpus. The pack's own walkvault/ copy is
  // byte-identical; answers are recorded pack-corpus-relative (WV stripped).
  var WV = "year=2026/month=07/18-02-meridian-rs/results/contract-v2-tournament/gate-3/adversarial-harness/fixtures/walkvault/";

  var probes = [
    { id: "WL-1", from: "walk.md", ref: "#B#Beta", law: "anywhere-after: closed B/C/A boundaries never stop the walk" },
    { id: "WL-2", from: "walk.md", ref: "#B#b", law: "generation-skipping: level 1->3 without naming intermediate X" },
    { id: "WL-3", from: "walk.md", ref: "#B#C", law: "strictly-deeper-level required: C is level 1, same as B" },
    { id: "WL-4", from: "walk.md", ref: "#A#Beta", law: "first-match-wins on duplicates, SILENT on the walk plane" },
    { id: "WL-5", from: "walk.md", ref: "#b#beta", law: "case-insensitive walk, both segments" },
    { id: "WL-6a", from: "walk.md", ref: "#A#a/b", law: "subpath splits on '#' only: 'a/b' is ONE literal-heading segment" },
    { id: "WL-6b", from: "walk.md", ref: "#A#a#b", law: "same chars + one more '#' -> a DIFFERENT node; the pair must diverge" },
    { id: "WX-6", from: "walk.md", ref: "blocks#^under-probe_x", law: "decision-011 `_` probe: pin what the app DOES with a legacy `_` block id" }
  ];

  function answer(p) {
    var fromPath = WV + p.from;
    var parsed = window.__ob.parseLinktext(p.ref); // { path, subpath }
    var linkpath = parsed.path;
    var subpath = parsed.subpath;

    // Stage 1: vault namespace, source-relative. Empty linkpath === same file.
    var dest = linkpath === ""
      ? app.vault.getAbstractFileByPath(fromPath)
      : app.metadataCache.getFirstLinkpathDest(linkpath, fromPath);

    if (!dest) {
      return { ok: false, error: { code: "ref_not_found", stage: 1, dest: null } };
    }

    var destRel = dest.path.indexOf(WV) === 0 ? dest.path.slice(WV.length) : dest.path;

    // Stage 2: subpath walk (heading path / block / footnote).
    var r = window.__ob.resolveSubpath(app.metadataCache.getCache(dest.path), subpath);
    if (!r) {
      return { ok: false, error: { code: "ref_not_found", stage: 2, dest: destRel } };
    }
    // The app returns end === null when the section runs to EOF (no later
    // heading at same-or-lower level). Span end is then the file's byte size
    // (app-sourced, not hand-computed). Corpus is ASCII, so byte == UTF-16 == UTF-8.
    var endOffset = r.end === null ? dest.stat.size : r.end.offset;
    var out = { ok: true, dest: destRel, span: [r.start.offset, endOffset], resolved_type: r.type };
    if (r.end === null) { out.span_end_is_eof = true; }
    return out;
  }

  var out = probes.map(function (p) {
    return { id: p.id, from: p.from, ref: p.ref, parsed: window.__ob.parseLinktext(p.ref), law: p.law, answer: answer(p) };
  });

  return JSON.stringify(out, null, 2);
})();
