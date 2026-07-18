//! The five measurement axes: append latency, query latency (cold/warm),
//! queryability, crash/attach/contention probes, root-stamp lifecycle.

use crate::db;
use crate::extract;
use crate::queries::*;
use duckdb::{params, AccessMode, Config, Connection, Result};
use std::path::PathBuf;
use std::time::Instant;

// ---------- stats ----------

pub fn percentiles(mut v: Vec<u128>) -> (u128, u128, u128, u128, u128) {
    // returns (min, p50, p90, p99, max) in the same unit as input (ns)
    v.sort_unstable();
    let n = v.len();
    if n == 0 {
        return (0, 0, 0, 0, 0);
    }
    let at = |q: f64| v[((n as f64 * q) as usize).min(n - 1)];
    (v[0], at(0.50), at(0.90), at(0.99), v[n - 1])
}

fn fmt_us(ns: u128) -> String {
    format!("{:.1}", ns as f64 / 1000.0)
}

/// When `SPIKE_NO_CHECKPOINT=1`, hold the WAL open (no shutdown checkpoint, high
/// threshold) so a script can stat db+WAL sizes BEFORE an explicit CHECKPOINT.
/// Commits still fsync the WAL — durability is unchanged, only WAL folding is delayed.
fn maybe_hold_wal(conn: &Connection) -> Result<()> {
    if std::env::var("SPIKE_NO_CHECKPOINT").as_deref() == Ok("1") {
        conn.execute_batch(
            "PRAGMA disable_checkpoint_on_shutdown; SET checkpoint_threshold='1GB';",
        )?;
    }
    Ok(())
}

// ---------- axis 1: SPEED ----------

/// Cold full build: extract (timed) → insert (timed) → indexes (timed) → stamp.
/// Deletes any existing db+wal first so the number is a true cold build.
pub fn cmd_build(db_path: &str, corpus: &[PathBuf]) -> Result<()> {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}.wal"));

    let t_ex = Instant::now();
    let (facts, nfiles, bytes) = extract::extract_corpus(corpus);
    let extract_time = t_ex.elapsed();

    let conn = Connection::open(db_path)?;
    db::create_schema(&conn)?;
    let insert_time = db::insert_facts(&conn, &facts)?;

    let t_idx = Instant::now();
    db::create_indexes(&conn)?;
    let index_time = t_idx.elapsed();

    let root = db::compute_root(&facts);
    let total_rows = facts.files.len()
        + facts.nodes.len()
        + facts.links.len()
        + facts.frontmatter.len()
        + facts.files.len(); // + changes
    db::stamp_build(&conn, &root, total_rows as i64)?;
    conn.execute_batch("CHECKPOINT")?;

    println!("== BUILD ==");
    println!("corpus_files_read   {nfiles}");
    println!("corpus_bytes_read   {bytes}  ({:.1} MB)", bytes as f64 / 1048576.0);
    println!("extract_time_s      {:.3}", extract_time.as_secs_f64());
    println!("insert_time_s       {:.3}", insert_time.as_secs_f64());
    println!("index_time_s        {:.3}", index_time.as_secs_f64());
    println!(
        "build_total_s       {:.3}  (extract+insert+index)",
        extract_time.as_secs_f64() + insert_time.as_secs_f64() + index_time.as_secs_f64()
    );
    println!("rows_files          {}", facts.files.len());
    println!("rows_nodes          {}", facts.nodes.len());
    println!("rows_links          {}", facts.links.len());
    println!("rows_frontmatter    {}", facts.frontmatter.len());
    println!("rows_changes        {}", facts.files.len());
    println!("root                {root}");
    Ok(())
}

