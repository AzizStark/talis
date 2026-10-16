//! Strict field allowlist: never serialize Config, playback metadata or errors.
use crate::{
    config::Config,
    playback::{PlaybackFeed, transport::Command},
    rendering::RendererInfo,
};
use std::{fmt::Display, time::Instant};

pub(super) struct Report(String);
impl Report {
    pub fn new() -> Self {
        Self("Talis debug info (format 1)\n\n".into())
    }
    pub fn field(&mut self, label: &str, value: impl Display) {
        self.0.push_str(label);
        self.0.push_str(": ");
        self.0.push_str(&safe_value(&value.to_string()));
        self.0.push('\n');
    }
    pub fn graphics(&mut self, info: Option<&RendererInfo>) {
        self.field("Renderer initialized", info.is_some());
        if let Some(info) = info {
            self.field("OpenGL", &info.version);
            self.field("GLSL", &info.shading_language);
            self.field("GPU vendor", &info.vendor);
            self.field("GPU renderer", &info.renderer);
            self.field(
                "Loaded scene pixels",
                format!("{}x{}", info.scene_size.0, info.scene_size.1),
            );
        }
    }
    pub fn settings(&mut self, config: &Config) {
        self.field("Scene", config.scene.variant.id());
        self.field("Resolution preference", config.scene.resolution.id());
        self.field("Scene mode", format!("{:?}", config.scene.mode));
        self.field("Frame rate limit", config.render.fps);
        self.field(
            "Pause when hidden preference",
            config.render.pause_when_hidden,
        );
        self.field("Minimize to tray preference", config.minimize_to_tray);
        self.field(
            "MATE compatibility preference",
            config.render.experimental_mate_overlay,
        );
        self.field("Animate arm", config.render.animate_arm);
        self.field("Scene controls enabled", config.transport.enabled);
        self.field("Arm interpolation", config.render.arm_interpolation);
        self.field(
            "Arm range",
            format!(
                "{}–{}%",
                config.render.arm_range_start * 100.0,
                config.render.arm_range_end * 100.0
            ),
        );
        self.field("Label size", config.artwork.size_percent);
        self.field("Record RPM", config.artwork.rotation_rpm);
        self.field("Dust/stain opacity", config.artwork.vinyl_opacity);
        self.field("Dust/stain brightness", config.artwork.vinyl_brightness);
    }
    pub fn playback(&mut self, feed: &PlaybackFeed, now: Instant) {
        self.field("MPRIS enabled", feed.enabled);
        self.field(
            "Player selection",
            if feed.preferred.is_some() {
                "Manual"
            } else {
                "Automatic"
            },
        );
        self.field("Discovered players", feed.players.len());
        // Keep only the application component; discard instance/unique-owner IDs.
        let player = feed
            .active
            .as_deref()
            .and_then(|name| name.strip_prefix("org.mpris.MediaPlayer2."))
            .and_then(|name| name.split('.').next())
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 64
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            });
        self.field(
            "Active player application",
            player.unwrap_or("Not reported"),
        );
        self.field(
            "Playback status (last observed)",
            format!("{:?}", feed.status),
        );
        self.field("Fresh track timing", feed.progress_at(now).is_some());
        self.field("Artwork supplied", feed.artwork.is_some());
        self.field("Control request pending", feed.control_pending);
        for command in Command::ALL {
            self.field(
                &format!("{} available", command.label()),
                feed.command_target(command, now).is_some(),
            );
        }
        self.field(
            "Volume control available",
            feed.volume_target(now).is_some(),
        );
    }
    pub fn finish(self) -> String {
        self.0
    }
}

fn safe_value(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| {
            if (word.contains('/') && !matches!(word, "GNU/Linux" | "Mesa/X.org"))
                || word.starts_with('~')
                || word.contains('@')
                || word.contains('\\')
            {
                "[redacted]".to_owned()
            } else {
                word.chars()
                    .filter(|c| {
                        !c.is_control()
                            && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                    })
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests;
