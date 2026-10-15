//! Manual normalized arm position and a reversible demo; music mapping lives in app/state.
use std::time::Duration;
pub mod motion;

#[derive(Clone, Debug)]
pub struct ArmState {
    pub position: f32,
    pub start: f32,
    pub end: f32,
    pub sweep_seconds: f32,
    pub playing: bool,
    phase: f64,
}
impl Default for ArmState {
    fn default() -> Self {
        Self {
            position: 0.0,
            start: 0.0,
            end: 1.0,
            sweep_seconds: 15.0,
            playing: false,
            phase: 0.0,
        }
    }
}
impl ArmState {
    pub fn moving(&self) -> bool {
        self.playing && self.end > self.start
    }
    pub fn scrub(&mut self, value: f32) {
        self.playing = false;
        self.position = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            self.start
        };
        self.sync_phase();
    }
    pub fn set_range(&mut self, start: f32, end: f32) {
        self.start = if start.is_finite() {
            start.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.end = if end.is_finite() {
            end.clamp(self.start, 1.0)
        } else {
            1.0
        };
        self.position = self.position.clamp(self.start, self.end);
        self.sync_phase();
    }
    pub fn play(&mut self, active: bool) {
        let position = self.position.clamp(self.start, self.end);
        if position != self.position {
            self.position = position;
            self.sync_phase();
        }
        self.playing = active;
    }
    fn sync_phase(&mut self) {
        self.phase = if self.end > self.start {
            ((self.position - self.start) / (self.end - self.start)) as f64
        } else {
            0.0
        };
    }
    pub fn advance(&mut self, elapsed: Duration) {
        if !self.moving() {
            return;
        }
        self.phase = (self.phase + elapsed.as_secs_f64() / self.sweep_seconds.max(0.1) as f64)
            .rem_euclid(2.0);
        let progress = 1.0 - (self.phase - 1.0).abs();
        self.position = self.start + (self.end - self.start) * progress as f32;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sweep_hits_both_ends_and_reverses() {
        let mut arm = ArmState::default();
        arm.play(true);
        arm.advance(Duration::from_secs(15));
        assert_eq!(arm.position, 1.0);
        arm.advance(Duration::from_secs(15));
        assert_eq!(arm.position, 0.0);
    }
    #[test]
    fn manual_scrub_pauses_and_disabled_sweep_stays_idle() {
        let mut arm = ArmState::default();
        arm.play(true);
        arm.scrub(0.6);
        assert!(!arm.moving());
        arm.advance(Duration::from_secs(100));
        assert_eq!(arm.position, 0.6);
        arm.set_range(0.4, 0.4);
        arm.play(true);
        assert!(!arm.moving());
        arm.advance(Duration::from_secs(100));
        assert_eq!(arm.position, 0.4);
    }
    #[test]
    fn narrowed_range_and_frame_rate_independence() {
        let mut a = ArmState::default();
        a.set_range(0.2, 0.8);
        a.play(true);
        let mut b = a.clone();
        a.advance(Duration::from_secs(7));
        for _ in 0..70 {
            b.advance(Duration::from_millis(100));
        }
        assert!((a.position - b.position).abs() < 0.00001);
        assert!((0.2..=0.8).contains(&a.position));
    }
    #[test]
    fn pause_resume_preserves_return_direction() {
        let mut arm = ArmState::default();
        arm.play(true);
        arm.advance(Duration::from_secs(18));
        let position = arm.position;
        arm.play(false);
        arm.advance(Duration::from_secs(10));
        assert_eq!(arm.position, position);
        arm.play(true);
        arm.advance(Duration::from_secs(1));
        assert!(arm.position < position);
    }
}
