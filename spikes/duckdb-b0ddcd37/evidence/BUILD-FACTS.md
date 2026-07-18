# BUILD-FACTS — panelist-B Evidence.dev dashboard (round 4)

Measured facts for the round-4 Evidence project. Every row maps 1:1 to the
pre-registered kill criteria in `results/foundation-panel/round-4/b0ddcd37.md`
(V1–V4). No claim without a command or artifact behind it.

- **Project:** `spikes/duckdb-b0ddcd37/evidence/`
- **Source:** CoW snapshot `snapshot/wiki-snap.duckdb` of `../data/query.duckdb`
- **Host:** zmax — macOS 26.5.2, arm64. Node v24.16.0 / npm 11.13.0.
- **Evidence:** `@evidence-dev/evidence@40.1.8`, stock `evidence-dev/template`,
  connector `@evidence-dev/duckdb@2.0.x` (bundles `@duckdb/node-api@1.4.2-r.1` = DuckDB 1.4.2).
- **Snapshot root:** `07e645630e595cee` · built_at `2026-07-18 17:25:55 UTC` ·
  file mtime `2026-07-18 13:25:59 EDT` · 32,256,000 bytes (30.8 MB).

---

## CoW snapshot (the law: Evidence never reads the live file)

```
cp -c ../data/query.duckdb snapshot/wiki-snap.duckdb
```

- Clone wall: **~5 ms** (`/usr/bin/time` reports `real 0.00`; APFS block-level
  clone — effectively instant, size-independent). Source and clone both 32,256,000 bytes.
- Snapshot opened **read-only** everywhere; the live file is never attached.

---

## V1 — toolchain integrity   → **PASS**

Threshold: install + source extract + static build succeed without patching
Evidence internals or forking; ≤ 15 min wall total.

| step | command | wall |
|---|---|---:|
| install | `npm install` | **10.14 s** |
| sources (cold, 1st run) | `npm run sources` | **5.13 s** |
| sources (warm, final) | `npm run sources` | 2.70 s |
| static build | `npm run build` | **18.66 s** (first: 19.36 s) |
| **cold total** | install + sources + build | **33.9 s ≈ 0.57 min** |

- **Budget margin: 15 min ÷ 0.57 min ≈ 26×.**
- **Evidence internals patched? NO. Forked? NO.** Stock template, stock
  `@evidence-dev/evidence@40.1.8`, unmodified `node_modules`.
- Final build tail (quality gate — build exits 0):

  ```
  > Using @sveltejs/adapter-static
    Wrote site to "./build"
    ✔ done
  Build complete --> ./build
  real 18.66
  ```

- Final sources tail:

  ```
    changes ✔ Finished, wrote 32344 rows.
    files ✔ Finished, wrote 22344 rows.
    frontmatter ✔ Finished, wrote 200449 rows.
    links ✔ Finished, wrote 26331 rows.
  [INFO]:    ✅ Done!
  ```

### Deviations from stock Evidence (V1 honesty)

None touch Evidence internals. Project-level authoring choices only:

1. Removed the sample `needful_things` source; added a `wiki` DuckDB source at
   the snapshot path.
