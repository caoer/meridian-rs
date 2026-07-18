// Round-3 DuckDB spike — panelist e4201e72. Throwaway measurement code.
//
// One binary, many subcommands. The spike answers ZT's five axes with NUMBERS:
// SPEED / SIZE / WAL / PERFORMANCE / QUERYABILITY. Extract time and DB time are
// timed SEPARATELY in every path so DB numbers stay comparable across extractors.
//
// Schema is the protocol minimum (files, nodes, links, frontmatter, changes,
// roots) plus ONE documented extension: `batches(seq, root_before, root_after,
// ts)` to record per-write-batch root transitions (host commitment 3 / the
// rootstamp demo needs root_before AND root_after; `roots(root, built_at)` alone
// cannot express a transition). Every other table is protocol-exact.

use duckdb::{AccessMode, Config, Connection};
use regex::Regex;
use std::fs;
use std::io::Write as _;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const CORPUS: [&str; 2] = [
    "/Users/Shared/projects/home-wiki",
    "/Users/Shared/projects/home-wiki-sessions",
];

// ---------------------------------------------------------------------------
// small utils
// ---------------------------------------------------------------------------

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// content hash as a signed i64 (DuckDB BIGINT). Bit-cast keeps it deterministic.
fn rev_of(bytes: &[u8]) -> i64 {
    fnv1a64(bytes) as i64
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn pct(sorted_us: &[f64], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let idx = (((p / 100.0) * (sorted_us.len() as f64 - 1.0)).round()) as usize;
    sorted_us[idx.min(sorted_us.len() - 1)]
}

fn stem_lower(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let base = base.strip_suffix(".md").unwrap_or(base);
    base.to_lowercase()
}

fn file_size(path: &str) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// extractor (cheap regex walk) — TIMED SEPARATELY from DB insert everywhere
// ---------------------------------------------------------------------------

struct FileRec {
    path: String,
    rev: i64,
    bytes: i64,
}
struct NodeRec {
    path: String,
    kind: String,
    span_start: i64,
    span_end: i64,
    hpath: String,
    rev: i64,
}
struct LinkRec {
    src: String,
    target: String, // normalized: basename, no extension, lowercased
    resolved: bool,
}
struct FmRec {
    path: String,
    key: String,
    value: String,
}

#[derive(Default)]
struct Corpus {
    files: Vec<FileRec>,
    nodes: Vec<NodeRec>,
    links: Vec<LinkRec>,
    fm: Vec<FmRec>,
    total_bytes: u64,
}

struct Rx {
    wikilink: Regex,
    heading: Regex,
    kv: Regex,
}
impl Rx {
    fn new() -> Self {
        Rx {
            wikilink: Regex::new(r"\[\[([^\]\|#]+)").unwrap(),
            heading: Regex::new(r"(?m)^(#{1,6})[ \t]+(.+?)\s*$").unwrap(),
            kv: Regex::new(r"^([A-Za-z][A-Za-z0-9_\-]*):\s*(.*)$").unwrap(),
        }
    }
}

// parse ONE file's content into rows (links unresolved until the global stem set
// is known). Pure CPU — this is the "extract" cost.
fn extract_one(rx: &Rx, path: &str, content: &str, out: &mut Corpus) {
    let rev = rev_of(content.as_bytes());
    out.files.push(FileRec {
        path: path.to_string(),
        rev,
        bytes: content.len() as i64,
    });

    // ---- frontmatter (YAML-ish leading fence) ----
    let mut body_start = 0usize;
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            body_start = 4 + end + 4; // "---\n" + fm + "\n---"
            let mut list_key: Option<String> = None;
            for line in fm.lines() {
                let trimmed = line.trim_end();
                if let Some(item) = trimmed.trim_start().strip_prefix("- ") {
                    // continuation of a block list under `key:`
                    if let Some(k) = &list_key {
                        let key = if k == "tags" { "tag" } else { k.as_str() };
                        out.fm.push(FmRec {
                            path: path.to_string(),
                            key: key.to_string(),
                            value: clean_scalar(item),
                        });
                    }
                    continue;
                }
                if let Some(c) = rx.kv.captures(trimmed) {
                    let key = c.get(1).unwrap().as_str().to_string();
                    let val = c.get(2).unwrap().as_str().trim();
                    if val.is_empty() {
                        list_key = Some(key); // block list follows
                        continue;
                    }
                    list_key = None;
                    if val.starts_with('[') {
                        // inline flow list e.g. tags: [type/task, topic/x]
                        let inner = val.trim_start_matches('[').trim_end_matches(']');
                        let emit_key = if key == "tags" { "tag" } else { key.as_str() };
                        for part in inner.split(',') {
                            let p = clean_scalar(part);
                            if !p.is_empty() {
                                out.fm.push(FmRec {
                                    path: path.to_string(),
                                    key: emit_key.to_string(),
                                    value: p,
                                });
                            }
                        }
                    } else {
                        let emit_key = if key == "tags" { "tag" } else { key.as_str() };
                        out.fm.push(FmRec {
                            path: path.to_string(),
                            key: emit_key.to_string(),
                            value: clean_scalar(val),
                        });
                    }
                }
            }
        }
    }

    // ---- headings -> nodes (span end filled by next heading / EOF) ----
    let mut hstack: Vec<(usize, String)> = Vec::new();
    let mut pending: Vec<(usize, i64)> = Vec::new(); // (index into out.nodes, span_start)
    for m in rx.heading.captures_iter(content) {
        let hashes = m.get(1).unwrap().as_str().len();
        let title = m.get(2).unwrap().as_str().to_string();
        let start = m.get(0).unwrap().start() as i64;
        while hstack.last().map_or(false, |(l, _)| *l >= hashes) {
            hstack.pop();
        }
        hstack.push((hashes, title.clone()));
        let hpath = hstack
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        pending.push((out.nodes.len(), start));
        out.nodes.push(NodeRec {
            path: path.to_string(),
            kind: "heading".to_string(),
            span_start: start,
            span_end: 0, // patched below
            hpath,
            rev,
        });
    }
    // patch span_end = next heading start, last = EOF
    for i in 0..pending.len() {
        let (node_idx, _) = pending[i];
        let end = if i + 1 < pending.len() {
            pending[i + 1].1
        } else {
            content.len() as i64
        };
        out.nodes[node_idx].span_end = end;
    }

    // ---- wikilinks -> links (target normalized; resolved filled later) ----
    let body = &content[body_start.min(content.len())..];
    for m in rx.wikilink.captures_iter(body) {
        let raw = m.get(1).unwrap().as_str().trim();
        let norm = stem_lower(raw);
        if !norm.is_empty() {
            out.links.push(LinkRec {
                src: path.to_string(),
                target: norm,
                resolved: false,
            });
        }
    }
}

