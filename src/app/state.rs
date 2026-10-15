//! Mutable preview state shared by controls and frame callbacks.
use crate::{
    arm::{ArmState, motion::ArmMotion},
    config::{Config, PreviewMode},
    playback::PlaybackFeed,
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) struct AppState {
    pub(super) config: Config,
    pub(super) previous_frame: Instant,
    pub(super) mode: PreviewMode,
    pub(super) benchmark: bool,
    pub(super) gpu_profile: bool,
    /// External compositor-owned source: request paints with an on-demand timer.
    pub(super) shell_surface: bool,
    pub(super) arm: ArmState,
    pub(super) arm_motion: ArmMotion,
    pub(super) playback: Rc<RefCell<PlaybackFeed>>,
    pub(super) artwork_rotation: crate::artwork_rotation::ArtworkRotation,
    /// Center layer capability: fetched artwork and default ink can both rotate.
    pub(super) artwork_ready: bool,
    pub(super) vinyl_ready: bool,
}
impl AppState {
    pub(super) fn resume_at(&mut self, now: Instant) {
        self.previous_frame = now;
        self.artwork_rotation.resume_at(now);
        self.sync_playback(now);
        self.sync_artwork(now);
    }
    /// Only advancing playback may reuse a subpixel arm patch. The renderer
    /// additionally checks catch-up distance to keep seeks at full cadence.
    pub(super) fn arm_cache_target(&self, now: Instant) -> Option<f32> {
        let feed = self.playback.borrow();
        (self.mode == PreviewMode::Arm
            && self.config.render.animate_arm
            && !self.benchmark
            && feed.moving(now))
        .then(|| {
            feed.progress_at(now)
                .map(|progress| self.arm_target(progress))
        })
        .flatten()
    }
    pub(super) fn needs_redraw(&self) -> bool {
        self.needs_redraw_at(Instant::now())
    }
    fn needs_redraw_at(&self, now: Instant) -> bool {
        let feed = self.playback.borrow();
        self.benchmark
            || self.should_rotate_artwork(now)
            || (self.mode == PreviewMode::Arm
                && self.config.render.animate_arm
                && if feed.enabled {
                    feed.progress_at(now).is_some_and(|progress| {
                        self.arm_motion.needs_target(self.arm_target(progress))
                            || (self.arm.end > self.arm.start && feed.moving(now))
                    })
                } else {
                    self.arm.moving()
                })
    }
    fn should_rotate_artwork(&self, now: Instant) -> bool {
        self.mode == PreviewMode::Arm
            && ((self.artwork_ready
                && self.config.artwork.enabled
                && self.playback.borrow().surface_rotating(now))
                || (self.vinyl_ready
                    && self.config.artwork.vinyl_rotation
                    && self.config.artwork.vinyl_opacity > 0.0
                    && self.playback.borrow().surface_rotating(now)))
            && self.config.render.rotate_artwork
            && self.config.artwork.rotation_rpm > 0.0
    }
    pub(super) fn sync_artwork(&mut self, now: Instant) {
        let rotating = self.should_rotate_artwork(now);
        self.artwork_rotation
            .set_speed(now, self.config.artwork.rotation_rpm);
        self.artwork_rotation.set_rotating(now, rotating);
    }
    fn arm_target(&self, progress: f32) -> f32 {
        self.arm.start + (self.arm.end - self.arm.start) * progress
    }
    /// Timing callbacks change only the destination, never the visible pose.
    pub(super) fn sync_playback(&mut self, now: Instant) {
        let feed = self.playback.borrow();
        if self.mode == PreviewMode::Arm && self.config.render.animate_arm && feed.enabled {
            // `play(false)` clamps to the editable range, which would teleport
            // a still-settling pose when that range is contracted.
            self.arm.playing = false;
            if let Some(progress) = feed.progress_at(now) {
                if !self.arm_motion.has_target() {
                    self.arm_motion.hold(self.arm.position);
                }
                let target = self.arm_target(progress);
                let starting = !self.arm_motion.moving() && self.arm_motion.needs_target(target);
                self.arm_motion.retarget(target);
                if starting {
                    self.previous_frame = now;
                }
            } else {
                // No trustworthy destination: freeze the current displayed pose.
                self.arm_motion.hold(self.arm.position);
            }
        } else {
            self.arm_motion.hold(self.arm.position);
        }
    }
    /// Shared by real frame callbacks and tests, including forced/benchmark
    /// redraws. Disabling motion freezes the pose and discards old velocity.
    pub(super) fn advance_arm(&mut self, elapsed: Duration) {
        if self.mode != PreviewMode::Arm || !self.config.render.animate_arm {
            self.arm_motion.hold(self.arm.position);
        } else if self.playback.borrow().enabled {
            self.arm.scrub(self.arm_motion.advance(elapsed));
        } else {
            self.arm.advance(elapsed);
        }
    }
    pub(super) fn set_arm_range(&mut self, start: f32, end: f32, now: Instant) {
        let position = self.arm.position;
        self.arm.set_range(start, end);
        self.config.render.arm_range_start = self.arm.start;
        self.config.render.arm_range_end = self.arm.end;
        if self.playback.borrow().enabled || !self.config.render.animate_arm {
            self.arm.scrub(position);
            self.sync_playback(now);
        }
    }
}

#[cfg(test)]
mod tests;
