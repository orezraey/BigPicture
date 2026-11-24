use std::time::Duration;

use gpui_kit::profiler::{FrameEvent, FrameTimingCollector};
use gpui_kit::*;

const WARMUP_DRAWS: usize = 8;

/// Collects GPUI `Window::draw` timings during `--benchmark-scroll` runs to produce
/// reproducible numbers for `docs/benchmarks.md` with and without `gpui-fast`.
pub struct ScrollBenchmarkRecorder {
    collector: FrameTimingCollector,
    window_id: WindowId,
    seen_draws: usize,
    draw_samples: Vec<Duration>,
    /// `Window::present` (submissão à GPU), para comparar com o `encode_ms` do Rune.
    present_samples: Vec<Duration>,
    present_count: usize,
    /// Tempo de `ingest_loaded_covers` (BGRA + `RenderImage` + LRU), que roda fora do draw.
    ingest_total: Duration,
    ingest_batches: Vec<Duration>,
    ingest_covers: usize,
}

impl ScrollBenchmarkRecorder {
    pub fn new(window_id: WindowId) -> Self {
        gpui_kit::profiler::set_trace_enabled(true);
        Self {
            collector: FrameTimingCollector::new(),
            window_id,
            seen_draws: 0,
            draw_samples: Vec::with_capacity(512),
            present_samples: Vec::with_capacity(512),
            present_count: 0,
            ingest_total: Duration::ZERO,
            ingest_batches: Vec::with_capacity(256),
            ingest_covers: 0,
        }
    }

    pub fn poll(&mut self) {
        for event in self.collector.collect_unseen() {
            match event {
                FrameEvent::Draw(timing) if timing.window_id == self.window_id => {
                    self.seen_draws += 1;
                    if self.seen_draws > WARMUP_DRAWS {
                        self.draw_samples.push(timing.draw_duration());
                    }
                }
                FrameEvent::Present(timing) if timing.window_id == self.window_id => {
                    self.present_count += 1;
                    if self.seen_draws > WARMUP_DRAWS {
                        self.present_samples.push(timing.present_duration());
                    }
                }
                _ => {}
            }
        }
    }

    pub fn record_ingest(&mut self, elapsed: Duration, covers: usize) {
        self.ingest_total += elapsed;
        self.ingest_batches.push(elapsed);
        self.ingest_covers += covers;
    }

    pub fn print_report(&mut self, backend_name: &str, cache_hits: usize, cache_misses: usize) {
        self.poll();
        if self.draw_samples.is_empty() {
            println!("BENCHMARK_RESULT backend={backend_name} samples=0");
            return;
        }

        let mut sorted = self.draw_samples.clone();
        sorted.sort_unstable();

        let total: Duration = sorted.iter().copied().sum();
        let count = sorted.len();
        let mean_ms = (total.as_secs_f64() * 1000.0) / count as f64;
        let min_ms = sorted[0].as_secs_f64() * 1000.0;
        let p50_ms = percentile(&sorted, 0.50).as_secs_f64() * 1000.0;
        let p95_ms = percentile(&sorted, 0.95).as_secs_f64() * 1000.0;
        let p99_ms = percentile(&sorted, 0.99).as_secs_f64() * 1000.0;
        let max_ms = sorted[count - 1].as_secs_f64() * 1000.0;

        println!(
            "BENCHMARK_RESULT backend={backend_name} samples={count} presents={} mean_ms={mean_ms:.3} min_ms={min_ms:.3} p50_ms={p50_ms:.3} p95_ms={p95_ms:.3} p99_ms={p99_ms:.3} max_ms={max_ms:.3} cache_hits={cache_hits} cache_misses={cache_misses}",
            self.present_count
        );

        if !self.present_samples.is_empty() {
            let mut presents = self.present_samples.clone();
            presents.sort_unstable();
            let n = presents.len();
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            let mean = presents.iter().map(|d| ms(*d)).sum::<f64>() / n as f64;
            println!(
                "  present (submit ~ Rune encode): samples={n} mean_ms={mean:.3} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3}",
                ms(percentile(&presents, 0.50)),
                ms(percentile(&presents, 0.95)),
                ms(percentile(&presents, 0.99)),
                ms(presents[n - 1]),
            );
            let over_budget = sorted.iter().filter(|d| ms(**d) > 1000.0 / 240.0).count();
            println!("  draw over 4.17 ms: {over_budget}/{count}");
        }

        if !self.ingest_batches.is_empty() {
            let mut batches = self.ingest_batches.clone();
            batches.sort_unstable();
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            println!(
                "  ingest (fora do draw): total_ms={:.1} covers={} batches={} p95_batch_ms={:.3} max_batch_ms={:.3}",
                ms(self.ingest_total),
                self.ingest_covers,
                batches.len(),
                ms(percentile(&batches, 0.95)),
                ms(batches[batches.len() - 1]),
            );
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let last = sorted.len() - 1;
    let ix = (p.clamp(0.0, 1.0) * last as f64).round() as usize;
    sorted[ix.min(last)]
}