fn clean_scalar(s: &str) -> String {
    s.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

// walk both corpus roots, excluding dot-dirs; returns (paths).
fn walk_corpus() -> Vec<String> {
    let mut out = Vec::new();
    for root in CORPUS.iter() {
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name()
                    .to_str()
                    .map(|s| s.starts_with('.'))
                    .unwrap_or(false)
            })
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Some(p) = entry.path().to_str() {
                    if p.ends_with(".md") {
                        out.push(p.to_string());
                    }
                }
            }
        }
    }
    out
}

// full extract phase (walk + read + parse). Returns (Corpus, extract_secs, read_secs).
fn extract_all() -> (Corpus, f64, f64) {
    let rx = Rx::new();
    let paths = walk_corpus();
    let mut corpus = Corpus::default();
    let mut read_secs = 0.0f64;
    let t0 = Instant::now();
    for p in &paths {
        let tr = Instant::now();
        let content = fs::read(p).unwrap_or_default();
        read_secs += tr.elapsed().as_secs_f64();
        let text = String::from_utf8_lossy(&content).into_owned();
        corpus.total_bytes += content.len() as u64;
        extract_one(&rx, p, &text, &mut corpus);
    }
    // resolve links against the global stem set
    let stems: std::collections::HashSet<String> =
        corpus.files.iter().map(|f| stem_lower(&f.path)).collect();
    for l in corpus.links.iter_mut() {
        l.resolved = stems.contains(&l.target);
    }
    let extract_secs = t0.elapsed().as_secs_f64();
    (corpus, extract_secs, read_secs)
}

