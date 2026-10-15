//! Preview benchmark sampling; not total application GPU utilization.
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Metrics {
    warmup: Option<Instant>,
    start: Option<Instant>,
    frames: usize,
    cpu: Vec<f64>,
    gpu: Vec<f64>,
}
fn p95(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len().saturating_sub(1) as f64) * 0.95).ceil() as usize]
}
impl Metrics {
    pub(super) fn sample(&mut self, cpu: f64, gpu: Option<f64>) -> Option<String> {
        self.sample_at(cpu, gpu, Instant::now())
    }

    fn sample_at(&mut self, cpu: f64, gpu: Option<f64>, now: Instant) -> Option<String> {
        let warmup = *self.warmup.get_or_insert(now + Duration::from_secs(2));
        if now < warmup {
            return None;
        }
        let start = *self.start.get_or_insert(now);
        self.frames += 1;
        self.cpu.push(cpu);
        if let Some(gpu) = gpu {
            self.gpu.push(gpu);
        }
        let seconds = now.duration_since(start).as_secs_f64();
        if seconds < 1.0 {
            return None;
        }
        let fps = self.frames as f64 / seconds;
        let cpu_avg = self.cpu.iter().sum::<f64>() / self.cpu.len() as f64;
        let cpu_p95 = p95(&mut self.cpu);
        let gpu_avg = if self.gpu.is_empty() {
            None
        } else {
            Some(self.gpu.iter().sum::<f64>() / self.gpu.len() as f64)
        };
        let gpu_p95 = if self.gpu.is_empty() {
            None
        } else {
            Some(p95(&mut self.gpu))
        };
        tracing::info!(
            fps,
            cpu_submit_ms = cpu_avg,
            cpu_p95_ms = cpu_p95,
            gpu_ms = gpu_avg,
            gpu_p95_ms = gpu_p95,
            "Scene renderer timings (not total application GPU usage)"
        );
        let gpu_text = gpu_avg.map_or_else(|| "pending".into(), |ms| format!("{ms:.2} ms"));
        let text = format!(
            "{fps:.1} FPS · GPU {gpu_text} · CPU submission {cpu_avg:.2} ms (p95 {cpu_p95:.2})"
        );
        *self = Self {
            warmup: Some(warmup),
            start: Some(now),
            ..Self::default()
        };
        Some(text)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentile_uses_sorted_samples() {
        assert_eq!(p95(&mut [3.0, 1.0, 2.0]), 3.0);
    }

    #[test]
    fn warmup_and_window_reset_preserve_sampling_behavior() {
        let now = Instant::now();
        let mut metrics = Metrics::default();
        assert!(metrics.sample_at(9.0, Some(9.0), now).is_none());
        assert!(
            metrics
                .sample_at(9.0, Some(9.0), now + Duration::from_secs(1))
                .is_none()
        );
        assert!(metrics.cpu.is_empty());
        assert!(
            metrics
                .sample_at(0.2, Some(5.0), now + Duration::from_secs(2))
                .is_none()
        );
        let text = metrics
            .sample_at(0.4, Some(7.0), now + Duration::from_secs(3))
            .unwrap();
        assert!(text.contains("GPU 6.00 ms"));
        assert!(text.contains("submission 0.30 ms"));
        assert!(metrics.cpu.is_empty() && metrics.gpu.is_empty());
        let text = metrics
            .sample_at(0.5, Some(2.0), now + Duration::from_secs(4))
            .unwrap();
        assert!(text.contains("GPU 2.00 ms"));
    }

    #[test]
    fn unavailable_gpu_sample_is_reported_as_pending() {
        let now = Instant::now();
        let mut metrics = Metrics::default();
        metrics.sample_at(0.1, None, now);
        metrics.sample_at(0.1, None, now + Duration::from_secs(2));
        let text = metrics
            .sample_at(0.1, None, now + Duration::from_secs(3))
            .unwrap();
        assert!(text.contains("GPU pending"));
    }
}