/// Discard + rebuild: drop all tables, recreate, re-extract, re-insert.
pub fn cmd_rebuild(db_path: &str, corpus: &[PathBuf]) -> Result<()> {
    let conn = Connection::open(db_path)?;
    let t = Instant::now();
    conn.execute_batch(
        "DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS nodes;
         DROP TABLE IF EXISTS links; DROP TABLE IF EXISTS frontmatter;
         DROP TABLE IF EXISTS changes; DROP TABLE IF EXISTS roots;
         DROP TABLE IF EXISTS batches;",
    )?;
    let drop_time = t.elapsed();

    let t_ex = Instant::now();
    let (facts, _n, _b) = extract::extract_corpus(corpus);
    let extract_time = t_ex.elapsed();

    db::create_schema(&conn)?;
    let insert_time = db::insert_facts(&conn, &facts)?;
    db::create_indexes(&conn)?;
    let root = db::compute_root(&facts);
    db::stamp_build(&conn, &root, facts.files.len() as i64)?;
    conn.execute_batch("CHECKPOINT")?;

    println!("== REBUILD (discard + rebuild) ==");
    println!("drop_time_s      {:.3}", drop_time.as_secs_f64());
    println!("extract_time_s   {:.3}", extract_time.as_secs_f64());
    println!("insert_time_s    {:.3}", insert_time.as_secs_f64());
    println!(
        "rebuild_total_s  {:.3}",
        drop_time.as_secs_f64() + extract_time.as_secs_f64() + insert_time.as_secs_f64()
    );
    println!("root             {root}");
    Ok(())
}

/// Incremental append benchmark: N single-file-change transactions.
/// Each append = 1 changes-row insert + 1 files-row update, one COMMIT (one WAL
/// fsync). Records per-append latency; reports p50/p99 and sustained appends/sec.
pub fn cmd_append_bench(db_path: &str, n: usize) -> Result<()> {
    let conn = Connection::open(db_path)?;
    maybe_hold_wal(&conn)?;

    // load real paths + current revs to cycle through
    let mut rows: Vec<(String, i64)> = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT path, file_rev FROM files")?;
        let mut q = stmt.query([])?;
        while let Some(r) = q.next()? {
            rows.push((r.get(0)?, r.get(1)?));
        }
    }
    assert!(!rows.is_empty(), "no files in db — build first");

    // SPIKE_HOTSET=N concentrates all appends onto the first N paths (so a real
    // history window exists for Q4); default = spread across all files.
    if let Ok(h) = std::env::var("SPIKE_HOTSET") {
        if let Ok(k) = h.parse::<usize>() {
            if k > 0 && k < rows.len() {
                rows.truncate(k);
            }
        }
    }

    let mut seq: i64 = conn
        .query_row("SELECT coalesce(max(seq),0) FROM changes", [], |r| r.get(0))?;

    let now = db::now_micros();
    let mut lat: Vec<u128> = Vec::with_capacity(n);
    let t_all = Instant::now();
    for i in 0..n {
        let (ref path, old_rev) = rows[i % rows.len()];
        let new_rev = old_rev ^ (i as i64).wrapping_mul(0x9e37_79b9_7f4a_7c15u64 as i64);
        seq += 1;
        let t = Instant::now();
        conn.execute_batch("BEGIN TRANSACTION")?;
        conn.execute(
            "INSERT INTO changes VALUES (?, make_timestamp(?), ?, 'file', 'modified', ?, ?)",
            params![seq, now, path, old_rev, new_rev],
        )?;
        conn.execute(
            "UPDATE files SET file_rev = ? WHERE path = ?",
            params![new_rev, path],
        )?;
        conn.execute_batch("COMMIT")?;
        lat.push(t.elapsed().as_nanos());
    }
    let wall = t_all.elapsed();
    let (mn, p50, p90, p99, mx) = percentiles(lat);

    println!("== APPEND BENCH (n={n}) ==");
    println!("delta_rows_per_append 2  (1 changes insert + 1 files update)");
    println!("wall_s               {:.3}", wall.as_secs_f64());
    println!("appends_per_sec      {:.0}", n as f64 / wall.as_secs_f64());
    println!("latency_us_min       {}", fmt_us(mn));
    println!("latency_us_p50       {}", fmt_us(p50));
    println!("latency_us_p90       {}", fmt_us(p90));
    println!("latency_us_p99       {}", fmt_us(p99));
    println!("latency_us_max       {}", fmt_us(mx));
    Ok(())
}

pub fn cmd_checkpoint(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    let t = Instant::now();
    conn.execute_batch("CHECKPOINT")?;
    println!("checkpoint_s {:.3}", t.elapsed().as_secs_f64());
    Ok(())
}

// ---------- axis 4: PERFORMANCE ----------

