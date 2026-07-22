//! U8 gates: stdout streamed live AND stored content-addressed out-of-tree;
//! the S8 fsync-before-receipt ordering is structural; env KEYS never values
//! (S7); no implicit stdout→tree path exists in this module.

use std::io::Cursor;

use run::record::{self, ExecRecord, ExecRecordSink, RecordError, RunLog, StdoutRecord};
use sha2::{Digest, Sha256};

fn ws() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Task-card gate: stdout streamed LIVE and stored content-addressed
/// out-of-tree at `.meridian/runs/<invocation-id>.log`; hash + size are the
/// real content facts.
#[test]
fn tee_streams_live_and_logs_out_of_tree() {
    let ws = ws();
    let payload = b"line one\nline two\nbinary \x00\xff ok\n";
    let mut log = RunLog::create(ws.path(), "inv-0001").expect("create");
    let mut live: Vec<u8> = Vec::new();
    let n = record::stream(&mut Cursor::new(&payload[..]), &mut log, &mut live).expect("stream");

    // Live sink saw exactly the child's bytes.
    assert_eq!(live, payload);
    assert_eq!(n, payload.len() as u64);

    // The log landed out-of-tree at the addressed path with identical bytes.
    let on_disk = std::fs::read(ws.path().join(".meridian/runs/inv-0001.log")).expect("log file");
    assert_eq!(on_disk, payload);

    // Sealed facts carry the REAL content hash + size + address.
    let rec = log.seal().expect("seal");
    assert_eq!(
        rec,
        StdoutRecord {
            invocation: "inv-0001".to_owned(),
            log: ".meridian/runs/inv-0001.log".to_owned(),
            sha256: hex_sha256(payload),
            bytes: payload.len() as u64,
        }
    );
}

/// Multi-chunk stream (larger than the tee buffer): bytes, hash, and size
/// survive chunk boundaries.
#[test]
fn tee_survives_chunk_boundaries() {
    let ws = ws();
    let payload: Vec<u8> = (0u32..40_000).flat_map(u32::to_le_bytes).collect();
    let mut log = RunLog::create(ws.path(), "inv-big").expect("create");
    let mut live: Vec<u8> = Vec::new();
    let n = record::stream(&mut Cursor::new(&payload[..]), &mut log, &mut live).expect("stream");
    assert_eq!(n, payload.len() as u64);
    assert_eq!(live, payload);
    let rec = log.seal().expect("seal");
    assert_eq!(rec.bytes, payload.len() as u64);
    assert_eq!(rec.sha256, hex_sha256(&payload));
    let on_disk = std::fs::read(ws.path().join(".meridian/runs/inv-big.log")).expect("log file");
    assert_eq!(on_disk, payload);
}

/// Empty stdout is a valid record: zero bytes, the sha256 of the empty
/// string, and an existing (empty) log file.
#[test]
fn empty_stdout_records_cleanly() {
    let ws = ws();
    let log = RunLog::create(ws.path(), "inv-empty").expect("create");
    let rec = log.seal().expect("seal");
    assert_eq!(rec.bytes, 0);
    assert_eq!(rec.sha256, hex_sha256(b""));
    assert_eq!(
        std::fs::read(ws.path().join(".meridian/runs/inv-empty.log")).expect("log file"),
        Vec::<u8>::new()
    );
}

/// A reused invocation id refuses loudly — two runs must never interleave
/// one log.
#[test]
fn invocation_id_reuse_refuses() {
    let ws = ws();
    let _first = RunLog::create(ws.path(), "inv-dup").expect("create");
    let err = RunLog::create(ws.path(), "inv-dup").expect_err("collision");
    assert_eq!(
        err,
        RecordError::Collision {
            path: ".meridian/runs/inv-dup.log".to_owned(),
        }
    );
}

