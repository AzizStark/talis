//! Finite, retargetable marker motion; never sends or predicts volume commands.
use std::time::{Duration, Instant};
const TRAVEL: Duration = Duration::from_millis(180);
#[derive(Default)]
pub struct KnobMotion {
    angle: f32,
    from: f32,
    target: f32,
    start: Option<Instant>,
}
impl KnobMotion {
    pub fn angle(&self) -> f32 {
        self.angle
    }
    pub fn moving(&self) -> bool {
        self.start.is_some()
    }
    pub fn retarget(&mut self, volume: f64, now: Instant) {
        if !volume.is_finite() || volume < 0.0 {
            return;
        }
        // Keep the endpoint unwrapped: increasing volume traverses the full
        // clockwise turn rather than taking a shortcut at the camera-facing seam.
        let target = (volume.clamp(0.0, 1.0) * std::f64::consts::TAU) as f32;
        if (target - self.target).abs() < 1e-6 {
            return;
        }
        self.advance(now);
        self.from = self.angle;
        self.target = target;
        self.start = Some(now);
    }
    pub fn stop(&mut self) {
        self.target = self.angle;
        self.start = None;
    }
    pub fn advance(&mut self, now: Instant) {
        if let Some(start) = self.start {
            let t = (now.saturating_duration_since(start).as_secs_f32() / TRAVEL.as_secs_f32())
                .min(1.0);
            self.angle = self.from + (self.target - self.from) * t * t * (3.0 - 2.0 * t);
            if t >= 1.0 {
                self.angle = self.target;
                self.start = None;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_motion_reverses_continuously_and_settles() {
        let now = Instant::now();
        let mut m = KnobMotion::default();
        m.retarget(1.0, now);
        m.advance(now + TRAVEL / 2);
        let a = m.angle();
        assert!(a > 0.0);
        assert!(m.moving());
        m.retarget(0.0, now + TRAVEL / 2);
        assert_eq!(m.angle(), a);
        m.advance(now + TRAVEL * 2);
        assert!(!m.moving());
        assert_eq!(m.angle(), 0.0);
        m.retarget(f64::NAN, now);
        assert!(!m.moving());
        m.retarget(3.0, now + TRAVEL * 2);
        m.stop();
        assert!(!m.moving());
    }
    #[test]
    fn duplicate_poll_does_not_restart_and_long_stall_finishes() {
        let now = Instant::now();
        let mut m = KnobMotion::default();
        m.retarget(0.6, now);
        m.retarget(0.6, now + TRAVEL / 2);
        m.advance(now + Duration::from_secs(60));
        assert!(!m.moving());
        assert!((m.angle() - std::f32::consts::TAU * 0.6).abs() < 1e-6);
    }
    #[test]
    fn full_clockwise_turn_has_camera_facing_endpoints() {
        let now = Instant::now();
        let mut m = KnobMotion::default();
        for (step, volume) in [0.0, 0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
            let t = now + TRAVEL * (step as u32 * 2);
            let previous = m.angle();
            m.retarget(volume, t);
            m.advance(t + TRAVEL / 2);
            assert!(m.angle() >= previous);
            m.advance(t + TRAVEL);
            assert!((m.angle() - volume as f32 * std::f32::consts::TAU).abs() < 1e-6);
            assert!(!m.moving());
        }
        assert_eq!(m.angle(), std::f32::consts::TAU);
    }
}
