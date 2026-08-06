//! Seam bench: transport codec today, sidecar end-to-end when rungs land.
//!
//! # Metric contract (claims.toml)
//! - `transport.codec.ndjson_roundtrip_p99` — **LIVE day 1**: µs per single-frame
//!   encode+decode round-trip, p99 via the hdr path (`NdjsonCodec` is implemented).
//!   Recorded to staging on every run — this target is the end-to-end proof of the claims
//!   pipeline.
//! - `roundtrip.op.p99` — dormant until rung 1+: NDJSON request in → response out through
//!   parse+assemble+project+codec, the latency Go actually feels.
//!
//! Hand-written `main` (not `criterion_main!`): criterion runs first, then the hdr claim
//! run records its measurement — both happen in smoke mode too, so every CI run refreshes
//! the staged measurement.

use std::hint::black_box;
use std::io::Write as _;

use criterion::{Criterion, Throughput, criterion_group};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};
use serde_json::json;
use transport::{Codec, Message, NdjsonCodec, Request, Response};

use perfsuite::measure::{LatencyRun, record_measurement};

/// Deterministic mixed frame set: toc/read/splice-shaped requests and node-list
/// responses, wire-contract field shapes (hpath arrays, span pairs, 16-byte
/// text prefixes) without depending on `wire` — the codec is untyped by design.
fn frames(n: usize) -> Vec<Message> {
    let mut rng = ChaCha8Rng::seed_from_u64(0x6d65_7269_6469_616e);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let msg = match i % 4 {
            0 => Message::Request(Request {
                id: Some(i as u64),
                op: "toc".into(),
                params: obj(json!({"path": format!("notes/d{:03}/f{:05}.md", i % 7, i)})),
            }),
            1 => Message::Request(Request {
                id: Some(i as u64),
                op: "splice".into(),
                params: obj(json!({
                    "path": format!("notes/f{:05}.md", i),
                    "span": [rng.next_u64() % 4096, 4096 + rng.next_u64() % 4096],
                    "node_rev": format!("{:016x}", rng.next_u64()),
                    "body": "replacement body text\nsecond line\n",
                })),
            }),
            _ => {
                let nodes: Vec<serde_json::Value> = (0..16)
                    .map(|k| {
                        json!({
                            "hpath": [{"h": "section"}, {"h": format!("h{k}")}],
                            "span": [k * 128, (k + 1) * 128],
                            "kind": "section",
                            "node_rev": format!("{:016x}", rng.next_u64()),
                            "text_prefix_16b": "sixteen bytes ok",
                        })
                    })
                    .collect();
                Message::Response(Response {
                    id: Some(i as u64),
                    ok: true,
                    body: obj(json!({"nodes": nodes})),
                })
            }
        };
        out.push(msg);
    }
    out
}

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match v {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("json! object literal"),
    }
}

fn encode_all(msgs: &[Message]) -> Vec<u8> {
    let mut out = Vec::with_capacity(msgs.len() * 256);
    for msg in msgs {
        NdjsonCodec
            .encode(msg, &mut out)
            .expect("encode to Vec cannot fail");
    }
    out
}

fn bench_codec(c: &mut Criterion) {
    let msgs = frames(1000);
    let encoded = encode_all(&msgs);
    let mut group = c.benchmark_group("roundtrip");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("ndjson_encode_1000", |b| {
        b.iter(|| encode_all(black_box(&msgs)).len());
    });
    group.bench_function("ndjson_decode_1000", |b| {
        b.iter(|| {
            let mut input: &[u8] = black_box(&encoded);
            let mut count = 0usize;
            while let Some(msg) = NdjsonCodec.decode(&mut input).expect("valid frames") {
                black_box(&msg);
                count += 1;
            }
            count
        });
    });

    group.finish();
}

/// The hdr claim run: p99 of one frame through encode+decode.
fn record_codec_claim() {
    let msgs = frames(64);
    let mut buf = Vec::with_capacity(4096);
    let mut i = 0usize;
    let run = LatencyRun::run(1_000, 20_000, || {
        let msg = &msgs[i % msgs.len()];
        i += 1;
        buf.clear();
        NdjsonCodec.encode(msg, &mut buf).expect("encode");
        let mut input: &[u8] = &buf;
        let decoded = NdjsonCodec
            .decode(&mut input)
            .expect("decode")
            .expect("one frame");
        black_box(decoded);
    });
    let measurement = run.to_p99_measurement("transport.codec.ndjson_roundtrip_p99", "us");
    match record_measurement(&measurement) {
        Ok(path) => {
            let _ = writeln!(
                std::io::stderr(),
                "claim transport.codec.ndjson_roundtrip_p99: p99 {:.2} µs over {} samples → {}",
                measurement.value,
                measurement.samples,
                path.display()
            );
        }
        Err(e) => eprintln!("claim staging failed: {e}"),
    }
}

/// D-C1: the server-side `match{old,new}` scan p99 per edit, measured on the
/// real `model::validate_batch` path. One large section, one match edit whose
/// `old` sits at the section tail so the scan walks the full span. Measured,
/// never asserted (`threshold_source = ruleset`).
fn record_match_scan_claim() {
    // A single large section (fence-bomb / vault-1gb section-span shape): a heading
    // plus ~800 body lines, the unique `old` marker at the tail.
    let mut body = String::from("# Section\n\n");
    for i in 0..800u32 {
        body.push_str("padding paragraph line ");
        body.push_str(&i.to_string());
        body.push('\n');
    }
    body.push_str("the-unique-old-marker\n");
    let doc = model::build(body.clone(), syntax::parse(&body));
    let batch = model::SpliceRequest {
        engine: None,
        if_root: None,
        edits: vec![model::Edit {
            target: model::Ref::Hpath(vec![model::HpathSeg {
                h: "Section".into(),
                n: None,
            }]),
            edit: model::EditKind::Match {
                old: "the-unique-old-marker".into(),
                new: "the-new-marker".into(),
            },
            if_node_rev: None,
        }],
    };
    let run = LatencyRun::run(500, 8_000, || {
        let v = model::validate_batch(&doc, None, &batch, None);
        black_box(&v);
    });
    let measurement = run.to_p99_measurement("policy.splice.match_scan.p99", "us");
    match record_measurement(&measurement) {
        Ok(path) => {
            let _ = writeln!(
                std::io::stderr(),
                "claim policy.splice.match_scan.p99: p99 {:.2} µs over {} samples → {}",
                measurement.value,
                measurement.samples,
                path.display()
            );
        }
        Err(e) => eprintln!("claim staging failed: {e}"),
    }
}

criterion_group!(benches, bench_codec);

fn main() {
    benches();
    Criterion::default().configure_from_args().final_summary();
    record_codec_claim();
    record_match_scan_claim();
}
