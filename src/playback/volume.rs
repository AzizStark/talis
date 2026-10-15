//! Silent, player-local volume intent; no audio-server/system-volume fallback.
use super::MAX_SNAPSHOT_AGE;
use std::time::Instant;

pub const WHEEL_STEP: f64 = 0.05;
#[derive(Clone, Debug)]
pub struct Target {
    pub player: String,
    pub owner: String,
    pub value: f64,
    pub observed_at: Instant,
}
impl Target {
    pub fn fresh(&self, now: Instant) -> bool {
        self.value.is_finite()
            && self.value >= 0.0
            && self.player.starts_with("org.mpris.MediaPlayer2.")
            && self.owner.starts_with(':')
            && now.saturating_duration_since(self.observed_at) <= MAX_SNAPSHOT_AGE
    }
    pub fn same_player(&self, other: &Self) -> bool {
        self.player == other.player && self.owner == other.owner
    }
}
pub fn adjusted(value: f64, delta: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 || !delta.is_finite() || delta == 0.0 {
        return None;
    }
    // Do not turn a requested increase into a decrease for amplified players.
    if value >= 1.0 && delta > 0.0 {
        return None;
    }
    let target = (value + delta.clamp(-1.0, 1.0)).clamp(0.0, 1.0);
    ((target - value).abs() > 1e-9).then_some(target)
}
pub fn wheel_delta(dy: f64) -> Option<f64> {
    (dy.is_finite() && dy != 0.0).then(|| -dy.clamp(-20.0, 20.0) * WHEEL_STEP)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_direction_limits_and_malformed_values_are_safe() {
        assert_eq!(wheel_delta(-1.0), Some(0.05));
        assert_eq!(wheel_delta(1.0), Some(-0.05));
        assert_eq!(adjusted(0.98, 0.05), Some(1.0));
        assert_eq!(adjusted(0.02, -0.05), Some(0.0));
        assert_eq!(adjusted(0.0, -0.05), None);
        assert_eq!(adjusted(1.5, 0.05), None);
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(wheel_delta(v), None);
            assert_eq!(adjusted(v, 0.05), None);
            assert_eq!(adjusted(0.5, v), None);
        }
    }
    #[test]
    fn volume_requires_fresh_selected_enabled_player_but_not_song_timing() {
        let now = Instant::now();
        let mut f = super::super::PlaybackFeed {
            enabled: true,
            active: Some("org.mpris.MediaPlayer2.fixture".into()),
            volume: Some(Target {
                player: "org.mpris.MediaPlayer2.fixture".into(),
                owner: ":1.9".into(),
                value: 0.5,
                observed_at: now,
            }),
            ..Default::default()
        };
        assert!(f.snapshot.is_none());
        assert!(f.volume_target(now).is_some());
        assert!(
            f.volume_target(now + std::time::Duration::from_secs(7))
                .is_none()
        );
        f.enabled = false;
        assert!(f.volume_target(now).is_none());
        f.enabled = true;
        f.active = Some("org.mpris.MediaPlayer2.other".into());
        assert!(f.volume_target(now).is_none());
        f.select(None);
        assert!(f.volume.is_none());
    }
}
