//! Reuse a complete arm RGB/coverage patch only for subpixel playback motion.
//! This is a displacement estimate, not a bound on the nonlinear shader's RGB
//! error. Keep a safety margin, a short maximum age, and exact diagnostic paths.
use std::time::{Duration, Instant};

const MAX_HELD_PIXELS: f64 = 0.125;
const MAX_CATCHUP_PIXELS: f64 = 1.0;
const FLOW_SAFETY_FACTOR: f64 = 2.0;
const MAX_HOLD: Duration = Duration::from_millis(100);

pub(super) struct ArmCadence {
    dimensions: (u32, u32),
    distances: Vec<f64>,
    retained: Option<(f32, Instant)>,
}
impl ArmCadence {
    pub(super) fn new(dimensions: (u32, u32), bounds: &[f32]) -> Self {
        let mut distances = vec![0.0];
        for &bound in bounds {
            // Even zero-flow images can change shading between poses. Invalid
            // metadata disables reuse rather than making a cheap but stale arm.
            if !bound.is_finite() || bound < 0.0 {
                distances.clear();
                break;
            }
            distances.push(distances.last().unwrap() + f64::from(bound.max(1.0)));
        }
        Self {
            dimensions,
            distances,
            retained: None,
        }
    }
    fn position(&self, progress: f32) -> f64 {
        let count = self.distances.len();
        let (a, b, t) = crate::arm_assets::interpolation_samples(progress, count);
        self.distances[a] + (self.distances[b] - self.distances[a]) * f64::from(t)
    }
    /// `target` is supplied only for fresh, advancing music playback. Paused,
    /// stopped, stale, manual/demo and diagnostic frames use the exact request.
    /// Selection does not advance the retained timestamp: only a real arm/full
    /// cache update may commit it. Sleeve/button/knob redraws cannot reset age.
    pub(super) fn select(
        &self,
        requested: f32,
        target: Option<f32>,
        viewport: (i32, i32),
        now: Instant,
    ) -> f32 {
        let Some(target) = target.filter(|t| t.is_finite() && (0.0..=1.0).contains(t)) else {
            return requested;
        };
        let Some((retained, rendered_at)) = self.retained else {
            return requested;
        };
        if self.distances.len() < 2
            || !requested.is_finite()
            || !(0.0..=1.0).contains(&requested)
            || self.dimensions.0 == 0
            || self.dimensions.1 == 0
            || viewport.0 <= 0
            || viewport.1 <= 0
        {
            return requested;
        }
        let count = self.distances.len();
        let (a, _, t) = crate::arm_assets::interpolation_samples(requested, count);
        let (old_a, _, _) = crate::arm_assets::interpolation_samples(retained, count);
        // Never carry an approximation across an authored-pose boundary.
        if a != old_a || t == 0.0 || now.saturating_duration_since(rendered_at) >= MAX_HOLD {
            return requested;
        }
        let scale = (f64::from(viewport.0) / f64::from(self.dimensions.0))
            .max(f64::from(viewport.1) / f64::from(self.dimensions.1))
            * FLOW_SAFETY_FACTOR;
        let position = self.position(requested);
        if (self.position(target) - position).abs() * scale > MAX_CATCHUP_PIXELS
            || (self.position(retained) - position).abs() * scale >= MAX_HELD_PIXELS
        {
            requested
        } else {
            retained
        }
    }
    pub(super) fn commit(&mut self, progress: f32, now: Instant) {
        self.retained = progress
            .is_finite()
            .then(|| (progress.clamp(0.0, 1.0), now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(now: Instant) -> ArmCadence {
        let mut cadence = ArmCadence::new((2560, 1440), &[5.0; 32]);
        cadence.commit(0.51, now);
        cadence
    }
    #[test]
    fn accumulates_from_rendered_pose_not_previous_requested_frame() {
        let now = Instant::now();
        let c = fixture(now);
        for n in 1..=3 {
            let p = 0.51 + n as f32 * 0.0001;
            assert_eq!(c.select(p, Some(p), (2560, 1440), now), 0.51);
        }
        let p = 0.5104;
        assert_eq!(c.select(p, Some(p), (2560, 1440), now), p);
        assert_eq!(c.select(0.5096, Some(0.5096), (2560, 1440), now), 0.5096);
    }
    #[test]
    fn physical_pixel_scale_controls_high_dpi_and_small_viewports() {
        let c = fixture(Instant::now());
        let now = Instant::now();
        let p = 0.5103;
        assert_eq!(c.select(p, Some(p), (2560, 1440), now), 0.51);
        assert_eq!(c.select(p, Some(p), (3840, 2160), now), p);
        assert_eq!(c.select(p, Some(p), (5120, 2880), now), p);
        assert_eq!(c.select(0.5106, Some(0.5106), (1280, 720), now), 0.51);
    }
    #[test]
    fn pause_stop_stale_seek_and_authored_boundaries_remain_exact() {
        let now = Instant::now();
        let c = fixture(now);
        let p = 0.5101;
        assert_eq!(c.select(p, None, (2560, 1440), now), p);
        assert_eq!(c.select(p, Some(0.8), (2560, 1440), now), p);
        assert_eq!(c.select(p, Some(0.2), (2560, 1440), now), p);
        for p in [0.5, 0.53125, 0.532, 0.0, 1.0] {
            assert_eq!(c.select(p, Some(p), (2560, 1440), now), p);
        }
        assert_eq!(c.select(p, Some(f32::NAN), (2560, 1440), now), p);
        assert_eq!(c.select(p, Some(p), (0, 0), now), p);
        assert_eq!(c.select(p, Some(p), (2560, 1440), now + MAX_HOLD), p);
    }
    #[test]
    fn absent_invalid_and_zero_flow_are_safe() {
        let now = Instant::now();
        for bounds in [&[][..], &[f32::NAN][..], &[-1.0][..]] {
            let mut c = ArmCadence::new((2560, 1440), bounds);
            c.commit(0.51, now);
            assert_eq!(c.select(0.5101, Some(0.5101), (2560, 1440), now), 0.5101);
        }
        let mut c = ArmCadence::new((2560, 1440), &[0.0; 32]);
        assert_eq!(c.select(0.51, Some(0.51), (2560, 1440), now), 0.51);
        c.commit(f32::NAN, now);
        assert_eq!(c.select(0.51, Some(0.51), (2560, 1440), now), 0.51);
        c.commit(0.51, now);
        assert_eq!(c.select(0.5101, Some(0.5101), (2560, 1440), now), 0.51);
        assert_eq!(
            c.select(0.5101, Some(0.5101), (2560, 1440), now + MAX_HOLD),
            0.5101
        );
    }
    #[test]
    fn simulated_three_minute_playback_reduces_updates_but_fast_motion_does_not() {
        let now = Instant::now();
        for (width, height, fps) in [
            (1920, 1080, 30),
            (2560, 1440, 30),
            (3840, 2160, 30),
            (3840, 2160, 60),
        ] {
            let mut c = ArmCadence::new((2560, 1440), &[5.0; 32]);
            let mut updates = 0;
            for frame in 0..fps * 3 {
                let time = now + Duration::from_secs_f64(f64::from(frame) / f64::from(fps));
                let p = 0.51 + frame as f32 / (fps as f32 * 180.0);
                let selected = c.select(p, Some(p), (width, height), time);
                if c.retained.is_none_or(|(old, _)| old != selected) {
                    c.commit(selected, time);
                    updates += 1;
                }
            }
            eprintln!(
                "{width}x{height} {fps} FPS: {updates}/{} arm updates (synthetic bounds)",
                fps * 3
            );
            assert!(updates <= fps * 3 / 2);
            for frame in 1..=30 {
                let p = frame as f32 / 30.0;
                let selected = c.select(p, Some(1.0), (width, height), now);
                assert_eq!(selected, p);
                c.commit(selected, now);
            }
        }
    }
}