/// §9: the invocation id is caller-supplied, so it is validated as a single
/// path component before it names a file — traversal and separators refuse.
#[test]
fn hostile_invocation_ids_refuse() {
    let ws = ws();
    let overlong = "z".repeat(129);
    for id in ["", ".", "..", "../evil", "a/b", "a\\b", "a b", "x\0y", overlong.as_str()] {
        let err = RunLog::create(ws.path(), id).expect_err("hostile id");
        assert_eq!(
            err,
            RecordError::BadInvocationId { id: id.to_owned() },
            "id {id:?} must refuse"
        );
        // Nothing escaped: no file appeared beside the runs dir.
        assert!(
            !ws.path().join("evil").exists() && !ws.path().parent().unwrap().join("evil").exists()
        );
    }
}

/// S8 ordering is structural: the receipt-side facts type is minted ONLY by
/// `seal()` — this test pins the seam (constructing `StdoutRecord` by hand
/// here is possible only because the fields are pub for the receipt's
/// serde; dispatch code obtains one exclusively from `seal`). What we can
/// verify at runtime: after `seal`, the log file content is complete and
/// matches the minted facts exactly.
#[test]
fn seal_returns_facts_matching_the_durable_log() {
    let ws = ws();
    let mut log = RunLog::create(ws.path(), "inv-seal").expect("create");
    log.append(b"partial ").expect("append");
    log.append(b"then done\n").expect("append");
    assert_eq!(log.rel_path(), ".meridian/runs/inv-seal.log");
    let rec = log.seal().expect("seal");
    let on_disk = std::fs::read(ws.path().join(&rec.log)).expect("log file");
    assert_eq!(rec.bytes, on_disk.len() as u64);
    assert_eq!(rec.sha256, hex_sha256(&on_disk));
}

/// A dead live sink is a distinct failure class from a dead log: the runner
/// must tell a lost record (fatal to the receipt) from a lost audience.
#[test]
fn live_sink_failure_is_typed_distinct() {
    struct DeadPipe;
    impl std::io::Write for DeadPipe {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let ws = ws();
    let mut log = RunLog::create(ws.path(), "inv-pipe").expect("create");
    let err = record::stream(&mut Cursor::new(b"data".as_slice()), &mut log, &mut DeadPipe)
        .expect_err("dead pipe");
    assert!(matches!(err, RecordError::Live { .. }), "got {err:?}");
    // The log kept the chunk (log-first ordering): the record can still seal.
    let rec = log.seal().expect("seal");
    assert_eq!(rec.bytes, 4);
}

/// u6a seam gate: a group-SIGKILLed child's stdout pipe EOFs after its
/// buffered bytes drain — `stream` must terminate cleanly (no hang, no
/// error) and the record must seal. The child writes `ready` in one atomic
/// `printf` (< `PIPE_BUF`), so the captured content is exactly `""` or
/// `"ready"` — never a torn write.
#[test]
fn stream_terminates_on_pipe_eof_after_sigkill() {
    use std::process::{Command, Stdio};

    let ws = ws();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "printf ready; exec sleep 30"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdout = child.stdout.take().expect("piped stdout");
    // Let the printf land in the pipe buffer, then the u6a analogue of the
    // group SIGKILL (#21): kill + reap closes the write end.
    std::thread::sleep(std::time::Duration::from_millis(300));
    child.kill().expect("SIGKILL");
    child.wait().expect("reap");

    // Watchdog thread: a hang here is the failure mode under test.
    let ws_path = ws.path().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut log = RunLog::create(&ws_path, "inv-kill").expect("create");
        let mut live: Vec<u8> = Vec::new();
        let streamed = record::stream(&mut stdout, &mut log, &mut live);
        let sealed = log.seal();
        let _ = tx.send((streamed, sealed, live));
    });
    let (streamed, sealed, live) = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("stream hung after SIGKILL — pipe EOF not honored");

    let n = streamed.expect("clean EOF termination");
    let rec = sealed.expect("seal");
    assert!(live == b"ready" || live.is_empty(), "torn capture: {live:?}");
    assert_eq!(n, live.len() as u64);
    assert_eq!(rec.bytes, n);
    assert_eq!(rec.sha256, hex_sha256(&live));
    assert_eq!(
        std::fs::read(ws.path().join(&rec.log)).expect("log file"),
        live
    );
}

