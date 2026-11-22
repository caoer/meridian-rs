//! The barrier loop against the helper (`docs/fsevents-helper.md` §4): does a
//! fence ever arrive before the record of a write that completed before the
//! barrier was asked?
//!
//! ```sh
//! cargo run --release -p fsevents-helper --example barrier -- <root> <iterations> [socket]
//! ```
//!
//! Each iteration rewrites one member, removes one, creates one, then asks
//! the helper for a barrier and reads records until its fence. `early` counts
//! fences that arrived with one of the three member records missing and no
//! loss record before them; `misordered` counts member records out of the
//! order the writes were made in; `drops` counts barriers a loss record
//! preceded, which an engine answers with one floor pass. Several at once,
//! one root each, are several writers. The last line is the verdict.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fsevents_helper::{fse, wire};

const MEMBERS: u64 = 8;

struct Tally {
    early: u64,
    misordered: u64,
    drops: u64,
    timeouts: u64,
    lat_sum: Duration,
    lat_max: Duration,
}

#[allow(clippy::too_many_lines, reason = "one loop, read top to bottom")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: barrier <root> <iterations> [socket]");
        std::process::exit(2);
    }
    let root = std::fs::canonicalize(&args[1]).expect("root");
    let iterations: u64 = args[2].parse().expect("iterations");
    let socket = args.get(3).map_or_else(
        || PathBuf::from(fsevents_helper::SOCKET_PATH),
        PathBuf::from,
    );
    for m in 0..MEMBERS {
        std::fs::write(root.join(format!("m{m}.md")), "seed").expect("seed member");
        let _ = std::fs::remove_file(root.join(format!("x{m}.md")));
    }
    let mut stream = fsevents_helper::client::subscribe(&socket, &root).expect("subscribe");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut decoder = wire::Decoder::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut tally = Tally {
        early: 0,
        misordered: 0,
        drops: 0,
        timeouts: 0,
        lat_sum: Duration::ZERO,
        lat_max: Duration::ZERO,
    };
    let started = Instant::now();
    for i in 0..iterations {
        let rewritten = format!("m{}.md", i % MEMBERS);
        let created = format!("x{}.md", i % MEMBERS);
        let removed = format!("x{}.md", (i + MEMBERS - 1) % MEMBERS);
        std::fs::write(root.join(&rewritten), i.to_string()).expect("rewrite");
        let did_remove = std::fs::remove_file(root.join(&removed)).is_ok();
        std::fs::write(root.join(&created), i.to_string()).expect("create");
        let serial = i + 1;
        let asked = Instant::now();
        stream
            .write_all(&serial.to_le_bytes())
            .expect("barrier request");
        // The member records this barrier must have seen, in write order.
        let mut expect: Vec<(u8, &str)> = vec![(fse::CONTENT_MODIFIED, rewritten.as_str())];
        if did_remove {
            expect.push((fse::DELETE, removed.as_str()));
        }
        expect.push((fse::CREATE_FILE, created.as_str()));
        let mut next = 0usize;
        let mut lost = false;
        let sighted = 'wait: loop {
            while let Some(record) = decoder.next_record() {
                if record.kind == fse::DROPPED {
                    lost = true;
                    continue;
                }
                if record.kind == fse::FENCE {
                    if record.path == serial.to_le_bytes() {
                        break 'wait true;
                    }
                    continue;
                }
                let path = String::from_utf8_lossy(&record.path);
                if let Some(at) = expect
                    .iter()
                    .position(|(kind, rel)| *kind == record.kind && *rel == path)
                {
                    if at == next {
                        next += 1;
                    } else if at > next {
                        tally.misordered += 1;
                        println!(
                            "MISORDER iter={i} got={path} kind={} wanted index {next}",
                            record.kind
                        );
                        next = at + 1;
                    }
                }
            }
            match stream.read(&mut buf) {
                Ok(0) => {
                    println!("helper closed the subscription at iter={i}");
                    report(&tally, i, started);
                    std::process::exit(1);
                }
                Ok(n) => decoder.push(&buf[..n]),
                Err(_) => break 'wait false,
            }
        };
        let latency = asked.elapsed();
        if !sighted {
            tally.timeouts += 1;
            println!("TIMEOUT iter={i}");
        } else if lost {
            tally.drops += 1;
        } else {
            tally.lat_sum += latency;
            tally.lat_max = tally.lat_max.max(latency);
            if next < expect.len() {
                tally.early += 1;
                println!(
                    "ANOMALY iter={i} fence with {next} of {} member records ({:?} missing)",
                    expect.len(),
                    &expect[next..]
                );
            }
        }
        if (i + 1) % 50_000 == 0 {
            report(&tally, i + 1, started);
        }
    }
    report(&tally, iterations, started);
    cleanup(&root);
    std::process::exit(i32::from(
        tally.early + tally.misordered + tally.timeouts > 0,
    ));
}

fn report(tally: &Tally, iters: u64, started: Instant) {
    let clean = iters.saturating_sub(tally.drops + tally.timeouts).max(1);
    println!(
        "progress iters={iters} secs={:.1} early={} misordered={} drops={} timeouts={} lat_avg_ms={:.3} lat_max_ms={:.1}",
        started.elapsed().as_secs_f64(),
        tally.early,
        tally.misordered,
        tally.drops,
        tally.timeouts,
        (tally.lat_sum / u32::try_from(clean).unwrap_or(u32::MAX)).as_secs_f64() * 1000.0,
        tally.lat_max.as_secs_f64() * 1000.0,
    );
}

fn cleanup(root: &Path) {
    for m in 0..MEMBERS {
        let _ = std::fs::remove_file(root.join(format!("x{m}.md")));
    }
}
