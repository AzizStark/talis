//! Normal desktop defaults live here, not in individual launchers.
use super::Options;
use crate::config::PreviewMode;

impl Options {
    pub(super) fn normal_desktop(&self) -> bool {
        self.desktop
            && !self.independent
            && !self.shell_surface
            && !self.scene_only
            && !self.benchmark
            && !self.gpu_profile
            && !self.arm_demo
            && self.capture.is_none()
            && self.smoke_seconds.is_none()
    }

    pub(super) fn apply_desktop_defaults(&mut self, mode: PreviewMode) {
        if self.normal_desktop()
            && mode == PreviewMode::Arm
            && !self.experimental_wayland
            && !self.regional_presenter
        {
            self.native_patch = true;
        }
    }
}

#[cfg(test)]
mod tests;