// content-addressed root over sorted (path, rev). Staleness self-refuting.
fn compute_root(files: &[FileRec]) -> String {
    let mut lines: Vec<String> = files.iter().map(|f| format!("{}:{}", f.path, f.rev)).collect();
    lines.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for l in &lines {
        for &b in l.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= b'\n' as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    path VARCHAR PRIMARY KEY, file_rev BIGINT, bytes BIGINT
);
CREATE TABLE IF NOT EXISTS nodes (
    path VARCHAR, kind VARCHAR, span_start BIGINT, span_end BIGINT,
    hpath VARCHAR, node_rev BIGINT
);
CREATE TABLE IF NOT EXISTS links (
    src_path VARCHAR, target VARCHAR, resolved BOOLEAN
);
CREATE TABLE IF NOT EXISTS frontmatter (
    path VARCHAR, key VARCHAR, value VARCHAR
);
CREATE TABLE IF NOT EXISTS changes (
    seq BIGINT, ts TIMESTAMP, path VARCHAR, kind VARCHAR,
    change VARCHAR, old_rev BIGINT, new_rev BIGINT
);
CREATE TABLE IF NOT EXISTS roots (
    root VARCHAR, built_at TIMESTAMP
);
-- documented extension: per-batch root transition (host commitment 3)
CREATE TABLE IF NOT EXISTS batches (
    seq BIGINT, root_before VARCHAR, root_after VARCHAR, ts TIMESTAMP
);
"#;

fn open_rw(path: &str) -> Connection {
    Connection::open(path).expect("open rw")
}

// bulk insert the extracted corpus via the Appender fast path. Returns insert_secs.
fn insert_corpus(conn: &Connection, corpus: &Corpus, root: &str) -> f64 {
    let t0 = Instant::now();
    {
        let mut app = conn.appender("files").unwrap();
        for f in &corpus.files {
            app.append_row(duckdb::params![f.path, f.rev, f.bytes]).unwrap();
        }
    }
    {
        let mut app = conn.appender("nodes").unwrap();
        for n in &corpus.nodes {
            app.append_row(duckdb::params![
                n.path, n.kind, n.span_start, n.span_end, n.hpath, n.rev
            ])
            .unwrap();
        }
    }
    {
        let mut app = conn.appender("links").unwrap();
        for l in &corpus.links {
            app.append_row(duckdb::params![l.src, l.target, l.resolved])
                .unwrap();
        }
    }
    {
        let mut app = conn.appender("frontmatter").unwrap();
        for m in &corpus.fm {
            app.append_row(duckdb::params![m.path, m.key, m.value]).unwrap();
        }
    }
    conn.execute(
        "INSERT INTO roots VALUES (?, to_timestamp(?::BIGINT/1000.0))",
        duckdb::params![root, now_ms()],
    )
    .unwrap();
    t0.elapsed().as_secs_f64()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {}", table), [], |r| r.get(0))
        .unwrap_or(-1)
}

fn next_seq(conn: &Connection) -> i64 {
    conn.query_row("SELECT coalesce(max(seq),0)+1 FROM changes", [], |r| r.get(0))
        .unwrap_or(1)
}

// ---------------------------------------------------------------------------
// reference queries (embedded so the binary is self-contained; the same text
// lives in queries/*.sql for a stranger to run with the duckdb CLI).
// ---------------------------------------------------------------------------

const QUERIES: [(&str, &str); 6] = [
    ("q1_backlinks", include_str!("../queries/q1_backlinks.sql")),
    ("q2_tags", include_str!("../queries/q2_tags.sql")),
    ("q3_board", include_str!("../queries/q3_board.sql")),
    ("q4_history", include_str!("../queries/q4_history.sql")),
    ("q5_orphans", include_str!("../queries/q5_orphans.sql")),
    ("q6_viz", include_str!("../queries/q6_viz.sql")),
];

// run a query to completion (drain all rows) and return (row_count, elapsed_us).
fn run_query(conn: &Connection, sql: &str) -> (usize, f64) {
    let t0 = Instant::now();
    let mut stmt = conn.prepare(sql).expect("prepare");
    let mut rows = stmt.query([]).expect("query");
    let mut n = 0usize;
    while let Some(_r) = rows.next().expect("row") {
        n += 1;
    }
    (n, t0.elapsed().as_secs_f64() * 1e6)
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

fn cmd_build(db: &str) {
    let _ = fs::remove_file(db);
    let _ = fs::remove_file(format!("{}.wal", db));
    println!("# build {}", db);
    let (corpus, extract_secs, read_secs) = extract_all();
    let root = compute_root(&corpus.files);
    let conn = open_rw(db);
    conn.execute_batch(DDL).unwrap();
    let insert_secs = insert_corpus(&conn, &corpus, &root);
    conn.execute_batch("CHECKPOINT").unwrap();
    let db_bytes = file_size(db);
    let wal_bytes = file_size(&format!("{}.wal", db));
    let libver: String = conn
        .query_row("SELECT version()", [], |r| r.get(0))
        .unwrap_or_default();

    println!("libduckdb   {}", libver);
    println!("files       {}", corpus.files.len());
    println!("nodes       {}", corpus.nodes.len());
    println!("links       {}", corpus.links.len());
    println!("frontmatter {}", corpus.fm.len());
    println!("corpus_bytes {}", corpus.total_bytes);
    println!("root        {}", root);
    println!("extract_secs {:.4}  (of which file-read {:.4})", extract_secs, read_secs);
    println!("insert_secs  {:.4}", insert_secs);
    println!("total_secs   {:.4}", extract_secs + insert_secs);
    println!("db_bytes    {}", db_bytes);
    println!("wal_bytes_after_checkpoint {}", wal_bytes);
    println!(
        "size_ratio_vs_corpus {:.4}",
        db_bytes as f64 / corpus.total_bytes as f64
    );
}

fn cmd_bench(db: &str, warm_runs: usize, cold_runs: usize) {
    println!("# bench {}  warm_runs={} cold_runs={}", db, warm_runs, cold_runs);
    println!("query,mode,rows,p50_us,p99_us,min_us,max_us,runs");
    // WARM: one shared connection, repeated runs (plan+buffers hot).
    let conn = open_rw(db);
    for (name, sql) in QUERIES.iter() {
        let mut samples = Vec::with_capacity(warm_runs);
        let mut rows0 = 0;
        for i in 0..warm_runs {
            let (n, us) = run_query(&conn, sql);
            if i == 0 {
                rows0 = n;
            }
            samples.push(us);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "{},warm,{},{:.1},{:.1},{:.1},{:.1},{}",
            name,
            rows0,
            pct(&samples, 50.0),
            pct(&samples, 99.0),
            samples[0],
            samples[samples.len() - 1],
            warm_runs
        );
    }
    drop(conn);
    // COLD: fresh connection PER run (buffer pool cold each time).
    for (name, sql) in QUERIES.iter() {
        let mut samples = Vec::with_capacity(cold_runs);
        let mut rows0 = 0;
        for i in 0..cold_runs {
            let c = open_rw(db);
            let (n, us) = run_query(&c, sql);
            if i == 0 {
                rows0 = n;
            }
            samples.push(us);
            drop(c);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "{},cold,{},{:.1},{:.1},{:.1},{:.1},{}",
            name,
            rows0,
            pct(&samples, 50.0),
            pct(&samples, 99.0),
            samples[0],
            samples[samples.len() - 1],
            cold_runs
        );
    }
}

// one append = re-extract one real file + delta rows (change + refreshed child
// rows). Returns elapsed_us for the whole txn.
fn append_one(conn: &Connection, rx: &Rx, path: &str, seq: i64, mutate: bool) -> f64 {
    let orig = fs::read(path).unwrap_or_default();
    let mut text = String::from_utf8_lossy(&orig).into_owned();
    if mutate {
        // simulate an edit so file_rev changes each churn iteration
        text.push_str(&format!("\n<!-- churn {} -->\n", seq));
    }
    let mut tmp = Corpus::default();
    extract_one(rx, path, &text, &mut tmp);
    let new_rev = tmp.files[0].rev;

    let t0 = Instant::now();
    let old_rev: Option<i64> = conn
        .query_row("SELECT file_rev FROM files WHERE path=?", duckdb::params![path], |r| r.get(0))
        .ok();
    conn.execute_batch("BEGIN").unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO files VALUES (?,?,?)",
        duckdb::params![path, new_rev, text.len() as i64],
    )
    .unwrap();
    conn.execute("DELETE FROM nodes WHERE path=?", duckdb::params![path]).unwrap();
    conn.execute("DELETE FROM links WHERE src_path=?", duckdb::params![path]).unwrap();
    conn.execute("DELETE FROM frontmatter WHERE path=?", duckdb::params![path]).unwrap();
    for n in &tmp.nodes {
        conn.execute(
            "INSERT INTO nodes VALUES (?,?,?,?,?,?)",
            duckdb::params![n.path, n.kind, n.span_start, n.span_end, n.hpath, n.rev],
        )
        .unwrap();
    }
    for l in &tmp.links {
        conn.execute(
            "INSERT INTO links VALUES (?,?,?)",
            duckdb::params![l.src, l.target, l.resolved],
        )
        .unwrap();
    }
    for m in &tmp.fm {
        conn.execute(
            "INSERT INTO frontmatter VALUES (?,?,?)",
            duckdb::params![m.path, m.key, m.value],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO changes VALUES (?, to_timestamp(?::BIGINT/1000.0), ?, 'update', 'edit', ?, ?)",
        duckdb::params![seq, now_ms(), path, old_rev, new_rev],
    )
    .unwrap();
    conn.execute_batch("COMMIT").unwrap();
    t0.elapsed().as_secs_f64() * 1e6
}

fn cmd_append(db: &str, path_opt: Option<String>) {
    let conn = open_rw(db);
    let rx = Rx::new();
    let path = path_opt.unwrap_or_else(|| {
        conn.query_row("SELECT path FROM files LIMIT 1", [], |r| r.get::<_, String>(0))
            .unwrap()
    });
    let seq = next_seq(&conn);
    let us = append_one(&conn, &rx, &path, seq, true);
    println!("# append {}", db);
    println!("path {}", path);
    println!("seq  {}", seq);
    println!("append_us {:.1}", us);
    println!("changes_rows {}", count(&conn, "changes"));
}

fn cmd_churn(db: &str, n: usize) {
    let conn = open_rw(db);
    // let WAL grow so we can observe its behavior, then checkpoint explicitly.
    conn.execute_batch("SET checkpoint_threshold='1TB'").unwrap();
    let rx = Rx::new();
    // rotate over a few real hot paths to simulate an edit stream
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM files WHERE path LIKE '%LLM_WIKI%' OR path LIKE '%MISSION%' OR path LIKE '%README%' LIMIT 20")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|x| x.ok())
        .collect();
    let paths = if paths.is_empty() {
        conn.prepare("SELECT path FROM files LIMIT 20")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect()
    } else {
        paths
    };
    let wal_before = file_size(&format!("{}.wal", db));
    let db_before = file_size(db);
    let mut seq = next_seq(&conn);
    let mut samples = Vec::with_capacity(n);
    let t0 = Instant::now();
    for i in 0..n {
        let path = &paths[i % paths.len()];
        let us = append_one(&conn, &rx, path, seq, true);
        samples.push(us);
        seq += 1;
    }
    let total = t0.elapsed().as_secs_f64();
    let wal_grown = file_size(&format!("{}.wal", db));
    let db_mid = file_size(db);
    conn.execute_batch("CHECKPOINT").unwrap();
    let wal_after = file_size(&format!("{}.wal", db));
    let db_after = file_size(db);
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!("# churn {}  n={}", db, n);
    println!("appends            {}", n);
    println!("total_secs         {:.4}", total);
    println!("sustained_appends_per_sec {:.1}", n as f64 / total);
    println!("per_append_p50_us  {:.1}", pct(&samples, 50.0));
    println!("per_append_p99_us  {:.1}", pct(&samples, 99.0));
    println!("per_append_min_us  {:.1}", samples[0]);
    println!("per_append_max_us  {:.1}", samples[samples.len() - 1]);
    println!("db_bytes_before    {}", db_before);
    println!("db_bytes_mid       {}", db_mid);
    println!("db_bytes_after_ckpt {}", db_after);
    println!("wal_bytes_before   {}", wal_before);
    println!("wal_bytes_grown    {}", wal_grown);
    println!("wal_bytes_after_ckpt {}", wal_after);
    println!("changes_rows       {}", count(&conn, "changes"));
}

// child process: append change rows forever, autocommit each, record last
// committed seq to a progress file. Killed -9 by the parent mid-flight.
fn cmd_crashchild(db: &str, progress: &str) {
    let conn = open_rw(db);
    let rx = Rx::new();
    let path: String = conn
        .query_row("SELECT path FROM files LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let mut seq = next_seq(&conn);
    loop {
        let _ = append_one(&conn, &rx, &path, seq, true);
        // record the seq we just COMMITTED
        let mut f = fs::File::create(progress).unwrap();
        writeln!(f, "{}", seq).unwrap();
        f.flush().unwrap();
        drop(f);
        seq += 1;
    }
}

fn cmd_crash(db: &str) {
    // fresh copy so we don't disturb the main measurement DB
    let crash_db = format!("{}.crash", db);
    let _ = fs::remove_file(&crash_db);
    let _ = fs::remove_file(format!("{}.wal", &crash_db));
    fs::copy(db, &crash_db).unwrap();
    let progress = format!("{}.progress", crash_db);
    let _ = fs::remove_file(&progress);

    let exe = std::env::current_exe().unwrap();
    println!("# crash {}", crash_db);
    let mut child = Command::new(&exe)
        .args(["_crashchild", &crash_db, &progress])
        .spawn()
        .expect("spawn child");
    // let it commit a few thousand rows
    std::thread::sleep(Duration::from_secs(3));
    let committed_before_kill = fs::read_to_string(&progress)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(-1);
    // kill -9 (SIGKILL) mid-append
    child.kill().expect("kill -9");
    let _ = child.wait();
    println!("child killed (SIGKILL) mid-append");
    println!("last_committed_seq_before_kill {}", committed_before_kill);

    // reopen + verify (WAL replay on open)
    let reopen = Command::new("duckdb")
        .args([&crash_db, "-c", "SELECT count(*) AS changes_rows, max(seq) AS max_seq FROM changes;"])
        .output()
        .expect("reopen via duckdb CLI");
    let out = String::from_utf8_lossy(&reopen.stdout);
    let err = String::from_utf8_lossy(&reopen.stderr);
    println!("--- reopen stdout ---\n{}", out.trim());
    if !err.trim().is_empty() {
        println!("--- reopen stderr ---\n{}", err.trim());
    }
    println!("reopen_exit_ok {}", reopen.status.success());
    // verdict logic is left to REPORT.md; we print the raw evidence here.
}

// helper: open RW and hold the file for N seconds (writer-live window).
fn cmd_hold(db: &str, secs: u64) {
    let conn = open_rw(db);
    let _seq = next_seq(&conn); // touch it
    println!("HOLD rw open for {}s pid={}", secs, std::process::id());
    std::thread::sleep(Duration::from_secs(secs));
    drop(conn);
    println!("HOLD released");
}

// helper: try to open READ-ONLY from THIS (second) process; print success/err.
fn cmd_roopen(db: &str) {
    let cfg = Config::default().access_mode(AccessMode::ReadOnly).unwrap();
    match Connection::open_with_flags(db, cfg) {
        Ok(c) => match c.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)) {
            Ok(n) => println!("RO-OPEN ok files={}", n),
            Err(e) => println!("RO-OPEN opened-but-query-failed: {}", e),
        },
        Err(e) => println!("RO-OPEN failed: {}", e),
    }
}

// attach measurement: spawn a writer that HOLDS the file RW, then (a) a second
// spike process tries read-only open, (b) the duckdb CLI tries read-only attach.
fn cmd_attach(db: &str) {
    let attach_db = format!("{}.attach", db);
    let _ = fs::remove_file(&attach_db);
    let _ = fs::remove_file(format!("{}.wal", &attach_db));
    fs::copy(db, &attach_db).unwrap();
    let exe = std::env::current_exe().unwrap();
    println!("# attach {}", attach_db);

    // start writer holding RW for 12s
    let mut writer = Command::new(&exe)
        .args(["hold", &attach_db, "12"])
        .spawn()
        .expect("spawn writer");
    std::thread::sleep(Duration::from_secs(2)); // ensure the writer holds the lock

    // (a) second spike process, read-only open
    let ro = Command::new(&exe)
        .args(["ro-open", &attach_db])
        .output()
        .expect("ro-open");
    println!("--- (a) second spike process, read-only open ---");
    print!("{}", String::from_utf8_lossy(&ro.stdout));
    let e = String::from_utf8_lossy(&ro.stderr);
    if !e.trim().is_empty() {
        println!("stderr: {}", e.trim());
    }
    println!("exit_ok {}", ro.status.success());

    // (b) duckdb CLI, read-only open (-readonly) while writer live
    let cli = Command::new("duckdb")
        .args(["-readonly", &attach_db, "-c", "SELECT count(*) FROM files;"])
        .output()
        .expect("duckdb cli");
    println!("--- (b) duckdb CLI -readonly while writer live ---");
    println!("stdout: {}", String::from_utf8_lossy(&cli.stdout).trim());
    let ce = String::from_utf8_lossy(&cli.stderr);
    if !ce.trim().is_empty() {
        println!("stderr: {}", ce.trim());
    }
    println!("exit_ok {}", cli.status.success());

    // (c) duckdb CLI ATTACH (READ_ONLY) from a separate in-memory db
    let attach_sql = format!("ATTACH '{}' AS live (READ_ONLY); SELECT count(*) FROM live.files;", attach_db);
    let cli2 = Command::new("duckdb")
        .args(["-c", &attach_sql])
        .output()
        .expect("duckdb cli attach");
    println!("--- (c) duckdb CLI ATTACH (READ_ONLY) while writer live ---");
    println!("stdout: {}", String::from_utf8_lossy(&cli2.stdout).trim());
    let ce2 = String::from_utf8_lossy(&cli2.stderr);
    if !ce2.trim().is_empty() {
        println!("stderr: {}", ce2.trim());
    }
    println!("exit_ok {}", cli2.status.success());

    let _ = writer.wait();
    println!("writer released");

    // (d) control: after writer is gone, read-only open must succeed
    let ro2 = Command::new(&exe)
        .args(["ro-open", &attach_db])
        .output()
        .expect("ro-open control");
    println!("--- (d) CONTROL: read-only open after writer released ---");
    print!("{}", String::from_utf8_lossy(&ro2.stdout));
    println!("exit_ok {}", ro2.status.success());
}

// end-to-end root-stamp: write a batch stamping (root_before, root_after), then
// simulate a stale open (live root != latest root_after) => discard current
// tables + rebuild; the changes table SURVIVES (journal law).
fn cmd_rootstamp(db: &str) {
    let conn = open_rw(db);
    let rx = Rx::new();
    println!("# rootstamp {}", db);

    // live root from current files
    let live_files: Vec<FileRec> = {
        let mut v = Vec::new();
        let mut stmt = conn.prepare("SELECT path, file_rev, bytes FROM files").unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            v.push(FileRec {
                path: r.get(0).unwrap(),
                rev: r.get(1).unwrap(),
                bytes: r.get(2).unwrap(),
            });
        }
        v
    };
    let root_before = compute_root(&live_files);
    println!("step 1  live root_before = {}", root_before);

    // write a batch: append a change, then compute root_after
    let seq = next_seq(&conn);
    let path: String = conn.query_row("SELECT path FROM files LIMIT 1", [], |r| r.get(0)).unwrap();
    append_one(&conn, &rx, &path, seq, true);
    let live_files2: Vec<FileRec> = {
        let mut v = Vec::new();
        let mut stmt = conn.prepare("SELECT path, file_rev, bytes FROM files").unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            v.push(FileRec {
                path: r.get(0).unwrap(),
                rev: r.get(1).unwrap(),
                bytes: r.get(2).unwrap(),
            });
        }
        v
    };
    let root_after = compute_root(&live_files2);
    let bseq = conn.query_row("SELECT coalesce(max(seq),0)+1 FROM batches", [], |r| r.get::<_,i64>(0)).unwrap();
    conn.execute(
        "INSERT INTO batches VALUES (?,?,?, to_timestamp(?::BIGINT/1000.0))",
        duckdb::params![bseq, root_before, root_after, now_ms()],
    )
    .unwrap();
    println!("step 2  wrote batch seq={} root_after = {}", bseq, root_after);

    // simulate a STALE open: engine's true live root differs from stored root_after
    let fake_engine_root = "deadbeefdeadbeef";
    let stored_after: String = conn
        .query_row("SELECT root_after FROM batches ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let changes_survive = count(&conn, "changes");
    println!("step 3  simulate stale open: engine_live_root={} stored_root_after={}", fake_engine_root, stored_after);
    if fake_engine_root != stored_after {
        println!("        MISMATCH -> current tables stale; discard + rebuild");
        let files_before = count(&conn, "files");
        conn.execute_batch("DELETE FROM files; DELETE FROM nodes; DELETE FROM links; DELETE FROM frontmatter;").unwrap();
        println!("step 4  discarded current tables (files {} -> {})", files_before, count(&conn, "files"));
        // rebuild from corpus
        let (corpus, ex, _rd) = extract_all();
        let root_rebuilt = compute_root(&corpus.files);
        let ins = insert_corpus(&conn, &corpus, &root_rebuilt);
        println!("step 5  rebuilt from corpus: files={} extract_secs={:.3} insert_secs={:.3}", count(&conn,"files"), ex, ins);
        println!("        rebuilt root = {}", root_rebuilt);
        println!("step 6  changes table SURVIVED rebuild: rows before={} after={}", changes_survive, count(&conn,"changes"));
    }
    println!("rootstamp_demo_ok true");
}

// ---------------------------------------------------------------------------

fn usage() -> ! {
    eprintln!(
        "usage: duckdb-spike <cmd> <db> [args]\n\
         cmds: build | bench [warm cold] | append [path] | churn [n] |\n\
               crash | attach | rootstamp | hold <secs> | ro-open | _crashchild <progress>"
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        usage();
    }
    let cmd = args[1].as_str();
    let db = args[2].as_str();
    match cmd {
        "build" => cmd_build(db),
        "bench" => {
            let warm = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
            let cold = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(100);
            cmd_bench(db, warm, cold);
        }
        "append" => cmd_append(db, args.get(3).cloned()),
        "churn" => {
            let n = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10_000);
            cmd_churn(db, n);
        }
        "crash" => cmd_crash(db),
        "attach" => cmd_attach(db),
        "rootstamp" => cmd_rootstamp(db),
        "hold" => cmd_hold(db, args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10)),
        "ro-open" => cmd_roopen(db),
        "_crashchild" => cmd_crashchild(db, args.get(3).map(|s| s.as_str()).unwrap_or("progress.txt")),
        _ => usage(),
    }
}
