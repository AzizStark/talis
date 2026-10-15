//! Capability-gated transport intent; no D-Bus, GTK or renderer dependencies.
use super::{MAX_SNAPSHOT_AGE, Status};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Previous,
    PlayPause,
    Next,
}
impl Command {
    pub const ALL: [Self; 3] = [Self::Previous, Self::PlayPause, Self::Next];
    pub fn label(self) -> &'static str {
        match self {
            Self::Previous => "Previous",
            Self::PlayPause => "Play / pause",
            Self::Next => "Next",
        }
    }
    /// Explicit Play/Pause are idempotent, unlike a repeated toggle. The
    /// intended action is captured from the state visible when clicked.
    pub fn method(self, status: Status) -> &'static str {
        match self {
            Self::Previous => "Previous",
            Self::Next => "Next",
            Self::PlayPause if status == Status::Playing => "Pause",
            Self::PlayPause => "Play",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub control: bool,
    pub play: bool,
    pub pause: bool,
    pub previous: bool,
    pub next: bool,
}
impl Capabilities {
    pub fn supports(self, method: &str) -> bool {
        self.control
            && match method {
                "Play" => self.play,
                "Pause" => self.pause,
                "Previous" => self.previous,
                "Next" => self.next,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub player: String,
    /// D-Bus unique owner, not an activatable/reassignable well-known name.
    pub owner: String,
    pub status: Status,
    pub capabilities: Capabilities,
    pub observed_at: Instant,
}
impl Target {
    pub fn supports(&self, command: Command, now: Instant) -> bool {
        self.supports_method(command.method(self.status), now)
    }
    pub fn supports_method(&self, method: &str, now: Instant) -> bool {
        now.saturating_duration_since(self.observed_at) <= MAX_SNAPSHOT_AGE
            && self.player.starts_with("org.mpris.MediaPlayer2.")
            && self.owner.starts_with(':')
            && self.capabilities.supports(method)
    }
    pub fn same_player(&self, other: &Self) -> bool {
        self.player == other.player && self.owner == other.owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn missing_capabilities_and_stale_targets_fail_closed() {
        let now = Instant::now();
        let mut t = Target {
            player: "org.mpris.MediaPlayer2.fixture".into(),
            owner: ":1.42".into(),
            status: Status::Playing,
            capabilities: Capabilities::default(),
            observed_at: now,
        };
        for c in Command::ALL {
            assert!(!t.supports(c, now));
        }
        t.capabilities = Capabilities {
            control: true,
            play: true,
            pause: false,
            previous: false,
            next: true,
        };
        assert!(!t.supports(Command::PlayPause, now));
        assert!(t.supports(Command::Next, now));
        t.status = Status::Paused;
        assert!(t.supports(Command::PlayPause, now));
        assert_eq!(Command::PlayPause.method(t.status), "Play");
        t.status = Status::Stopped;
        assert_eq!(Command::PlayPause.method(t.status), "Play");
        assert!(!t.supports(Command::Next, now + Duration::from_secs(7)));
        t.capabilities.control = false;
        assert!(!t.supports(Command::Next, now));
        t.capabilities.control = true;
        t.owner = t.player.clone();
        assert!(!t.supports(Command::Next, now));
        assert!(!t.capabilities.supports("OpenUri"));
    }
}
