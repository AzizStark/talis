//! Monotonic playback clock and recoverable now-playing state; no renderer dependencies.
use std::time::{Duration, Instant};

#[cfg(feature = "desktop")]
pub mod mpris;
pub mod transport;
pub mod volume;

pub const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

/// A stale request may release its own slot, but never overwrite newer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestToken {
    generation: u64,
    id: u64,
}
#[derive(Default, Debug)]
pub struct RequestGate {
    generation: u64,
    pending: Option<RequestToken>,
}
impl RequestGate {
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn begin(&mut self, id: u64) -> Option<RequestToken> {
        if self.pending.is_some() {
            return None;
        }
        let token = RequestToken {
            generation: self.generation,
            id,
        };
        self.pending = Some(token);
        Some(token)
    }
    pub fn current(&self, token: RequestToken) -> bool {
        self.pending == Some(token) && token.generation == self.generation
    }
    pub fn finish(&mut self, token: RequestToken) -> bool {
        let valid = self.current(token);
        if self.pending == Some(token) {
            self.pending = None;
        }
        valid
    }
}

#[derive(Clone, Debug)]
pub struct PlaybackSnapshot {
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    pub observed_at: Instant,
    pub rate: f64,
}
impl PlaybackSnapshot {
    pub fn demo(now: Instant) -> Self {
        Self {
            playing: true,
            position: Duration::ZERO,
            duration: Duration::from_secs(180),
            observed_at: now,
            rate: 1.0,
        }
    }
    pub fn position_at(&self, now: Instant) -> Duration {
        let elapsed = if self.playing {
            now.saturating_duration_since(self.observed_at)
        } else {
            Duration::ZERO
        };
        let seconds = self.position.as_secs_f64() + elapsed.as_secs_f64() * self.rate;
        let seconds = if seconds.is_nan() {
            self.position.as_secs_f64()
        } else {
            seconds
        };
        Duration::try_from_secs_f64(seconds.clamp(0.0, self.duration.as_secs_f64()))
            .unwrap_or(self.duration)
    }
    pub fn progress_at(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 0.0;
        }
        (self.position_at(now).as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0) as f32
    }
}

