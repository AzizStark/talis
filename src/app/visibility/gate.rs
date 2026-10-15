//! Animation scheduling and finite content refreshes share one GTK-thread gate.
use gtk::{glib, prelude::*};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Visible,
    AnimationCovered,
    Hidden,
}

#[derive(Default)]
pub(super) struct Gate {
    pub mode: Cell<Mode>,
    pub forced_hidden: Cell<bool>,
    pub generation: Cell<u64>,
    pub resume_needed: Cell<bool>,
    pub resume_notification: Cell<bool>,
    content: Cell<bool>,
    background: Cell<bool>,
}
fn key() -> glib::Quark {
    glib::Quark::from_str("talis-private-visibility-gate")
}
fn get(area: &gtk::GLArea) -> Option<Rc<Gate>> {
    // Only install() owns this key/type; all access is on GTK's thread.
    unsafe { area.qdata::<Rc<Gate>>(key()).map(|p| p.as_ref().clone()) }
}
pub(super) fn install(area: &gtk::GLArea, gate: Rc<Gate>) {
    unsafe {
        area.set_qdata(key(), gate);
    }
}
impl Gate {
    pub fn set_mode(&self, mode: Mode, forced: bool) -> Mode {
        self.forced_hidden.set(forced);
        let old = self.mode.replace(mode);
        if mode == Mode::Hidden || old != mode {
            self.content.set(false);
            self.background.set(false);
        }
        old
    }
    fn request_content(&self) -> bool {
        match self.mode.get() {
            Mode::Visible => true,
            Mode::AnimationCovered => {
                self.content.set(true);
                true
            }
            Mode::Hidden => false,
        }
    }
    fn begin_frame(&self) -> Option<(bool, bool)> {
        match self.mode.get() {
            Mode::Visible => Some((true, self.resume_needed.replace(false))),
            Mode::AnimationCovered if self.content.replace(false) => Some((false, false)),
            _ => None,
        }
    }
    fn invalidate_layout(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if !self.forced_hidden.get() {
            if self.mode.replace(Mode::Visible) != Mode::Visible {
                self.resume_needed.set(true);
                self.resume_notification.set(true);
            }
            self.content.set(false);
            self.background.set(false);
        }
    }
}
pub(in crate::app) fn suspended(area: &gtk::GLArea) -> bool {
    get(area).is_some_and(|g| g.mode.get() != Mode::Visible)
}
pub(in crate::app) fn generation(area: &gtk::GLArea) -> u64 {
    get(area).map_or(0, |g| g.generation.get())
}
pub(in crate::app) fn render_pending(area: &gtk::GLArea) -> bool {
    get(area).is_none_or(|g| {
        g.mode.get() == Mode::Visible || (g.mode.get() == Mode::AnimationCovered && g.content.get())
    })
}
/// A content refresh consumes its permit, so incidental GTK callbacks cannot
/// turn a song change into a new animation loop. Background has its own permit.
pub(in crate::app) fn begin_frame(area: &gtk::GLArea) -> Option<(bool, bool)> {
    let Some(g) = get(area) else {
        return Some((true, false));
    };
    g.begin_frame()
}
pub(in crate::app) fn request_content(area: &gtk::GLArea) -> bool {
    get(area).is_none_or(|g| g.request_content())
}
pub(in crate::app) fn request_background(area: &gtk::GLArea) -> bool {
    get(area).is_none_or(|g| match g.mode.get() {
        Mode::Visible => true,
        Mode::AnimationCovered => {
            g.background.set(true);
            true
        }
        Mode::Hidden => false,
    })
}
pub(in crate::app) fn begin_background(area: &gtk::GLArea) -> bool {
    get(area).is_none_or(|g| {
        g.mode.get() == Mode::Visible
            || (g.mode.get() == Mode::AnimationCovered && g.background.replace(false))
    })
}
/// Layout changes retire asynchronous evidence. Locks/power-off still defer
/// rendering; otherwise expose the new layout before assessing coverage again.
pub(in crate::app) fn invalidate_layout(area: &gtk::GLArea) {
    if let Some(g) = get(area) {
        g.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn covered_content_refresh_is_frozen_and_consumed_once() {
        let gate = Gate::default();
        gate.set_mode(Mode::AnimationCovered, false);
        assert_eq!(gate.begin_frame(), None);
        assert!(gate.request_content());
        assert!(gate.request_content());
        assert_eq!(gate.begin_frame(), Some((false, false)));
        assert_eq!(gate.begin_frame(), None);
        gate.set_mode(Mode::Visible, false);
        assert_eq!(gate.begin_frame(), Some((true, false)));
    }
    #[test]
    fn layout_invalidates_coverage_but_cannot_override_lock_or_power_off() {
        let gate = Gate::default();
        for mode in [Mode::AnimationCovered, Mode::Hidden] {
            gate.set_mode(mode, false);
            let generation = gate.generation.get();
            gate.invalidate_layout();
            assert_ne!(gate.generation.get(), generation);
            assert_eq!(gate.mode.get(), Mode::Visible);
            assert!(gate.resume_notification.replace(false));
            assert_eq!(gate.begin_frame(), Some((true, true)));
            assert_eq!(gate.begin_frame(), Some((true, false)));
        }
        gate.set_mode(Mode::Hidden, true);
        let generation = gate.generation.get();
        gate.invalidate_layout();
        assert_ne!(gate.generation.get(), generation);
        assert_eq!(gate.mode.get(), Mode::Hidden);
        assert_eq!(gate.begin_frame(), None);
        assert!(!gate.resume_notification.get());
    }
    #[test]
    fn fully_hidden_transitions_discard_pending_refresh_permissions() {
        let gate = Gate::default();
        gate.set_mode(Mode::AnimationCovered, false);
        assert!(gate.request_content());
        gate.background.set(true);
        gate.set_mode(Mode::Hidden, true);
        assert!(!gate.request_content());
        assert!(!gate.content.get() && !gate.background.get());
        gate.set_mode(Mode::AnimationCovered, false);
        assert!(!gate.content.get() && !gate.background.get());
        assert!(gate.request_content());
    }
}
