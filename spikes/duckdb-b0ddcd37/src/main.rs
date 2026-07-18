//! meridian-rs round-3 DuckDB spike (host team, b0ddcd37).
//! Measures DuckDB as the rebuildable materialized view on the real wiki corpus.
//!
//! Subcommands (see `just` recipes / REPORT.md for the measured invocations):
//!   build [db]                 cold full build (extract vs insert split)
//!   rebuild [db]               discard + full rebuild
//!   append-bench [db] [n]      incremental append latency (p50/p99, appends/s)
//!   checkpoint [db]            CHECKPOINT + timing
//!   query-bench [db] [runs]    5 reference queries + ceiling, cold vs warm
//!   queryability [db]          each query's SQL + sample rows + LOC
//!   writer-hold [db]           hold rw + commit in a loop until killed
//!   reader-attach [db]         try read-only open of a live db
//!   writer-contend [db]        try rw open of a live db (second writer)
//!   verify [db]                reopen + integrity/usability report
//!   rootstamp-demo [fix] [db]  build@root_a → mutate → mismatch → rebuild@root_b

mod bench;
mod db;
mod extract;
mod queries;

use std::path::PathBuf;

const CORPUS: [&str; 2] = [
    "/Users/Shared/projects/home-wiki",
    "/Users/Shared/projects/home-wiki-sessions",
];

fn corpus() -> Vec<PathBuf> {
    if let Ok(env) = std::env::var("SPIKE_CORPUS") {
        env.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()
    } else {
        CORPUS.iter().map(PathBuf::from).collect()
    }
}

fn arg<'a>(args: &'a [String], i: usize, default: &'a str) -> &'a str {
    args.get(i).map(String::as_str).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");
    let db = "data/wiki.duckdb";

    let res: duckdb::Result<()> = match cmd {
        "build" => bench::cmd_build(arg(&args, 2, db), &corpus()),
        "rebuild" => bench::cmd_rebuild(arg(&args, 2, db), &corpus()),
        "append-bench" => {
            let n: usize = arg(&args, 3, "10000").parse().unwrap_or(10000);
            bench::cmd_append_bench(arg(&args, 2, db), n)
        }
        "checkpoint" => bench::cmd_checkpoint(arg(&args, 2, db)),
        "query-bench" => {
            let runs: usize = arg(&args, 3, "200").parse().unwrap_or(200);
            bench::cmd_query_bench(arg(&args, 2, db), runs)
        }
        "queryability" => bench::cmd_queryability(arg(&args, 2, db)),
        "writer-hold" => bench::cmd_writer_hold(arg(&args, 2, db)),
        "reader-attach" => bench::cmd_reader_attach(arg(&args, 2, db)),
        "writer-contend" => bench::cmd_writer_contend(arg(&args, 2, db)),
        "verify" => bench::cmd_verify(arg(&args, 2, db)),
        "rootstamp-demo" => bench::cmd_rootstamp_demo(
            arg(&args, 2, "data/fixture"),
            arg(&args, 3, "data/rootstamp.duckdb"),
        ),
        "version" => {
            let conn = duckdb::Connection::open_in_memory().unwrap();
            let v: String = conn.query_row("PRAGMA version", [], |r| r.get(0)).unwrap();
            println!("libduckdb {v}");
            Ok(())
        }
        _ => {
            eprintln!("unknown command: {cmd}\nsee src/main.rs header for subcommands");
            std::process::exit(2);
        }
    };
    if let Err(e) = res {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}