#[derive(Clone, Debug)]
pub struct PlaybackFeed {
    pub enabled: bool,
    pub preferred: Option<String>,
    pub players: Vec<String>,
    pub active: Option<String>,
    pub status: Status,
    pub snapshot: Option<PlaybackSnapshot>,
    pub notice: String,
    pub artwork: Option<ArtworkSource>,
    /// Track identity remains available even when artwork/timing is missing.
    pub artwork_track: Option<String>,
    pub artwork_owner: Option<String>,
    /// Status for rotation, even when song timing is unavailable.
    pub artwork_playback: Option<(Status, Instant)>,
    /// Fresh capability/status snapshot, independent of track timing validity.
    pub transport: Option<transport::Target>,
    pub volume: Option<volume::Target>,
    pub control_pending: bool,
    pub control_notice: Option<String>,
    pub control_notice_target: Option<(String, String)>,
    /// Selection changes invalidate gestures and in-flight command completions.
    pub selection_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkSource {
    pub track: String,
    // Inline artwork can be large; frame/history clones must not copy its bytes.
    pub url: std::sync::Arc<str>,
}
impl Default for PlaybackFeed {
    fn default() -> Self {
        Self {
            enabled: false,
            preferred: None,
            players: Vec::new(),
            active: None,
            status: Status::Stopped,
            snapshot: None,
            notice: "Manual/demo mode".into(),
            artwork: None,
            artwork_track: None,
            artwork_owner: None,
            artwork_playback: None,
            transport: None,
            volume: None,
            control_pending: false,
            control_notice: None,
            control_notice_target: None,
            selection_revision: 0,
        }
    }
}
impl PlaybackFeed {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.enabled = enabled;
        self.invalidate(if enabled {
            "Waiting for a player…"
        } else {
            "Manual/demo mode"
        });
    }
    pub fn select(&mut self, name: Option<String>) {
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.preferred = name;
        self.active = None;
        self.invalidate("Waiting for selected player…");
    }
    pub fn invalidate(&mut self, notice: &str) {
        self.transport = None;
        self.volume = None;
        self.control_notice = None;
        self.control_notice_target = None;
        self.snapshot = None;
        self.artwork = None;
        self.artwork_track = None;
        self.artwork_owner = None;
        self.artwork_playback = None;
        self.notice = notice.into();
    }
    pub fn progress_at(&self, now: Instant) -> Option<f32> {
        let s = self.snapshot.as_ref()?;
        (self.enabled
            && self.status != Status::Stopped
            && !s.duration.is_zero()
            && now.saturating_duration_since(s.observed_at) <= MAX_SNAPSHOT_AGE)
            .then(|| s.progress_at(now))
    }
    pub fn command_target(
        &self,
        command: transport::Command,
        now: Instant,
    ) -> Option<&transport::Target> {
        self.transport.as_ref().filter(|t| {
            self.enabled
                && !self.control_pending
                && self.active.as_deref() == Some(t.player.as_str())
                && t.supports(command, now)
        })
    }
    /// Keep the sleeve when paused; timing/artwork availability is not playback
    /// status. In particular, duration-less streams can still show a cover.
    pub fn sleeve_visible(&self) -> bool {
        self.enabled
            && self.active.is_some()
            && self
                .artwork_playback
                .map_or(self.status, |(status, _)| status)
                != Status::Stopped
    }
    pub fn volume_target(&self, now: Instant) -> Option<&volume::Target> {
        self.volume.as_ref().filter(|t| {
            self.enabled && self.active.as_deref() == Some(t.player.as_str()) && t.fresh(now)
        })
    }
    /// Playback status is sufficient for a vinyl surface without cover artwork.
    pub fn surface_rotating(&self, now: Instant) -> bool {
        self.enabled
            && self.artwork_playback.is_some_and(|(status, observed)| {
                status == Status::Playing
                    && now.saturating_duration_since(observed) <= MAX_SNAPSHOT_AGE
            })
    }
    pub fn moving(&self, now: Instant) -> bool {
        self.progress_at(now).is_some()
            && self.status == Status::Playing
            && self.snapshot.as_ref().is_some_and(|s| {
                let p = s.position_at(now);
                if s.rate > 0.0 {
                    p < s.duration
                } else {
                    !p.is_zero()
                }
            })
    }
    pub fn label(&self, now: Instant) -> String {
        if !self.enabled {
            return "Playback tracking disabled".into();
        }
        let player = self
            .active
            .as_deref()
            .unwrap_or("No player")
            .trim_start_matches("org.mpris.MediaPlayer2.");
        match self
            .snapshot
            .as_ref()
            .filter(|_| self.progress_at(now).is_some())
        {
            Some(s) => format!(
                "{player} · {:?} · {} / {}",
                self.status,
                clock(s.position_at(now)),
                clock(s.duration)
            ),
            None => format!("{player} · {} · arm held", self.notice),
        }
    }
}
fn clock(time: Duration) -> String {
    let seconds = time.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sleeve_visibility_distinguishes_pause_stop_and_absent_player() {
        let mut feed = PlaybackFeed::default();
        assert!(!feed.sleeve_visible());
        feed.enabled = true;
        feed.active = Some("org.mpris.MediaPlayer2.fixture".into());
        let now = Instant::now();
        for status in [
            Status::Playing,
            Status::Paused,
            Status::Stopped,
            Status::Paused,
        ] {
            feed.artwork_playback = Some((status, now));
            assert_eq!(feed.sleeve_visible(), status != Status::Stopped);
        }
        assert!(feed.snapshot.is_none() && feed.artwork.is_none());
        feed.active = None;
        assert!(!feed.sleeve_visible());
        feed.active = Some("org.mpris.MediaPlayer2.fixture".into());
        feed.enabled = false;
        assert!(!feed.sleeve_visible());
        feed.enabled = true;
        feed.artwork_playback = None;
        feed.status = Status::Paused;
        assert!(feed.sleeve_visible());
        feed.status = Status::Stopped;
        assert!(!feed.sleeve_visible());
        feed.select(Some("org.mpris.MediaPlayer2.other".into()));
        assert!(!feed.sleeve_visible());
    }
    #[test]
    fn progress_advances_without_polling_and_stops_when_paused() {
        let now = Instant::now();
        let mut snapshot = PlaybackSnapshot::demo(now);
        assert_eq!(snapshot.progress_at(now + Duration::from_secs(90)), 0.5);
        snapshot.playing = false;
        snapshot.position = Duration::from_secs(45);
        assert_eq!(snapshot.progress_at(now + Duration::from_secs(90)), 0.25);
    }
    #[test]
    fn seeks_and_track_end_are_clamped() {
        let now = Instant::now();
        let mut snapshot = PlaybackSnapshot::demo(now);
        snapshot.position = Duration::from_secs(179);
        assert_eq!(snapshot.progress_at(now + Duration::from_secs(5)), 1.0);
        snapshot.duration = Duration::ZERO;
        assert_eq!(snapshot.progress_at(now), 0.0);
    }
    #[test]
    fn clock_handles_rate_reverse_pause_and_overflow_without_panics() {
        let now = Instant::now();
        let mut s = PlaybackSnapshot::demo(now);
        s.rate = 2.0;
        assert_eq!(
            s.position_at(now + Duration::from_secs(10)),
            Duration::from_secs(20)
        );
        s.position = Duration::from_secs(30);
        s.rate = -2.0;
        assert_eq!(s.position_at(now + Duration::from_secs(20)), Duration::ZERO);
        s.rate = f64::MAX;
        assert_eq!(s.progress_at(now + Duration::from_secs(20)), 1.0);
    }
    #[test]
    fn stale_and_replaced_requests_cannot_overwrite_new_tracks() {
        let mut g = RequestGate::default();
        let old = g.begin(1).unwrap();
        assert!(g.begin(2).is_none());
        g.invalidate();
        g.invalidate();
        assert!(!g.finish(old));
        let new = g.begin(3).unwrap();
        assert!(!g.finish(old));
        assert!(g.current(new));
        assert!(g.finish(new));
        let mut replacement = RequestGate::default();
        let newest = replacement.begin(4).unwrap();
        assert!(!replacement.finish(new));
        assert!(replacement.finish(newest));
    }
    #[test]
    fn unavailable_unknown_and_stale_timing_holds_the_arm() {
        let now = Instant::now();
        let mut feed = PlaybackFeed::default();
        feed.set_enabled(true);
        feed.status = Status::Playing;
        feed.snapshot = Some(PlaybackSnapshot::demo(now));
        assert!(feed.moving(now));
        assert!(feed.progress_at(now + Duration::from_secs(7)).is_none());
        feed.snapshot.as_mut().unwrap().duration = Duration::ZERO;
        assert!(feed.progress_at(now).is_none());
        feed.invalidate("Player disconnected");
        assert!(!feed.moving(now));
    }
    #[test]
    fn duration_changes_seeks_and_track_end_do_not_reverse() {
        let now = Instant::now();
        let mut feed = PlaybackFeed {
            enabled: true,
            status: Status::Playing,
            snapshot: Some(PlaybackSnapshot::demo(now)),
            ..Default::default()
        };
        let s = feed.snapshot.as_mut().unwrap();
        s.position = Duration::from_secs(90);
        s.duration = Duration::from_secs(120);
        assert_eq!(feed.progress_at(now), Some(0.75));
        feed.snapshot.as_mut().unwrap().duration = Duration::from_secs(60);
        assert_eq!(feed.progress_at(now), Some(1.0));
        assert!(!feed.moving(now));
        feed.snapshot.as_mut().unwrap().position = Duration::from_secs(15);
        assert_eq!(feed.progress_at(now), Some(0.25));
        feed.set_enabled(false);
        assert!(feed.progress_at(now).is_none());
    }
}