/// u6a seam gate: the live sink dies MID-stream (after accepting bytes) —
/// typed `Live` error, and the record survives with every byte read so far
/// (log-first ordering: the failing chunk landed in the log before the
/// sink refused it).
#[test]
fn live_sink_death_mid_stream_keeps_the_record() {
    struct DiesAfterFirstWrite {
        writes: usize,
    }
    impl std::io::Write for DiesAfterFirstWrite {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes > 1 {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let ws = ws();
    // > one tee chunk (8192), so the sink dies on the SECOND chunk.
    let payload = vec![0xabu8; 20_000];
    let mut log = RunLog::create(ws.path(), "inv-midpipe").expect("create");
    let mut sink = DiesAfterFirstWrite { writes: 0 };
    let err = record::stream(&mut Cursor::new(&payload[..]), &mut log, &mut sink)
        .expect_err("sink died mid-stream");
    assert!(matches!(err, RecordError::Live { .. }), "got {err:?}");

    // The record survives: both chunks read so far are in the log (16384 =
    // 2 × 8192 — the second landed log-first before the sink refused).
    let rec = log.seal().expect("seal");
    assert_eq!(rec.bytes, 16_384);
    let on_disk = std::fs::read(ws.path().join(&rec.log)).expect("log file");
    assert_eq!(on_disk, payload[..16_384]);
    assert_eq!(rec.sha256, hex_sha256(&payload[..16_384]));
}

/// S7 gate: the record swallows the FULL child environment and emits keys
/// only — no serialization of any record type may contain an env value.
#[test]
fn env_record_carries_keys_never_values() {
    let ws = ws();
    let log = RunLog::create(ws.path(), "inv-env").expect("create");
    let stdout = log.seal().expect("seal");
    let env = [
        ("HOME_WIKI", "/Users/zt/wiki"),
        ("API_TOKEN", "sk-SECRET-VALUE-9911"),
        ("HOME_WIKI", "/dup/ignored"),
    ];
    let rec = ExecRecord::new(0, stdout, env);
    assert_eq!(rec.env_keys, vec!["API_TOKEN".to_owned(), "HOME_WIKI".to_owned()]);
    let json = serde_json::to_string(&rec).expect("json");
    assert!(json.contains("API_TOKEN"));
    assert!(!json.contains("SECRET-VALUE"), "env value leaked: {json}");
    assert!(!json.contains("/Users/zt/wiki"), "env value leaked: {json}");
}

/// Task-card gate: the receipt-side record carries invocation id + exit
/// code + stdout hash + size + log address, and round-trips through serde
/// (the receipt line is machine-re-readable).
#[test]
fn exec_record_serde_shape() {
    let rec = ExecRecord::new(
        3,
        StdoutRecord {
            invocation: "inv-7".to_owned(),
            log: ".meridian/runs/inv-7.log".to_owned(),
            sha256: hex_sha256(b"x"),
            bytes: 1,
        },
        [("PATH", "/usr/bin")],
    );
    let json = serde_json::to_string(&rec).expect("json");
    let back: ExecRecord = serde_json::from_str(&json).expect("parse back");
    assert_eq!(back, rec);
    assert_eq!(back.exit_code, 3);
    assert_eq!(back.stdout.invocation, "inv-7");
    assert_eq!(back.stdout.log, ".meridian/runs/inv-7.log");
    assert_eq!(back.stdout.bytes, 1);
}

/// The S10 adoption seam: a receipt owner implements `ExecRecordSink` to
/// take the record — no executor file needs editing for the edge to bind.
#[test]
fn exec_record_sink_seam_binds() {
    #[derive(Default)]
    struct FakeReceipt {
        exec: Option<ExecRecord>,
    }
    impl ExecRecordSink for FakeReceipt {
        fn fill_exec(&mut self, exec: ExecRecord) {
            self.exec = Some(exec);
        }
    }
    let ws = ws();
    let stdout = RunLog::create(ws.path(), "inv-sink")
        .expect("create")
        .seal()
        .expect("seal");
    let mut receipt = FakeReceipt::default();
    receipt.fill_exec(ExecRecord::new(0, stdout, [] as [(&str, &str); 0]));
    let exec = receipt.exec.expect("filled");
    assert_eq!(exec.stdout.invocation, "inv-sink");
    assert!(exec.env_keys.is_empty());
}
