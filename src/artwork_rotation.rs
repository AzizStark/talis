//! Continuous clockwise phase, independent of track position/seeks and arm easing.
use std::time::{Duration, Instant};

pub const DEFAULT_RPM: f64 = 33.33;
pub const MAX_RPM: f64 = 120.0;

pub struct ArtworkRotation {
    angle: f64,
    last_frame: Option<Instant>,
    rotating: bool,
    rpm: f64,
}
impl Default for ArtworkRotation {
    fn default() -> Self {
        Self {
            angle: 0.0,
            last_frame: None,
            rotating: false,
            rpm: DEFAULT_RPM,
        }
    }
}
impl ArtworkRotation {
    /// Preserve phase without charging hidden time to the next visible frame.
    pub fn resume_at(&mut self, now: Instant) {
        self.last_frame = Some(now);
    }
    /// Account for time at the old speed, then change speed without resetting phase.
    pub fn set_speed(&mut self, now: Instant, rpm: f64) {
        let rpm = if rpm.is_finite() {
            rpm.clamp(0.0, MAX_RPM)
        } else {
            0.0
        };
        if self.rpm != rpm {
            self.advance(now);
            self.rpm = rpm;
        }
    }
    pub fn set_rotating(&mut self, now: Instant, rotating: bool) {
        if self.rotating != rotating {
            self.last_frame = Some(now);
        }
        self.rotating = rotating;
    }
    pub fn advance(&mut self, now: Instant) {
        if let Some(previous) = self.last_frame.replace(now) {
            if self.rotating {
                // A suspended process or long stall must not jump several turns on resume.
                let elapsed = now
                    .saturating_duration_since(previous)
                    .min(Duration::from_millis(100));
                let speed = std::f64::consts::TAU * self.rpm / 60.0;
                self.angle =
                    (self.angle + elapsed.as_secs_f64() * speed).rem_euclid(std::f64::consts::TAU);
            }
        }
    }
    pub fn angle(&self) -> f32 {
        self.angle as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_preserves_phase_and_discards_hidden_elapsed_time() {
        let now = Instant::now();
        let mut phase = ArtworkRotation::default();
        phase.set_rotating(now, true);
        phase.advance(now + Duration::from_millis(40));
        let before = phase.angle();
        let resume = now + Duration::from_secs(600);
        phase.resume_at(resume);
        phase.advance(resume);
        assert_eq!(phase.angle(), before);
        phase.advance(resume + Duration::from_millis(16));
        assert!(phase.angle() > before);
        assert!(phase.angle() - before < 0.1);
    }
    #[test]
    fn speed_changes_preserve_phase_and_zero_does_not_catch_up() {
        let now = Instant::now();
        let mut phase = ArtworkRotation::default();
        phase.set_speed(now, 60.0);
        phase.set_rotating(now, true);
        phase.advance(now + Duration::from_millis(50));
        let first = phase.angle();
        assert!((first - std::f32::consts::TAU * 0.05).abs() < 0.00001);
        phase.set_speed(now + Duration::from_millis(50), 120.0);
        assert_eq!(phase.angle(), first);
        phase.advance(now + Duration::from_millis(100));
        assert!((phase.angle() - std::f32::consts::TAU * 0.15).abs() < 0.00001);
        phase.set_speed(now + Duration::from_millis(100), 0.0);
        let stopped = phase.angle();
        phase.set_speed(now + Duration::from_secs(30), 45.0);
        assert_eq!(phase.angle(), stopped);
        phase.advance(now + Duration::from_millis(30_050));
        assert!((phase.angle() - stopped - std::f32::consts::TAU * 0.75 * 0.05).abs() < 0.00001);
        phase.set_rotating(now + Duration::from_millis(30_050), false);
        phase.set_speed(now + Duration::from_secs(40), 78.0);
        let paused = phase.angle();
        phase.set_rotating(now + Duration::from_secs(50), true);
        phase.advance(now + Duration::from_secs(50));
        assert_eq!(phase.angle(), paused);
    }
    #[test]
    fn invalid_runtime_speeds_cannot_poison_phase() {
        let now = Instant::now();
        let mut phase = ArtworkRotation::default();
        phase.set_rotating(now, true);
        for (index, rpm) in [f64::NAN, f64::INFINITY, -1.0, 121.0]
            .into_iter()
            .enumerate()
        {
            let at = now + Duration::from_millis(index as u64 * 50);
            phase.set_speed(at, rpm);
            phase.advance(at + Duration::from_millis(25));
            assert!(phase.angle().is_finite());
            assert!((0.0..=MAX_RPM).contains(&phase.rpm));
        }
    }
    #[test]
    fn changing_speed_accounts_for_elapsed_time_at_old_rate() {
        let now = Instant::now();
        let mut phase = ArtworkRotation::default();
        phase.set_speed(now, 60.0);
        phase.set_rotating(now, true);
        phase.set_speed(now + Duration::from_millis(50), 120.0);
        assert!((phase.angle() - std::f32::consts::TAU * 0.05).abs() < 0.00001);
        phase.advance(now + Duration::from_millis(100));
        assert!((phase.angle() - std::f32::consts::TAU * 0.15).abs() < 0.00001);
    }
    #[test]
    fn frame_rates_agree_pause_resume_preserves_phase_and_stalls_are_bounded() {
        let now = Instant::now();
        let mut expected: Option<f32> = None;
        for fps in [15, 30, 60, 120] {
            let mut phase = ArtworkRotation::default();
            phase.set_rotating(now, true);
            for frame in 1..=fps {
                phase.advance(now + Duration::from_secs_f64(frame as f64 / fps as f64));
            }
            if let Some(angle) = expected {
                assert!((phase.angle() - angle).abs() < 0.00001);
            }
            expected = Some(phase.angle());
            let end = now + Duration::from_secs(1);
            let angle = phase.angle();
            phase.set_rotating(end, false);
            phase.advance(end + Duration::from_secs(30));
            assert_eq!(phase.angle(), angle);
            phase.set_rotating(end + Duration::from_secs(31), true);
            phase.advance(end + Duration::from_secs(31));
            assert_eq!(phase.angle(), angle, "resume must not include idle time");
            phase.advance(end + Duration::from_secs(100));
            assert!((phase.angle() - angle).abs() < 0.35);
        }
    }
    #[test]
    fn wraparound_stays_finite_under_repeated_frames() {
        let now = Instant::now();
        let mut phase = ArtworkRotation::default();
        phase.set_rotating(now, true);
        for frame in 1..=100_000 {
            phase.advance(now + Duration::from_millis(frame * 33));
            assert!(
                phase.angle().is_finite() && (0.0..=std::f32::consts::TAU).contains(&phase.angle())
            );
        }
    }
}