2. **Boundary page uses a comparison table, not a mermaid diagram** — Evidence
   ships **no mermaid renderer** (only a Prism *syntax-highlight* entry; a
   ```` ```mermaid ```` fence errors in the query preprocessor and never renders
   a diagram). This is an Evidence capability gap, worked around at the page
   level, not an internals patch.
3. Banner interpolations authored *without* backtick code-spans — Evidence does
   not template `{...}` inside inline code spans. Page-authoring detail.

---

## V2 — storage-format coupling   → **PASS (native read, no Parquet intermediary)**

Threshold: Evidence reads the libduckdb-1.5.4-written snapshot natively. A
required Parquet intermediary would be a *named coupling finding* (not a kill);
total failure would be a kill.

- Snapshot written by **libduckdb 1.5.4** (per `REPORT.md` § environment).
  Evidence's connector bundles **DuckDB 1.4.2** — an *older* engine reading a
  *newer* writer's file.
- **Result: native read succeeds.** `npm run sources` extracted every table
  directly from the snapshot with zero errors (22,344 files · 160,629 nodes were
  not queried · 26,331 links · 200,449 frontmatter · 32,344 changes · roots ·
  batches). **No Parquet intermediary was required** — best-case V2, not even a
  named coupling finding.
- **Parquet intermediary needed: NO.**

Converse (the meta surface, not a workaround): Evidence's *own* source step
extracts to Parquet under `.evidence/template/static/data/wiki/`. That extract is
the catalog's "Parquet export converging with the harvest datalake":

| parquet | rows | bytes |
|---|---:|---:|
| files | 22,344 | 655,798 |
| frontmatter | 200,449 | 1,393,934 |
| links | 26,331 | 179,064 |
| changes | 32,344 | 539,254 |
| roots | 1 | 409 |
| batches | 1 | 764 |
| asof | 1 | 1,233 |
| **total** | **281,470** | **≈ 2.77 MB** |

30.8 MB snapshot → 2.77 MB Parquet projection of queried columns = **0.09×**.

---

## V3 — page latency   → **PASS (max 206 ms, threshold < 1 s)**

Threshold: every minimum page interactive < 1 s on this volume (22,344 files /
160k nodes / 32k changes) after load.

| page | time-to-data |
|---|---:|
| read/backlinks | 144 ms |
| read/orphans (22k×26k anti-join) | 130 ms |
| read/tags (agg over 200k rows) | 152 ms |
| read/board (frontmatter self-join) | 136 ms |
| read/shapes (type×key coverage) | 130 ms |
| read/hubs (in-degree distribution) | 151 ms |
| history/path-history | 125 ms |
| history/heatmap | 127 ms |
| history/compound-inputs | **206 ms** (max) |

**Method:** wall-clock (`python perf_counter`) around *[navigate away → navigate
to page → wait until a known DOM-text data marker appears]*, warm preview server
+ warm DuckDB-WASM (page pre-visited so Parquet is cached in the browser),
headless Chrome, `localhost:3000`. Includes navigation + agent-browser CLI
subprocess overhead, so each figure is an **upper bound** on the interactive
query-render latency; coarse (~±30 ms). All nine data pages land ≤ 206 ms.

---

## V4 — authoring cost   → **PASS (≤ 30 lines, zero custom JS)**

Threshold: one existing r3 SQL query → new rendered page in ≤ 30 lines of
page-markdown, zero custom JS.

- **Exemplar: `read/tags.md`** = the r3 reference query `Q2` verbatim →
  `<BarChart>` + `<DataTable>`. **22 non-blank body lines** total (banner + prose
  included); the SQL-block + two render components alone are **~12 lines**.
- **Zero custom JS across the whole project** — no `.svelte`, no `components/`,
  no `<script>`. Pure markdown + declarative Evidence components.
- Every read page maps directly to an r3 reference query: backlinks→Q1, tags→Q2,
  board→Q3, path-history→Q4, orphans→Q5, hubs→QC.

### Per-page markdown LOC (total / non-blank body, excl. frontmatter)

| page | surface | total | body |
|---|---|---:|---:|
| index.md | landing + provenance legend | 62 | 41 |
| read/backlinks.md | Q1 backlinks (parameterized) | 40 | 27 |
| read/orphans.md | Q5 orphans + dangling anti-joins | 58 | 42 |
| read/tags.md | Q2 tag aggregation *(V4 exemplar)* | 33 | 22 |
| read/board.md | Q3 board by status/owner | 45 | 31 |
| read/shapes.md | shape conformance (type×key) | 52 | 39 |
| read/hubs.md | QC hub/coverage distribution | 52 | 37 |
| history/path-history.md | Q4 per-path window | 46 | 33 |
| history/heatmap.md | activity heatmap (stream×kind) | 49 | 36 |
| history/compound-inputs.md | root A→B scoped (3 queries) | 85 | 67 |
| meta/parquet-export.md | datalake convergence note | 39 | 28 |
| meta/rulepack-ledger.md | schema + mock (NOT RUN) | 58 | 44 |
| boundary.md | two-faces law + R5 line | 54 | 39 |

(`compound-inputs.md` is the largest because it renders three queries — root
pair + scope summary + change set — in one page; it is not the V4 exemplar. The
V4 claim is the *minimum* authoring cost, which tags/parquet demonstrate.)

---

## Verdict against pre-registration

**Zero kills** (V1 PASS · V2 PASS native · V3 PASS · V4 PASS) ⇒ per the
pre-registered rule, **Evidence is ratified as THE rung-5 human face** — not
merely a reference implementation a plainer face could replace.

---

## Every page carries (quality gate)

- **as_of_root banner** — root hex `07e645630e595cee` read from the snapshot's
  own `roots`/`batches`, + snapshot file mtime. Visible on all 13 pages.
- **data-provenance label** — read surfaces **REAL**; history surfaces
  **SYNTHETIC-DEMO** (prominent warning); rulepack **NOT RUN / MOCK**.

## Screenshot inventory

`$SESSION_DIR/results/foundation-panel/round-4/b0ddcd37/`:

| file | page | notes |
|---|---|---|
| 00-index.png | landing | corpus BigValues + provenance legend |
| 01-backlinks.png | Q1 backlinks | dropdown=schema, 1154 backlinkers |
| 02-orphans.png | Q5 orphans+dangling | 16,625 orphans / 3,134 dangling |
| 03-tags.png | Q2 tags | bar chart + colorscale table |
| 04-board.png | Q3 board | status bars + owner table |
| 05-shapes.png | shape conformance | type×key coverage heatmap |
| 06-hubs.png | QC hubs | top-25 bars + power-law histogram |
| 07-path-history.png | Q4 window | SYNTHETIC-DEMO, dropdown-driven |
| 08-heatmap.png | activity heatmap | SYNTHETIC-DEMO, stream×kind |
| 09-compound-inputs.png | compound front door | SYNTHETIC-DEMO, root A→B, 2737 scoped |
| 10-parquet-export.png | datalake convergence | parquet manifest 2.77 MB |
| 11-rulepack-ledger.png | rule-pack ledger | NOT RUN banner + mock |
| 12-boundary.png | boundary | R5 line verbatim + two-faces table |

## Reproduce

```
cd spikes/duckdb-b0ddcd37/evidence
cp -c ../data/query.duckdb snapshot/wiki-snap.duckdb   # if snapshot absent
npm install && npm run sources && npm run build
npm run preview     # serves ./build at localhost:3000
```