fn hot_target(conn: &Connection) -> Result<String> {
    conn.query_row(
        "SELECT target FROM links WHERE resolved GROUP BY target ORDER BY count(*) DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
}

fn hot_path(conn: &Connection) -> Result<String> {
    conn.query_row(
        "SELECT path FROM changes GROUP BY path ORDER BY count(*) DESC LIMIT 1",
        [],
        |r| r.get(0),
    )
}

/// Time a single execution of a query to completion (drains all rows).
fn time_query(conn: &Connection, sql: &str, param: Option<&str>) -> Result<(u128, usize)> {
    let mut stmt = conn.prepare(sql)?;
    let t = Instant::now();
    let mut rows = if let Some(p) = param {
        stmt.query(params![p])?
    } else {
        stmt.query([])?
    };
    let mut count = 0usize;
    while let Some(_r) = rows.next()? {
        count += 1;
    }
    Ok((t.elapsed().as_nanos(), count))
}

pub fn cmd_query_bench(db_path: &str, runs: usize) -> Result<()> {
    let warm = Connection::open(db_path)?;
    let target = hot_target(&warm)?;
    let path = hot_path(&warm)?;

    let specs: [(&str, &str, Option<String>); 6] = [
        ("Q1_backlinks", Q1_BACKLINKS, Some(target.clone())),
        ("Q2_tags", Q2_TAGS, None),
        ("Q3_board", Q3_BOARD, None),
        ("Q4_history", Q4_HISTORY, Some(path.clone())),
        ("Q5_orphans", Q5_ORPHANS, None),
        ("QC_vizfeed", QC_VIZFEED, None),
    ];

    println!("== QUERY BENCH (runs={runs}, cold=fresh-open first-exec) ==");
    println!("hot_target = {target}");
    println!("hot_path   = {path}");
    println!(
        "{:<14} {:>8} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "query", "rows", "cold_us", "warm_p50", "warm_p90", "warm_p99", "warm_max"
    );
    for (name, sql, p) in &specs {
        // cold: brand-new connection (buffer pool cold for this instance)
        let cold_conn = Connection::open_with_flags(
            db_path,
            Config::default().access_mode(AccessMode::ReadOnly)?,
        )?;
        let (cold_ns, rows) = time_query(&cold_conn, sql, p.as_deref())?;
        drop(cold_conn);

        // warm: same persistent connection, `runs` executions
        let mut lat = Vec::with_capacity(runs);
        for _ in 0..runs {
            let (ns, _c) = time_query(&warm, sql, p.as_deref())?;
            lat.push(ns);
        }
        let (_mn, p50, p90, p99, mx) = percentiles(lat);
        println!(
            "{:<14} {:>8} {:>10} {:>10} {:>10} {:>10} {:>8}",
            name,
            rows,
            fmt_us(cold_ns),
            fmt_us(p50),
            fmt_us(p90),
            fmt_us(p99),
            fmt_us(mx)
        );
    }
    Ok(())
}

// ---------- axis 5: QUERYABILITY ----------

pub fn cmd_queryability(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    let target = hot_target(&conn)?;
    let path = hot_path(&conn)?;

    let show = |title: &str, sql: &str, param: Option<&str>| -> Result<()> {
        println!("\n### {title}");
        println!("```sql\n{sql}\n```");
        let loc = sql.lines().count();
        let mut stmt = conn.prepare(sql)?;
        let mut rows = if let Some(p) = param {
            stmt.query(params![p])?
        } else {
            stmt.query([])?
        };
        // column_count is only valid AFTER execution — read it from the live stmt
        let ncol = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
        let mut n = 0usize;
        let mut sample: Vec<String> = Vec::new();
        while let Some(r) = rows.next()? {
            n += 1;
            if sample.len() < 3 {
                let mut cells = Vec::new();
                for c in 0..ncol {
                    // stringify defensively across column types
                    let v: String = r
                        .get::<_, String>(c)
                        .or_else(|_| r.get::<_, i64>(c).map(|x| x.to_string()))
                        .or_else(|_| r.get::<_, bool>(c).map(|x| x.to_string()))
                        .unwrap_or_else(|_| "<?>".into());
                    cells.push(v);
                }
                sample.push(cells.join(" | "));
            }
        }
        println!("lines_of_sql: {loc} | result_rows: {n}");
        for s in &sample {
            println!("  {s}");
        }
        Ok(())
    };

    println!("# Queryability inventory (param target={target}, path={path})");
    show("Q1 backlinks", Q1_BACKLINKS, Some(&target))?;
    show("Q2 tags", Q2_TAGS, None)?;
    show("Q3 board", Q3_BOARD, None)?;
    show("Q4 history", Q4_HISTORY, Some(&path))?;
    show("Q5 orphans", Q5_ORPHANS, None)?;
    show("QC vizfeed (ceiling)", QC_VIZFEED, None)?;
    Ok(())
}

// ---------- axis 3: WAL / crash / attach / contention ----------

/// Hold the file open read-write and commit appends in a loop until killed.
/// Prints READY + pid, then progress to stderr. Used by kill/attach scripts.
pub fn cmd_writer_hold(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    maybe_hold_wal(&conn)?;
    let mut rows: Vec<(String, i64)> = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT path, file_rev FROM files")?;
        let mut q = stmt.query([])?;
        while let Some(r) = q.next()? {
            rows.push((r.get(0)?, r.get(1)?));
        }
    }
    let mut seq: i64 = conn
        .query_row("SELECT coalesce(max(seq),0) FROM changes", [], |r| r.get(0))?;
    let now = db::now_micros();
    println!("READY pid={}", std::process::id());
    use std::io::Write;
    std::io::stdout().flush().ok();

    let mut i = 0usize;
    loop {
        let (ref path, old_rev) = rows[i % rows.len()];
        let new_rev = old_rev ^ (i as i64).wrapping_mul(0x9e37_79b9_7f4a_7c15u64 as i64);
        seq += 1;
        conn.execute_batch("BEGIN TRANSACTION")?;
        conn.execute(
            "INSERT INTO changes VALUES (?, make_timestamp(?), ?, 'file', 'modified', ?, ?)",
            params![seq, now, path, old_rev, new_rev],
        )?;
        conn.execute_batch("COMMIT")?;
        i += 1;
        if i % 2000 == 0 {
            eprintln!("writer committed seq={seq}");
        }
    }
}

