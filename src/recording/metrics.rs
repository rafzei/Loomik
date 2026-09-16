use serde::Serialize;
use std::time::Duration;

/// Fixed-memory latency histogram, 1 ms buckets; max retains outliers.
#[derive(Clone)]
pub struct Latency {
    buckets: [u64; 501],
    count: u64,
    total_ms: f64,
    max_ms: f64,
}
impl Default for Latency {
    fn default() -> Self {
        Self {
            buckets: [0; 501],
            count: 0,
            total_ms: 0.0,
            max_ms: 0.0,
        }
    }
}
#[derive(Serialize)]
pub struct LatencyReport {
    pub count: u64,
    pub mean_ms: f64,
    pub p95_ms_upper_bound: u64,
    pub max_ms: f64,
}
impl Latency {
    pub fn add(&mut self, duration: Duration) {
        let ms = duration.as_secs_f64() * 1000.0;
        self.count += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        self.buckets[(ms.ceil() as usize).min(500)] += 1;
    }
    pub fn report(&self) -> LatencyReport {
        let threshold = (self.count * 95).div_ceil(100);
        let mut count = 0;
        let p95 = self
            .buckets
            .iter()
            .position(|v| {
                count += v;
                count >= threshold
            })
            .unwrap_or(500);
        LatencyReport {
            count: self.count,
            mean_ms: self.total_ms / self.count.max(1) as f64,
            p95_ms_upper_bound: if p95 == 500 {
                self.max_ms.ceil() as u64
            } else {
                p95 as u64
            },
            max_ms: self.max_ms,
        }
    }
}
