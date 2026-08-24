//! PROBE (card `sql-set-config-cross-caller-starvation`, step 2) — throwaway.
//!
//! Question the advisor's (c) ruling put a bar on: on ONE `DatabaseInstance`,
//! can a per-call snapshot/restore actually clear a caller's GLOBAL `SET`
//! before the next caller's statement — for `memory_limit` AND at least one
//! other GLOBAL-scope setting, so the claim is about the CLASS?
//!
//! Prints rows; asserts nothing except that the probe ran. `-- --nocapture`.

use view::store::SqlStore;
use view::duckdb::Connection;

fn cur(c: &Connection, name: &str) -> String {
    c.query_row(
        &format!("SELECT current_setting('{name}')::VARCHAR"),
        [],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|e| format!("<ERR {e}>"))
}

/// (name, value, scope) for every setting DuckDB reports.
fn snapshot(c: &Connection) -> Vec<(String, String, String)> {
    let mut stmt = c
        .prepare(
            "SELECT name, coalesce(value::VARCHAR,'<NULL>'), coalesce(scope::VARCHAR,'<NULL>') \
             FROM duckdb_settings() ORDER BY name",
        )
        .expect("prepare duckdb_settings");
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })
    .expect("query")
    .collect::<Result<Vec<_>, _>>()
    .expect("rows")
}

#[test]
fn probe_global_set_restore_on_one_instance() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let file = dir.path().join("probe-sql.duckdb");
    let store = SqlStore::open(&file).expect("open");

    // ---- P0: does duckdb_settings() even carry a scope column, and how many
    // settings are GLOBAL? ----
    let base = snapshot(store.connection());
    let globals = base.iter().filter(|r| r.2 == "GLOBAL").count();
    println!("P0 settings_total={} global={}", base.len(), globals);
    for n in [
        "memory_limit",
        "threads",
        "temp_directory",
        "max_temp_directory_size",
        "default_null_order",
        "default_order",
        "preserve_insertion_order",
    ] {
        let row = base.iter().find(|r| r.0 == n);
        println!("P0 {n} = {row:?}");
    }

    // ---- P1: the leak, on ONE DatabaseInstance (two clones, as SqlStore::query
    // makes them). Call 1 sets; call 2 reads. ----
    let engine_temp = cur(store.connection(), "temp_directory");
    let engine_budget = cur(store.connection(), "max_temp_directory_size");
    println!("P1 engine temp_directory={engine_temp} max_temp_directory_size={engine_budget}");

    let c1 = store.connection().try_clone().expect("clone 1");
    c1.execute_batch("BEGIN").expect("begin 1");
    for s in [
        "SET memory_limit='1GB'",
        "SET threads=2",
        "SET default_null_order='NULLS_LAST'",
        "SET temp_directory='/tmp/mrd-hostile-probe'",
        "SET max_temp_directory_size='90%'",
    ] {
        match c1.execute_batch(s) {
            Ok(()) => println!("P1 caller {s} -> Success"),
            Err(e) => println!("P1 caller {s} -> ERR {e}"),
        }
    }
    c1.execute_batch("ROLLBACK").expect("rollback 1");
    drop(c1);

    let c2 = store.connection().try_clone().expect("clone 2");
    for n in [
        "memory_limit",
        "threads",
        "default_null_order",
        "temp_directory",
        "max_temp_directory_size",
    ] {
        println!("P1 after-call-1 (fresh clone) {n} = {}", cur(&c2, n));
    }

    // ---- P2: restore by diff against the snapshot taken BEFORE the statement.
    // This is the candidate mechanism. Note it re-SETs the ENGINE's value, so
    // the temp_directory/max_temp_directory_size trap (card
    // sql-spill-config-lockout) cannot bite the way a bare RESET would. ----
    let after = snapshot(&c2);
    let mut restored = 0;
    let mut failed = Vec::new();
    for (name, was, scope) in &base {
        if scope != "GLOBAL" {
            continue;
        }
        let now = after.iter().find(|r| &r.0 == name).map(|r| r.1.clone());
        if now.as_deref() == Some(was.as_str()) {
            continue;
        }
        println!("P2 drift {name}: {was:?} -> {now:?}");
        let esc = was.replace('\'', "''");
        match c2.execute_batch(&format!("SET \"{name}\"='{esc}';")) {
            Ok(()) => restored += 1,
            Err(e) => {
                println!("P2 restore FAILED {name}: {e}");
                failed.push(name.clone());
            }
        }
    }
    println!("P2 restored={restored} failed={failed:?}");

    // ---- P3: does a THIRD, fresh clone (the "next caller") see the engine's
    // values again? ----
    drop(c2);
    let c3 = store.connection().try_clone().expect("clone 3");
    for n in [
        "memory_limit",
        "threads",
        "default_null_order",
        "temp_directory",
        "max_temp_directory_size",
    ] {
        println!("P3 next-caller {n} = {}", cur(&c3, n));
    }
    println!(
        "P3 temp_directory back to engine value: {}",
        cur(&c3, "temp_directory") == engine_temp
    );
    println!(
        "P3 max_temp_directory_size back to engine value: {}",
        cur(&c3, "max_temp_directory_size") == engine_budget
    );

    // ---- P4: round-trip fidelity for the WHOLE GLOBAL class. If a setting
    // cannot be re-SET to the value duckdb_settings() reports for it, the
    // restore mechanism cannot restore it, and we must know its name now. ----
    drop(c3);
    let c4 = store.connection().try_clone().expect("clone 4");
    let mut bad = Vec::new();
    for (name, value, scope) in &base {
        if scope != "GLOBAL" || value == "<NULL>" {
            continue;
        }
        // lock_configuration is excluded from the SWEEP on purpose: setting it
        // true would lock every later statement in this probe. It gets its own
        // measurement in P5.
        if name == "lock_configuration" {
            continue;
        }
        let esc = value.replace('\'', "''");
        if let Err(e) = c4.execute_batch(&format!("SET \"{name}\"='{esc}';")) {
            bad.push(format!("{name} (= {value}): {e}"));
        }
    }
    println!("P4 self-SET failures: {}", bad.len());
    for b in &bad {
        println!("P4 NOT-RESTORABLE {b}");
    }

    // ---- P5: the escalation a restore mechanism cannot undo — a caller who
    // locks the configuration. Measured last, on its own connection, because it
    // is one-way for the whole instance. ----
    drop(c4);
    let c5 = store.connection().try_clone().expect("clone 5");
    match c5.execute_batch("SET lock_configuration=true;") {
        Ok(()) => println!("P5 caller SET lock_configuration=true -> Success"),
        Err(e) => println!("P5 caller SET lock_configuration=true -> ERR {e}"),
    }
    drop(c5);
    let c6 = store.connection().try_clone().expect("clone 6");
    match c6.execute_batch("SET memory_limit='2GB';") {
        Ok(()) => println!("P5 next caller SET memory_limit -> Success (lock did not leak)"),
        Err(e) => println!("P5 next caller SET memory_limit -> ERR {e}"),
    }
    println!("P5 lock_configuration now = {}", cur(&c6, "lock_configuration"));
}