/// Try to attach the live db read-only. Print exact success/error.
pub fn cmd_reader_attach(db_path: &str) -> Result<()> {
    match Connection::open_with_flags(
        db_path,
        Config::default().access_mode(AccessMode::ReadOnly)?,
    ) {
        Ok(c) => match c.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)) {
            Ok(n) => println!("READER-ATTACH: SUCCESS files={n}"),
            Err(e) => println!("READER-ATTACH: OPEN-OK QUERY-ERR: {e}"),
        },
        Err(e) => println!("READER-ATTACH: ERROR: {e}"),
    }
    Ok(())
}

/// Try to open the live db read-write (second writer). Print exact success/error.
pub fn cmd_writer_contend(db_path: &str) -> Result<()> {
    match Connection::open(db_path) {
        Ok(c) => match c.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)) {
            Ok(n) => println!("WRITER-CONTEND: SUCCESS (rw opened) files={n}"),
            Err(e) => println!("WRITER-CONTEND: OPEN-OK QUERY-ERR: {e}"),
        },
        Err(e) => println!("WRITER-CONTEND: ERROR: {e}"),
    }
    Ok(())
}

/// Reopen after a crash and report integrity: counts, max seq, latest root,
/// and that a real query still runs.
pub fn cmd_verify(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    let files = db::count(&conn, "files")?;
    let changes = db::count(&conn, "changes")?;
    let maxseq: i64 =
        conn.query_row("SELECT coalesce(max(seq),0) FROM changes", [], |r| r.get(0))?;
    let root = db::latest_root(&conn)?.unwrap_or_else(|| "<none>".into());
    // usability probe: run orphans query, count rows
    let orphans: i64 =
        conn.query_row(&format!("SELECT count(*) FROM ({Q5_ORPHANS})"), [], |r| r.get(0))?;
    println!("== VERIFY (reopen) ==");
    println!("files      {files}");
    println!("changes    {changes}");
    println!("max_seq    {maxseq}");
    println!("latest_root {root}");
    println!("orphans_query_rows {orphans}  (db usable after reopen)");
    Ok(())
}

// ---------- root-stamp lifecycle demo (step 8) ----------

