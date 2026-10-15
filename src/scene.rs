//! Animation state, independent of the graphics and desktop backend.
use crate::config::RenderSettings;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct SceneState {
    pub artwork_angle: f32,
    pub arm_progress: f32,
}
impl SceneState {
    pub fn advance(
        &mut self,
        elapsed: Duration,
        progress: f32,
        playing: bool,
        settings: &RenderSettings,
    ) {
        if playing && settings.rotate_artwork {
            let speed = std::f32::consts::TAU * (100.0 / 3.0) / 60.0;
            self.artwork_angle = (self.artwork_angle + elapsed.as_secs_f32() * speed)
                .rem_euclid(std::f32::consts::TAU);
        }
        if settings.animate_arm {
            self.arm_progress = progress.clamp(0.0, 1.0);
        }
    }
    pub fn needs_animation(playing: bool, settings: &RenderSettings) -> bool {
        playing && (settings.rotate_artwork || settings.animate_arm)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paused_or_static_scene_does_not_request_animation() {
        let settings = RenderSettings::default();
        assert!(SceneState::needs_animation(true, &settings));
        assert!(!SceneState::needs_animation(false, &settings));
        let settings = RenderSettings {
            rotate_artwork: false,
            animate_arm: false,
            ..settings
        };
        assert!(!SceneState::needs_animation(true, &settings));
    }
    #[test]
    fn disabled_arm_keeps_pose_while_artwork_rotates() {
        let settings = RenderSettings {
            animate_arm: false,
            ..Default::default()
        };
        let mut state = SceneState {
            artwork_angle: 0.0,
            arm_progress: 0.4,
        };
        state.advance(Duration::from_millis(100), 0.9, true, &settings);
        assert_eq!(state.arm_progress, 0.4);
        assert!(state.artwork_angle > 0.0);
    }
    #[test]
    fn rotation_is_frame_rate_independent_and_pauses() {
        let settings = RenderSettings::default();
        let mut one = SceneState::default();
        let mut many = SceneState::default();
        one.advance(Duration::from_secs(1), 0.0, true, &settings);
        for _ in 0..100 {
            many.advance(Duration::from_millis(10), 0.0, true, &settings);
        }
        assert!((one.artwork_angle - many.artwork_angle).abs() < 0.0001);
        let angle = one.artwork_angle;
        one.advance(Duration::from_secs(10), 0.0, false, &settings);
        assert_eq!(one.artwork_angle, angle);
    }
}
