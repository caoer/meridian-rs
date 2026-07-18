//! The tail-latency measurement path. Criterion's sampling is built for mean
//! estimation and systematically underestimates tails, so p99-shaped claims
//! (`Budget{class, p99_us}` is a product contract) run through hdrhistogram
//! here: N recorded iterations, exact percentiles, headline value = the
//! claim's percentile. Benches write [`Measurement`]s into a staging dir;
//! `bench-report` joins them against `claims.toml`.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use crate::corpus::workspace_target;

/// One claim measurement: `id` matches a `claims.toml` id, `value` is the
/// headline number in the claim's metric unit, `dist` carries the full
/// latency distribution when the measurement is latency-shaped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub id: String,
    pub value: f64,
    pub unit: String,
    pub samples: u64,
    #[serde(default)]
    pub dist: Option<LatencyDist>,
}

/// Microsecond percentiles from the hdrhistogram run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyDist {
    pub mean_us: f64,
    pub p50_us: f64,
    pub p90_us: f64,
    pub p99_us: f64,
    pub p999_us: f64,
    pub max_us: f64,
}

/// Nanosecond-resolution latency recorder, 3 significant figures, ceiling 60 s.
pub struct LatencyRun {
    hist: Histogram<u64>,
}

impl LatencyRun {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hist: Histogram::new_with_bounds(1, 60_000_000_000, 3)
                .expect("static bounds are valid"),
        }
    }

    /// Warm up, then record `iters` timed invocations.
    pub fn run(warmup: u32, iters: u32, mut f: impl FnMut()) -> Self {
        let mut this = Self::new();
        for _ in 0..warmup {
            f();
        }
        for _ in 0..iters {
            let start = Instant::now();
            f();
            this.record(start.elapsed());
        }
        this
    }

    pub fn record(&mut self, d: Duration) {
        let ns = u64::try_from(d.as_nanos())
            .unwrap_or(u64::MAX)
            .clamp(1, 60_000_000_000);
        self.hist.record(ns).expect("value clamped into bounds");
    }

    #[must_use]
    pub fn percentile_us(&self, q: f64) -> f64 {
        self.hist.value_at_quantile(q) as f64 / 1_000.0
    }

    #[must_use]
    pub fn dist(&self) -> LatencyDist {
        LatencyDist {
            mean_us: self.hist.mean() / 1_000.0,
            p50_us: self.percentile_us(0.50),
            p90_us: self.percentile_us(0.90),
            p99_us: self.percentile_us(0.99),
            p999_us: self.percentile_us(0.999),
            max_us: self.hist.max() as f64 / 1_000.0,
        }
    }

    #[must_use]
    pub fn samples(&self) -> u64 {
        self.hist.len()
    }

    /// Package as a claim measurement with the p99 headline.
    #[must_use]
    pub fn to_p99_measurement(&self, id: &str, unit: &str) -> Measurement {
        Measurement {
            id: id.to_owned(),
            value: self.percentile_us(0.99),
            unit: unit.to_owned(),
            samples: self.samples(),
            dist: Some(self.dist()),
        }
    }
}

impl Default for LatencyRun {
    fn default() -> Self {
        Self::new()
    }
}

/// Where benches stage claim measurements for `bench-report` to collect.
#[must_use]
pub fn staging_dir() -> PathBuf {
    workspace_target().join("bench-claims")
}

/// Write a measurement into staging (one JSON file per claim id; a re-run
/// overwrites — latest measurement wins).
pub fn record_measurement(m: &Measurement) -> io::Result<PathBuf> {
    let dir = staging_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", m.id));
    fs::write(&path, serde_json::to_string_pretty(m)?)?;
    Ok(path)
}

/// All staged measurements, sorted by id.
pub fn read_all() -> io::Result<Vec<Measurement>> {
    let dir = staging_dir();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(out); // no staging dir: nothing measured yet
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(text) = fs::read_to_string(&path)
            && let Ok(m) = serde_json::from_str::<Measurement>(&text)
        {
            out.push(m);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}