/// End-to-end: build @root_a from a fixture → mutate one fixture file →
/// reopen, detect live-root ≠ stamped-root → discard + rebuild → verify @root_b.
pub fn cmd_rootstamp_demo(fixture: &str, db_path: &str) -> Result<()> {
    let corpus = vec![PathBuf::from(fixture)];
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{db_path}.wal"));

    println!("== ROOT-STAMP LIFECYCLE DEMO ==");
    println!("fixture = {fixture}");

    // 1. build @root_a
    let (facts_a, na, _b) = extract::extract_corpus(&corpus);
    let root_a = db::compute_root(&facts_a);
    let conn = Connection::open(db_path)?;
    db::create_schema(&conn)?;
    db::insert_facts(&conn, &facts_a)?;
    db::create_indexes(&conn)?;
    db::stamp_build(&conn, &root_a, facts_a.files.len() as i64)?;
    let changes_a = db::count(&conn, "changes")?;
    println!("1. built projection from {na} files; stamped root_a = {root_a}");
    println!("   changes journal rows = {changes_a}");

    // 2. mutate ONE fixture file on disk (append a marker line)
    let mutated: PathBuf;
    let old_rev: i64;
    {
        let mut all = Vec::new();
        extract::walk_md(&PathBuf::from(fixture), &mut all);
        all.sort();
        let p = all.first().expect("fixture is empty").clone();
        let bytes = std::fs::read(&p).unwrap_or_default();
        old_rev = extract::hash64(&bytes);
        let mut c = String::from_utf8_lossy(&bytes).into_owned();
        c.push_str("\n\n<!-- root-stamp demo mutation -->\n");
        std::fs::write(&p, c).ok();
        mutated = p;
    }
    println!("2. mutated fixture file: {}", mutated.to_string_lossy());

    // 3. on open: recompute LIVE root from bytes, compare to stamped root
    let stamped = db::latest_root(&conn)?.unwrap();
    let (facts_b, _n, _b) = extract::extract_corpus(&corpus);
    let live_root = db::compute_root(&facts_b);
    let new_rev = facts_b
        .files
        .iter()
        .find(|f| f.path == mutated.to_string_lossy())
        .map(|f| f.file_rev)
        .unwrap_or(0);
    println!("3. on open: stamped_root={stamped}  live_root={live_root}");
    if stamped == live_root {
        println!("   MATCH — no rebuild needed (unexpected in demo)");
        return Ok(());
    }
    println!("   MISMATCH → staleness self-refuting → discard + rebuild");

    // 4. discard ONLY the projection (changes journal + stamp trail survive)
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_links_target;
         DROP TABLE files; DROP TABLE nodes; DROP TABLE links; DROP TABLE frontmatter;",
    )?;
    db::create_schema_projection(&conn)?;
    db::insert_current(&conn, &facts_b)?;
    db::create_index_projection(&conn)?;
    // journal continues: append the observed modification (survives, not rewritten)
    let seq: i64 =
        conn.query_row("SELECT coalesce(max(seq),0)+1 FROM changes", [], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO changes VALUES (?, make_timestamp(?), ?, 'file', 'modified', ?, ?)",
        params![seq, db::now_micros(), mutated.to_string_lossy(), old_rev, new_rev],
    )?;
    // stamp the new build edge root_a → root_b
    let bseq = db::next_batch_seq(&conn)?;
    db::stamp_batch(&conn, bseq, Some(&root_a), &live_root, facts_b.files.len() as i64)?;
    conn.execute_batch("CHECKPOINT")?;

    // 5. verify: projection reflects root_b; journal retained root_a's rows + the new one
    let changes_b = db::count(&conn, "changes")?;
    let new_latest = db::latest_root(&conn)?.unwrap();
    println!("4. rebuilt projection from live bytes; stamped root_b = {live_root}");
    println!(
        "5. VERIFY: latest_root={new_latest} (== root_b: {})",
        new_latest == live_root
    );
    println!(
        "   changes journal rows = {changes_b} (was {changes_a}; +{} — journal SURVIVED rebuild)",
        changes_b - changes_a
    );

    // revert the fixture mutation so the demo is re-runnable
    {
        let c = std::fs::read_to_string(&mutated).unwrap_or_default();
        let reverted = c.replace("\n\n<!-- root-stamp demo mutation -->\n", "");
        std::fs::write(&mutated, reverted).ok();
    }
    println!("   (fixture mutation reverted — demo is idempotent)");
    Ok(())
}
