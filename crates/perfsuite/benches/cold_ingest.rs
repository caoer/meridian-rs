//! `cold-ingest` — the real-world smoke detector (ZT ruling 2026-07-18): one
//! cold pass over a 1 GB-class corpus, wall-clock, NO repetition. The warm
//! criterion lane asks "did the parser get 5% slower?"; this lane asks "does
//! opening a huge vault still feel sane?" — a 10× swing is cache/OS noise we
//! accept, 100×+ means something is structurally broken, and one run is
//! enough to see that. Page-cache state is deliberately NOT controlled: this
//! measures the experience, not the microarchitecture.
//!
//! Two phases, each a single timed pass, each staging one claim:
//! - `ingest.cold.<profile>` — walk + read every file from disk
//! - `ingest.codec.ndjson.<profile>` — every file through `NdjsonCodec`
//!   encode+decode as a splice-shaped request (1 GB of real JSON cost)
//!
//! Message construction is pre-built OUTSIDE the timed window so the codec
//! claim measures encode+decode, not `String` clones. When rung 1 lands,
//! `syntax::parse` joins as a fourth phase and the claims keep their history.
//!
//! Hand-written `main`, `harness = false` (roundtrip.rs pattern): under the
//! PR smoke lane (`cargo bench -- --test`) it downshifts to a 50-file
//! subsample and stages nothing — hosted runners never generate 1 GB. The
//! full run is the perf lane's (or a local)
//! `cargo bench -p perfsuite --bench cold-ingest`.

#![allow(
    clippy::cast_precision_loss,
    reason = "byte counts to MB floats; perf reporting, not arithmetic"
)]

use std::hint::black_box;
use std::time::Instant;

use serde_json::json;
use transport::{Codec, Message, NdjsonCodec, Request};

use perfsuite::corpus;
use perfsuite::measure::{Measurement, record_measurement};
use perfsuite::profile::{Profile, Recipe};

const SMOKE_FILES: u32 = 50;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--test");
    let profile_arg = flag(&args, "--profile").unwrap_or_else(|| "vault-1gb".to_owned());
    let profile = Profile::resolve(&profile_arg).expect("profile resolves");
    let files = if smoke {
        Some(SMOKE_FILES)
    } else {
        flag(&args, "--files").map(|s| s.parse::<u32>().expect("--files is a number"))
    };
    let full_recipe = files.is_none_or(|n| n == profile.files);
    let recipe = Recipe::new(profile, 1, files);
    let suffix = recipe.profile.name.replace('-', "_");

    // Generate-or-reuse, outside every timed window: the claims are ingest
    // and codec cost, never corpusgen throughput.
    eprintln!("ensuring corpus {} (untimed)…", recipe.id());
    let dir = corpus::ensure(&recipe).expect("corpus generates");

    // Phase 1: cold read — walk + read every file from disk, once.
    let start = Instant::now();
    let loaded = corpus::load_files(&dir).expect("corpus loads");
    let elapsed = start.elapsed().as_secs_f64();
    let total: u64 = loaded.iter().map(|(_, text)| text.len() as u64).sum();
    black_box(&loaded);
    report(
        &format!("ingest.cold.{suffix}"),
        elapsed,
        total,
        loaded.len(),
        full_recipe,
    );

    let (secs, bytes, count) = ndjson_pass(&loaded);
    report(
        &format!("ingest.codec.ndjson.{suffix}"),
        secs,
        bytes,
        count,
        full_recipe,
    );
}

/// Phase 2: the corpus through the untyped NDJSON seam, one splice-shaped
/// frame per file. Returns (timed seconds, encoded bytes, frames decoded).
fn ndjson_pass(loaded: &[(String, String)]) -> (f64, u64, usize) {
    let msgs: Vec<Message> = loaded
        .iter()
        .enumerate()
        .map(|(i, (path, text))| {
            Message::Request(Request {
                id: Some(i as u64),
                op: "splice".into(),
                params: match json!({
                    "path": path,
                    "span": [0, text.len()],
                    "if_node_rev": "0000000000000000",
                    "text": text,
                }) {
                    serde_json::Value::Object(map) => map,
                    _ => unreachable!("json! object literal"),
                },
            })
        })
        .collect();
    let mut buf = Vec::with_capacity(512 * 1024);
    let mut encoded_bytes = 0u64;
    let mut decoded = 0usize;
    let start = Instant::now();
    for msg in &msgs {
        buf.clear();
        NdjsonCodec.encode(msg, &mut buf).expect("encode");
        encoded_bytes += buf.len() as u64;
        let mut input: &[u8] = &buf;
        let frame = NdjsonCodec
            .decode(&mut input)
            .expect("decode")
            .expect("one frame");
        black_box(&frame);
        decoded += 1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    assert_eq!(decoded, msgs.len(), "every frame decodes");
    (elapsed, encoded_bytes, decoded)
}

/// Print one phase line; stage the claim only on full-recipe runs — smoke and
/// subsampled numbers would lie under a full-corpus claim id.
fn report(id: &str, secs: f64, bytes: u64, files: usize, full_recipe: bool) {
    let ms = secs * 1_000.0;
    let mb = bytes as f64 / 1_000_000.0;
    println!(
        "{id}: {files} files, {mb:.1} MB in {ms:.0} ms ({:.0} MB/s)",
        mb / secs.max(f64::MIN_POSITIVE),
    );
    if full_recipe {
        let m = Measurement {
            id: id.to_owned(),
            value: ms,
            unit: "ms".to_owned(),
            samples: 1,
            dist: None,
        };
        let path = record_measurement(&m).expect("staging dir writable");
        println!("  staged → {}", path.display());
    }
}
