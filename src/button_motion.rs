//! Finite mechanical press/release motion. It never sends playback commands.
use std::time::{Duration, Instant};

pub const PRESS_TIME: Duration = Duration::from_millis(90);
pub const RELEASE_TIME: Duration = Duration::from_millis(140);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Rest,
    Down,
    Held,
    Up,
}
#[derive(Default)]
pub struct ButtonMotion {
    position: f32,
    from: f32,
    start: Option<Instant>,
    phase: Phase,
    release_after_press: bool,
}
impl ButtonMotion {
    pub fn position(&self) -> f32 {
        self.position
    }
    pub fn moving(&self) -> bool {
        matches!(self.phase, Phase::Down | Phase::Up)
    }
    fn begin(&mut self, phase: Phase, now: Instant) {
        self.from = self.position;
        self.start = Some(now);
        self.phase = phase;
    }
    pub fn press(&mut self, now: Instant) {
        self.advance(now);
        self.release_after_press = false;
        if self.phase != Phase::Held {
            self.begin(Phase::Down, now);
        }
    }
    /// A very short click still completes its downstroke before returning.
    pub fn release(&mut self, now: Instant) {
        self.advance(now);
        if self.phase == Phase::Down {
            self.release_after_press = true;
        } else if self.phase == Phase::Held {
            self.begin(Phase::Up, now);
        }
    }
    /// Invalidated captures/drag-outs return immediately from their current depth.
    pub fn cancel(&mut self, now: Instant) {
        self.advance(now);
        self.release_after_press = false;
        if matches!(self.phase, Phase::Down | Phase::Held) {
            self.begin(Phase::Up, now);
        }
    }
    pub fn advance(&mut self, now: Instant) {
        while self.moving() {
            let start = self.start.unwrap();
            let duration = if self.phase == Phase::Down {
                PRESS_TIME
            } else {
                RELEASE_TIME
            };
            let t = (now.saturating_duration_since(start).as_secs_f32() / duration.as_secs_f32())
                .min(1.0);
            let eased = t * t * (3.0 - 2.0 * t);
            let target = if self.phase == Phase::Down { 1.0 } else { 0.0 };
            self.position = self.from + (target - self.from) * eased;
            if t < 1.0 {
                break;
            }
            self.position = target;
            if self.phase == Phase::Down && self.release_after_press {
                self.release_after_press = false;
                self.begin(Phase::Up, start + duration);
            } else {
                self.phase = if target == 1.0 {
                    Phase::Held
                } else {
                    Phase::Rest
                };
                self.start = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hold_settles_without_a_continuous_animation_loop() {
        let now = Instant::now();
        let mut m = ButtonMotion::default();
        m.press(now);
        m.advance(now + PRESS_TIME / 2);
        assert!((m.position() - 0.5).abs() < 0.0001);
        m.advance(now + PRESS_TIME);
        assert_eq!(m.position(), 1.0);
        assert!(!m.moving());
        m.release(now + Duration::from_secs(10));
        m.advance(now + Duration::from_secs(10) + RELEASE_TIME);
        assert_eq!(m.position(), 0.0);
        assert!(!m.moving());
    }
    #[test]
    fn rapid_click_completes_press_then_release_and_catches_up_after_stalls() {
        let now = Instant::now();
        let mut m = ButtonMotion::default();
        m.press(now);
        m.release(now + Duration::from_millis(1));
        m.advance(now + PRESS_TIME);
        assert_eq!(m.position(), 1.0);
        assert!(m.moving());
        m.advance(now + Duration::from_secs(20));
        assert_eq!(m.position(), 0.0);
        assert!(!m.moving());
    }
    #[test]
    fn cancel_and_repress_are_continuous_bounded_and_independent() {
        let now = Instant::now();
        let mut a = ButtonMotion::default();
        let b = ButtonMotion::default();
        a.press(now);
        a.advance(now + PRESS_TIME / 2);
        let p = a.position();
        a.cancel(now + PRESS_TIME / 2);
        assert_eq!(a.position(), p);
        a.advance(now + PRESS_TIME);
        assert!(a.position() < p);
        let p = a.position();
        a.press(now + PRESS_TIME);
        assert_eq!(a.position(), p);
        a.release(now + PRESS_TIME);
        for ms in 90..1000 {
            a.advance(now + Duration::from_millis(ms));
            assert!((0.0..=1.0).contains(&a.position()));
        }
        assert_eq!(a.position(), 0.0);
        assert!(!a.moving());
        assert_eq!(b.position(), 0.0);
    }
}
