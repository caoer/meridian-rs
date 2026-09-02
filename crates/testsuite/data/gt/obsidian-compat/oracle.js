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
// byte-identical to gate-3 adversarial-harness.
//
// SPAN UNITS — true UTF-8 byte offsets. The app's `Loc.offset` is UTF-16 code
// units (the corpus is NOT ASCII: walk.md carries U+2014 x5 + U+2192 x1, each a
// 3-byte / 1-UTF-16-unit char). We transcode each offset to a UTF-8 byte offset
// against the file's own bytes — TextEncoder.encode(content.slice(0, off)).length
// (content.slice indexes in UTF-16 code units, matching Loc.offset). The span
// VALUE still originates from the live resolver; only its unit is normalized,
// the same UTF-16->UTF-8 transcode the parity ruling assigns to the harness,
// performed once at generation so the recorded offsets are byte-coherent.
(function () {
  // Vault-relative path of the walkvault/ copy the open vault carries (see
  // generate.sh preconditions); answers are recorded pack-corpus-relative
  // (WV stripped), so the copy's location in the vault never enters the pack.
  var WV = "walkvault/";

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

  // UTF-16 code-unit offset (Loc.offset) -> UTF-8 byte offset, against `content`.
  // content.slice(0, o) indexes in UTF-16 code units, so it selects exactly the
  // prefix the app measured; its UTF-8 encoding length is the byte offset.
  function utf16ToByte(content, o) {
    return new TextEncoder().encode(content.slice(0, o)).length;
  }

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
      return Promise.resolve({ ok: false, error: { code: "ref_not_found", stage: 1, dest: null } });
    }

    var destRel = dest.path.indexOf(WV) === 0 ? dest.path.slice(WV.length) : dest.path;

    // Stage 2: subpath walk (heading path / block / footnote).
    var r = window.__ob.resolveSubpath(app.metadataCache.getCache(dest.path), subpath);
    if (!r) {
      return Promise.resolve({ ok: false, error: { code: "ref_not_found", stage: 2, dest: destRel } });
    }

    // Read the dest's own bytes to transcode the resolver's UTF-16 offsets to
    // UTF-8 byte offsets. The app returns end === null when the section runs to
    // EOF (no later heading at same-or-lower level); the span end is then the
    // file's byte size (dest.stat.size, already UTF-8 bytes, == encode(content)).
    return app.vault.cachedRead(dest).then(function (content) {
      var startByte = utf16ToByte(content, r.start.offset);
      var endByte = r.end === null ? dest.stat.size : utf16ToByte(content, r.end.offset);
      var out = { ok: true, dest: destRel, span: [startByte, endByte], resolved_type: r.type };
      if (r.end === null) { out.span_end_is_eof = true; }
      return out;
    });
  }

  return Promise.all(probes.map(function (p) {
    return answer(p).then(function (a) {
      return { id: p.id, from: p.from, ref: p.ref, parsed: window.__ob.parseLinktext(p.ref), law: p.law, answer: a };
    });
  })).then(function (out) {
    return JSON.stringify(out, null, 2);
  });
})();
