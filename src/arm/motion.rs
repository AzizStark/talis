//! Bounded, damped visual following; never alters the authoritative playback clock.
use std::time::Duration;

const POSITION_EPSILON: f64 = 0.00001;
const VELOCITY_EPSILON: f64 = 0.0001;
// Do not replay minutes of motion after suspension or a stalled UI thread.
const MAX_FRAME_SECONDS: f64 = 0.1;

#[derive(Clone, Debug)]
pub struct ArmMotion {
    position: f64,
    velocity: f64,
    target: Option<f64>,
    response_seconds: f64,
}
impl ArmMotion {
    pub fn new(position: f32, response_seconds: f64) -> Self {
        Self {
            position: normalized(position).unwrap_or(0.0),
            velocity: 0.0,
            target: None,
            response_seconds: if response_seconds.is_finite() {
                response_seconds.clamp(0.2, 2.0)
            } else {
                0.6
            },
        }
    }
    pub fn position(&self) -> f32 {
        self.position as f32
    }
    pub fn target(&self) -> Option<f32> {
        self.target.map(|target| target as f32)
    }
    /// True even for a stationary song target while the visual arm is catching up.
    pub fn needs_target(&self, target: f32) -> bool {
        normalized(target).is_some_and(|target| {
            (target - self.position).abs() > POSITION_EPSILON
                || self.velocity.abs() > VELOCITY_EPSILON
        })
    }
    pub fn moving(&self) -> bool {
        self.target
            .is_some_and(|target| self.needs_target(target as f32))
    }
    pub fn has_target(&self) -> bool {
        self.target.is_some()
    }
    /// Retargeting preserves position and velocity, including mid-flight reversals.
    pub fn retarget(&mut self, target: f32) {
        self.target = normalized(target);
        if self.target.is_none() {
            self.velocity = 0.0;
        }
    }
    /// Explicit safety/mode reset: freeze at the supplied visible pose, not the old target.
    pub fn hold(&mut self, position: f32) {
        if let Some(position) = normalized(position) {
            self.position = position;
        }
        self.velocity = 0.0;
        self.target = None;
    }
    pub fn advance(&mut self, elapsed: Duration) -> f32 {
        let Some(target) = self.target else {
            return self.position();
        };
        let seconds = elapsed.as_secs_f64().min(MAX_FRAME_SECONDS);
        if seconds == 0.0 {
            return self.position();
        }
        let omega = 12.0 / self.response_seconds;
        let speed_limit = 2.0 / self.response_seconds;
        // A >= omega * speed_limit preserves the critically damped approach
        // envelope from rest. Retargeted momentum still needs time to brake.
        let acceleration_limit = omega * speed_limit;
        let max_step = (self.response_seconds / 120.0).min(1.0 / 240.0);
        let steps = (seconds / max_step).ceil() as u32;
        let dt = seconds / f64::from(steps);
        for _ in 0..steps {
            let error = target - self.position;
            let acceleration = (omega * omega * error - 2.0 * omega * self.velocity)
                .clamp(-acceleration_limit, acceleration_limit);
            let velocity = (self.velocity + acceleration * dt).clamp(-speed_limit, speed_limit);
            self.position += (self.velocity + velocity) * 0.5 * dt;
            self.velocity = velocity;
            // Global physical limits, not the user's editable playback range:
            // a range contraction must not teleport a pose outside the new range.
            if !(0.0..=1.0).contains(&self.position) {
                self.position = self.position.clamp(0.0, 1.0);
                self.velocity = 0.0;
            }
            if (target - self.position).abs() <= POSITION_EPSILON
                && self.velocity.abs() <= VELOCITY_EPSILON
            {
                // Subpixel termination prevents a perpetual animation timer.
                self.position = target;
                self.velocity = 0.0;
                break;
            }
        }
        self.position()
    }
}
fn normalized(value: f32) -> Option<f64> {
    value.is_finite().then(|| f64::from(value.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(motion: &mut ArmMotion) {
        for _ in 0..600 {
            if !motion.moving() {
                break;
            }
            motion.advance(Duration::from_secs_f64(1.0 / 120.0));
        }
        assert!(!motion.moving(), "failed to settle: {motion:?}");
    }
    #[test]
    fn rest_to_target_accelerates_brakes_and_settles_without_bounce() {
        for (start, target) in [
            (0.0, 1.0),
            (1.0, 0.0),
            (0.2, 0.8),
            (0.8, 0.2),
            (0.5, 0.5001),
        ] {
            let mut m = ArmMotion::new(start, 0.6);
            m.retarget(target);
            assert_eq!(m.position(), start);
            assert_eq!(m.advance(Duration::ZERO), start);
            let direction = f64::from(target - start).signum();
            let mut previous = m.position;
            for _ in 0..600 {
                if !m.moving() {
                    break;
                }
                m.advance(Duration::from_secs_f64(1.0 / 120.0));
                assert!(
                    (m.position - previous) * direction >= -1e-12,
                    "bounced: {m:?}"
                );
                assert!(
                    (f64::from(target) - m.position) * direction >= -1e-12,
                    "overshot: {m:?}"
                );
                previous = m.position;
            }
            assert!(!m.moving());
            assert_eq!(m.position(), target);
            assert_eq!(m.velocity, 0.0);
        }
        let mut m = ArmMotion::new(0.0, 0.6);
        m.retarget(1.0);
        m.advance(Duration::from_millis(33));
        assert!(m.position() > 0.0 && m.position() < 0.05);
        let mut elapsed = 0.033;
        while m.moving() {
            m.advance(Duration::from_secs_f64(1.0 / 120.0));
            elapsed += 1.0 / 120.0;
            assert!(elapsed < 1.5);
        }
        assert!(elapsed > 0.3);
    }
    #[test]
    fn retarget_preserves_momentum_and_limits_speed_and_acceleration() {
        let mut m = ArmMotion::new(0.2, 0.6);
        m.retarget(0.9);
        m.advance(Duration::from_millis(100));
        let (position, velocity) = (m.position, m.velocity);
        m.retarget(0.1);
        assert_eq!((m.position, m.velocity), (position, velocity));
        assert!(velocity > 0.0);
        let dt = 1.0 / 240.0;
        let speed = 2.0 / 0.6;
        let acceleration = speed * 12.0 / 0.6;
        for _ in 0..250 {
            let (p, v) = (m.position, m.velocity);
            m.advance(Duration::from_secs_f64(dt));
            assert!((m.position - p).abs() <= speed * dt + POSITION_EPSILON);
            assert!((m.velocity - v).abs() <= acceleration * dt + VELOCITY_EPSILON);
            assert!(m.velocity.abs() <= speed);
        }
        settle(&mut m);
        assert_eq!(m.position(), 0.1);
    }
    #[test]
    fn frame_rates_agree_and_long_gaps_do_not_teleport_or_loop_unboundedly() {
        let mut reference = ArmMotion::new(0.0, 0.6);
        reference.retarget(1.0);
        for _ in 0..120 {
            reference.advance(Duration::from_secs_f64(1.0 / 240.0));
        }
        for fps in [15, 30, 60, 120, 240] {
            let mut m = ArmMotion::new(0.0, 0.6);
            m.retarget(1.0);
            for _ in 0..fps / 2 {
                m.advance(Duration::from_secs_f64(1.0 / f64::from(fps)));
            }
            // 15 FPS has no frame exactly at 0.5s; advance the residual.
            let residual = 0.5 - f64::from(fps / 2) / f64::from(fps);
            m.advance(Duration::from_secs_f64(residual));
            assert!(
                (m.position - reference.position).abs() < 0.001,
                "{fps} FPS: {m:?} vs {reference:?}"
            );
        }
        let mut m = ArmMotion::new(0.0, 0.6);
        m.retarget(1.0);
        let mut capped = m.clone();
        capped.advance(Duration::from_millis(100));
        m.advance(Duration::MAX);
        assert_eq!(m.position, capped.position);
        assert!(m.position() < 0.3);
        settle(&mut m);
    }
    #[test]
    fn invalid_inputs_freeze_and_hold_discards_an_old_destination() {
        let mut m = ArmMotion::new(f32::NAN, f64::NAN);
        assert_eq!(m.position(), 0.0);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            m.retarget(bad);
            assert!(!m.moving());
            assert_eq!(m.advance(Duration::MAX), 0.0);
        }
        m.retarget(9.0);
        settle(&mut m);
        assert_eq!(m.position(), 1.0);
        m.retarget(0.0);
        m.advance(Duration::from_millis(50));
        let held = m.position();
        m.hold(held);
        assert!(!m.has_target());
        assert_eq!(m.advance(Duration::MAX), held);
        m.hold(f32::NAN);
        assert_eq!(m.position(), held);
        m.retarget(-5.0);
        settle(&mut m);
        assert_eq!(m.position(), 0.0);
    }
    #[test]
    fn adversarial_retargets_remain_finite_bounded_and_eventually_idle() {
        for response in [0.2, 0.6, 2.0] {
            let mut m = ArmMotion::new(0.5, response);
            let mut seed = 7u64;
            for i in 0..10_000 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let target = (seed >> 32) as u32 as f64 / f64::from(u32::MAX);
                let (p, v) = (m.position, m.velocity);
                m.retarget(target as f32);
                assert_eq!((m.position, m.velocity), (p, v));
                let elapsed = match i % 9 {
                    0 => Duration::ZERO,
                    1 => Duration::from_secs(600),
                    _ => Duration::from_secs_f64(1.0 / [15.0, 30.0, 60.0, 120.0][i % 4]),
                };
                m.advance(elapsed);
                assert!(m.position.is_finite() && (0.0..=1.0).contains(&m.position));
                assert!(m.velocity.is_finite() && m.velocity.abs() <= 2.0 / response);
            }
            for target in [0.0, 1.0, 0.5] {
                m.retarget(target);
                settle(&mut m);
                assert_eq!(m.position(), target);
            }
        }
    }
    #[test]
    fn supported_responses_and_frame_rates_do_not_bounce_from_rest() {
        for response in [0.2, 0.6, 2.0] {
            for fps in [15, 30, 60, 120] {
                for start in [0.0, 0.1, 0.5, 0.9, 1.0] {
                    for target in [0.0, 0.1, 0.5, 0.9, 1.0] {
                        let mut m = ArmMotion::new(start, response);
                        m.retarget(target);
                        let direction = f64::from(target - start).signum();
                        for _ in 0..fps * 6 {
                            if !m.moving() {
                                break;
                            }
                            let p = m.position;
                            m.advance(Duration::from_secs_f64(1.0 / f64::from(fps)));
                            assert!((m.position - p) * direction >= -1e-12, "bounced: {m:?}");
                            assert!(
                                (f64::from(target) - m.position) * direction >= -1e-12,
                                "overshot: {m:?}"
                            );
                        }
                        assert!(!m.moving());
                        assert_eq!(m.position(), target);
                    }
                }
            }
        }
    }
}
