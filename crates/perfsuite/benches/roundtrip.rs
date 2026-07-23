//! Seam bench: transport codec today, sidecar end-to-end when rungs land.
//!
//! # Metric contract (claims.toml)
//! - `transport.codec.ndjson_roundtrip_p99` — **LIVE day 1**: µs per
//!   single-frame encode+decode round-trip, p99 via the hdr path
//!   (`NdjsonCodec` is implemented). Recorded to staging on every run — this
//!   target is the end-to-end proof of the claims pipeline.
//! - `transport.codec.proto_roundtrip_p99` — **LIVE**: the typed-protobuf twin
//!   (`transport-proto`, length-delimited `pb::Frame`) over the SAME logical
//!   frame mix, so the two codecs stay comparable claim-to-claim.
//! - `roundtrip.op.p99` — dormant until rung 1+: NDJSON request in → response
//!   out through parse+assemble+project+codec, the latency Go actually feels.
//!
//! Throughput lanes are per-codec bytes (each codec's own encoded size), so
//! MiB/s numbers are honest per encoding; cross-codec comparison belongs at
//! the frames/s and p99 level.
//!
//! Hand-written `main` (not `criterion_main!`): criterion runs first, then the
//! hdr claim run records its measurement — both happen in smoke mode too, so
//! every CI run refreshes the staged measurement.

use std::hint::black_box;
use std::io::Write as _;

use criterion::{Criterion, Throughput, criterion_group};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};
use serde_json::json;
use transport::{Codec, Message, NdjsonCodec, Request, Response};
use transport_proto::pb;

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
                            "hpath": ["section", format!("h{k}")],
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

/// The typed twin of [`frames`]: same mix (toc/splice requests, 16-node
/// responses), same rng draw order, so payload values match the NDJSON set.
/// Node kind is `heading` — the untyped set's invented `"section"` kind has no
/// typed spelling, and heading is its wire-contract counterpart.
fn proto_frames(n: usize) -> Vec<pb::Frame> {
    let mut rng = ChaCha8Rng::seed_from_u64(0x6d65_7269_6469_616e);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let kind = match i % 4 {
            0 => pb::frame::Kind::Request(pb::Request {
                id: Some(i as u64),
                op: Some(pb::request::Op::Toc(pb::TocRequest {
                    path: format!("notes/d{:03}/f{:05}.md", i % 7, i),
                })),
            }),
            1 => pb::frame::Kind::Request(pb::Request {
                id: Some(i as u64),
                op: Some(pb::request::Op::Splice(pb::SpliceRequest {
                    path: format!("notes/f{i:05}.md"),
                    actor: None,
                    now: None,
                    receipt: None,
                    if_root: None,
                    dry: None,
                    force: None,
                    edits: vec![pb::Edit {
                        target: Some(pb::SecRef {
                            form: Some(pb::sec_ref::Form::Hpath(pb::HpathRef {
                                segs: vec![pb::HpathSeg {
                                    h: format!("h{}", i % 16),
                                    n: None,
                                }],
                            })),
                        }),
                        edit: Some(pb::EditShape {
                            shape: Some(pb::edit_shape::Shape::Match(pb::MatchEdit {
                                old: "replacement body text\nsecond line\n".into(),
                                new: "replacement body text\nsecond line\n".into(),
                            })),
                        }),
                        if_node_rev: Some(format!("{:016x}", rng.next_u64())),
                    }],
                })),
            }),
            _ => {
                let nodes: Vec<pb::Node> = (0..16)
                    .map(|k: u64| pb::Node {
                        kind: pb::NodeKind::Heading.into(),
                        span: Some(pb::Span {
                            start: k * 128,
                            end: (k + 1) * 128,
                        }),
                        text_prefix_16b: "sixteen bytes ok".into(),
                        hpath: ["section".into(), format!("h{k}")]
                            .into_iter()
                            .map(|h| pb::HpathSeg { h, n: None })
                            .collect(),
                        unterminated: None,
                        info: None,
                        node_rev: Some(format!("{:016x}", rng.next_u64())),
                    })
                    .collect();
                pb::frame::Kind::Response(pb::Response {
                    id: Some(i as u64),
                    ok: true,
                    body: Some(pb::response::Body::Nodes(pb::NodesResponse {
                        // the untyped set omits `path` in node responses; the
                        // proto3 default string encodes zero bytes — matched.
                        path: String::new(),
                        nodes,
                    })),
                })
            }
        };
        out.push(pb::Frame { kind: Some(kind) });
    }
    out
}

fn encode_all_proto(frames: &[pb::Frame]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.len() * 256);
    for frame in frames {
        transport_proto::encode(frame, &mut out).expect("encode to Vec cannot fail");
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

    let proto_msgs = proto_frames(1000);
    let proto_encoded = encode_all_proto(&proto_msgs);
    group.throughput(Throughput::Bytes(proto_encoded.len() as u64));
    group.bench_function("proto_encode_1000", |b| {
        b.iter(|| encode_all_proto(black_box(&proto_msgs)).len());
    });
    group.bench_function("proto_decode_1000", |b| {
        b.iter(|| {
            let mut input: &[u8] = black_box(&proto_encoded);
            let mut count = 0usize;
            while let Some(frame) = transport_proto::decode(&mut input).expect("valid frames") {
                black_box(&frame);
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

/// The typed twin of [`record_codec_claim`]: p99 of one proto frame through
/// encode+decode, same sample counts, same frame mix.
fn record_proto_codec_claim() {
    let frames = proto_frames(64);
    let mut buf = Vec::with_capacity(4096);
    let mut i = 0usize;
    let run = LatencyRun::run(1_000, 20_000, || {
        let frame = &frames[i % frames.len()];
        i += 1;
        buf.clear();
        transport_proto::encode(frame, &mut buf).expect("encode");
        let mut input: &[u8] = &buf;
        let decoded = transport_proto::decode(&mut input)
            .expect("decode")
            .expect("one frame");
        black_box(decoded);
    });
    let measurement = run.to_p99_measurement("transport.codec.proto_roundtrip_p99", "us");
    match record_measurement(&measurement) {
        Ok(path) => {
            let _ = writeln!(
                std::io::stderr(),
                "claim transport.codec.proto_roundtrip_p99: p99 {:.2} µs over {} samples → {}",
                measurement.value,
                measurement.samples,
                path.display()
            );
        }
        Err(e) => eprintln!("claim staging failed: {e}"),
    }
}

/// D-C1 (risk R2): the server-side `match{old,new}` scan p99 per edit, measured on
/// the REAL `model::validate_batch` path. The row was UNTESTED-BY-DESIGN while
/// `model::validate_splice` was `todo!()`; M4-VALIDATE landed `validate_batch`, which
/// arms the scan on the splice path this bench drives. One large section, one match
/// edit whose `old` sits at the section tail so the scan walks the full span — the
/// bounded cost D-C1 describes. Measured, never asserted (`threshold_source =
/// ruleset`): a local number renders MEASURED, never a false PASS/FAIL.
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
    record_proto_codec_claim();
    record_match_scan_claim();
}
