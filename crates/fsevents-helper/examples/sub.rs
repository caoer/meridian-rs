//! Manual probe for `mrd-fsevents-helper`. Times are `CLOCK_MONOTONIC`
//! seconds, comparable across processes on one machine.
//!
//! ```text
//! sub watch <root> [--secs S] [--socket P]   print `<t> <kind> <node> <path>` per record
//! sub latency <root> [--poke] [-n N] [--socket P]
//!                                            close()-to-record latency of new files
//! sub hold <file>                            write 100 bytes, sleep 2 s, close
//! sub twice <file> [<other>] [--gap-ms G]    write+close <file> twice G ms apart
//!                                            (default 1), creating <other> in between
//! sub flood <dir> <n>                        create n empty files
//! ```

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use fsevents_helper::wire::{Decoder, Record};
use fsevents_helper::{SOCKET_PATH, client, fse};

fn now() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one timespec into a local.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    #[allow(clippy::cast_precision_loss)]
    let t = ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9;
    t
}

fn kind_name(kind: u8) -> &'static str {
    match kind {
        fse::CREATE_FILE => "CREATE_FILE",
        fse::DELETE => "DELETE",
        fse::STAT_CHANGED => "STAT_CHANGED",
        fse::RENAME => "RENAME",
        fse::CONTENT_MODIFIED => "CONTENT_MODIFIED",
        fse::EXCHANGE => "EXCHANGE",
        fse::FINDER_INFO_CHANGED => "FINDER_INFO_CHANGED",
        fse::CREATE_DIR => "CREATE_DIR",
        fse::CHOWN => "CHOWN",
        fse::XATTR_MODIFIED => "XATTR_MODIFIED",
        fse::XATTR_REMOVED => "XATTR_REMOVED",
        fse::CLONE => "CLONE",
        fse::DROPPED => "DROPPED",
        _ => "?",
    }
}

fn opt(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn connect(args: &[String], root: &str) -> UnixStream {
    let socket = opt(args, "--socket").unwrap_or_else(|| SOCKET_PATH.into());
    match client::subscribe(Path::new(&socket), Path::new(root)) {
        Ok(s) => {
            println!("{:.6} subscribed {root}", now());
            s
        }
        Err(e) => {
            println!("{:.6} refused {root}: {e}", now());
            std::process::exit(1);
        }
    }
}

fn next(stream: &mut UnixStream, d: &mut Decoder) -> Option<Record> {
    let mut buf = [0u8; 16_384];
    loop {
        if let Some(r) = d.next_record() {
            return Some(r);
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                println!("{:.6} helper closed the stream", now());
                std::process::exit(0);
            }
            Ok(n) => d.push(&buf[..n]),
            Err(_) => return None,
        }
    }
}

fn watch(args: &[String]) {
    let mut s = connect(args, &args[0]);
    let secs: f64 = opt(args, "--secs").map_or(3600.0, |v| v.parse().expect("--secs"));
    let end = Instant::now() + Duration::from_secs_f64(secs);
    let mut d = Decoder::default();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        s.set_read_timeout(Some(left.max(Duration::from_millis(1))))
            .unwrap();
        if let Some(r) = next(&mut s, &mut d) {
            let node = format!("{:?}", r.node).to_lowercase();
            println!(
                "{:.6} {} {node} {}",
                now(),
                kind_name(r.kind),
                String::from_utf8_lossy(&r.path)
            );
        }
    }
}

fn latency(args: &[String]) {
    let root = PathBuf::from(&args[0]);
    let n: usize = opt(args, "-n").map_or(200, |v| v.parse().expect("-n"));
    let poke = args.iter().any(|a| a == "--poke");
    let mut stream = connect(args, &args[0]);
    let mut dec = Decoder::default();
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("lat-{i}");
        let mut file = File::create(root.join(&name)).unwrap();
        file.write_all(b"0123456789abcdef").unwrap();
        drop(file);
        let t0 = Instant::now();
        if poke {
            stream.write_all(b"p").unwrap();
        }
        loop {
            let r = next(&mut stream, &mut dec).expect("record");
            if r.kind == fse::DROPPED {
                println!("sample {i}: DROPPED");
            }
            if r.kind == fse::CONTENT_MODIFIED && r.path == name.as_bytes() {
                break;
            }
        }
        samples.push(t0.elapsed());
        sleep(Duration::from_millis(20));
    }
    samples.sort();
    let us = |t: Duration| t.as_secs_f64() * 1e6;
    println!(
        "latency poke={poke} n={n} min={:.0}us median={:.0}us p90={:.0}us max={:.0}us",
        us(samples[0]),
        us(samples[n / 2]),
        us(samples[n * 9 / 10]),
        us(samples[n - 1])
    );
}

fn write_close(path: &str) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(b"x").unwrap();
    drop(f);
    println!("{:.6} wrote+closed {path}", now());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((mode, rest)) = args.split_first() else {
        eprintln!("usage: sub watch|latency|hold|twice|flood ...");
        std::process::exit(2);
    };
    match mode.as_str() {
        "watch" => watch(rest),
        "latency" => latency(rest),
        "hold" => {
            let mut f = File::create(&rest[0]).unwrap();
            f.write_all(&[b'x'; 100]).unwrap();
            println!("{:.6} wrote 100 bytes, not closed", now());
            sleep(Duration::from_secs(2));
            println!("{:.6} closing", now());
            drop(f);
            println!("{:.6} closed", now());
        }
        "twice" => {
            let gap: u64 = opt(rest, "--gap-ms").map_or(1, |v| v.parse().expect("--gap-ms"));
            write_close(&rest[0]);
            sleep(Duration::from_millis(gap));
            if let Some(other) = rest.get(1).filter(|o| !o.starts_with("--")) {
                File::create(other).unwrap();
                println!("{:.6} created {other}", now());
            }
            write_close(&rest[0]);
        }
        "flood" => {
            let n: usize = rest[1].parse().expect("n");
            for i in 0..n {
                File::create(Path::new(&rest[0]).join(format!("flood-{i}"))).unwrap();
            }
            println!("{:.6} created {n} files", now());
        }
        _ => {
            eprintln!("unknown mode {mode}");
            std::process::exit(2);
        }
    }
}
