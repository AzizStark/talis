//! User-facing arm range calibration, independent of the renderer lifecycle.
use super::{scheduling::ensure_animation, state::AppState, style};
use crate::arm::ArmState;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

#[derive(Clone)]
pub(super) struct ArmUi {
    pub(super) widget: gtk::Box,
    start: gtk::SpinButton,
    end: gtk::SpinButton,
    syncing: Rc<Cell<bool>>,
}
impl ArmUi {
    pub(super) fn range_reader(&self) -> impl Fn() -> Option<(f32, f32)> + 'static {
        let start = self.start.downgrade();
        let end = self.end.downgrade();
        let syncing = self.syncing.clone();
        move || {
            if syncing.get() {
                return None;
            }
            Some((
                start.upgrade()?.value() as f32 / 100.0,
                end.upgrade()?.value() as f32 / 100.0,
            ))
        }
    }
    pub(super) fn sync(&self, arm: &ArmState) {
        self.syncing.set(true);
        self.start.set_value(arm.start as f64 * 100.0);
        self.end.set_value(arm.end as f64 * 100.0);
        self.syncing.set(false);
    }
}
pub(super) fn arm_controls(
    area: &gtk::GLArea,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) -> ArmUi {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let start = gtk::SpinButton::with_range(0.0, 100.0, 0.5);
    start.set_widget_name("arm-range-start");
    start.set_digits(1);
    let end = gtk::SpinButton::with_range(0.0, 100.0, 0.5);
    end.set_widget_name("arm-range-end");
    end.set_digits(1);
    widget.append(&style::row(
        "Range start (%)",
        "Arm position when a track starts",
        &start,
    ));
    style::separator(&widget);
    widget.append(&style::row(
        "Range end (%)",
        "Arm position when a track ends",
        &end,
    ));
    let ui = ArmUi {
        widget,
        start,
        end,
        syncing: Rc::new(Cell::new(false)),
    };
    ui.sync(&state.borrow().arm);
    for (spin, is_start) in [(ui.start.clone(), true), (ui.end.clone(), false)] {
        let syncing = ui.syncing.clone();
        let start_widget = ui.start.downgrade();
        let end_widget = ui.end.downgrade();
        let state = state.clone();
        let area = area.downgrade();
        let running = running.clone();
        spin.connect_value_changed(move |spin| {
            if syncing.get() {
                return;
            }
            let mut s = state.borrow_mut();
            let (start, end) = (s.arm.start, s.arm.end);
            let now = Instant::now();
            if is_start {
                s.set_arm_range(spin.value() as f32 / 100.0, end, now);
            } else {
                s.set_arm_range(start, spin.value() as f32 / 100.0, now);
            }
            syncing.set(true);
            if let Some(start) = start_widget.upgrade() {
                start.set_value(s.arm.start as f64 * 100.0);
            }
            if let Some(end) = end_widget.upgrade() {
                end.set_value(s.arm.end as f64 * 100.0);
            }
            syncing.set(false);
            drop(s);
            if let Some(area) = area.upgrade() {
                super::presenter::queue_content_render(&area);
                ensure_animation(&area, state.clone(), running.clone());
            }
        });
    }
    ui
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PreviewMode};
    use std::time::Duration;
    #[test]
    #[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
    fn arm_range_controls_round_trip_without_feedback() {
        gtk::init().unwrap();
        let now = Instant::now();
        let state = Rc::new(RefCell::new(AppState {
            config: Config::default(),
            previous_frame: now,
            mode: PreviewMode::Arm,
            benchmark: false,
            gpu_profile: false,
            shell_surface: false,
            arm: ArmState::default(),
            arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
            playback: Rc::new(RefCell::new(crate::playback::PlaybackFeed::default())),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
        }));
        let area = gtk::GLArea::new();
        let ui = arm_controls(&area, state.clone(), Rc::new(Cell::new(false)));
        ui.start.set_value(20.0);
        ui.end.set_value(80.0);
        assert!((state.borrow().arm.start - 0.2).abs() < 0.00001);
        assert!((state.borrow().arm.end - 0.8).abs() < 0.00001);
        assert!(!state.borrow().arm.playing);
        ui.start.set_value(90.0);
        assert!((ui.end.value() - 90.0).abs() < 0.00001);
        assert!(!state.borrow().arm.moving());
        // Editable ranges must retarget music, not clamp the visible pose.
        {
            use crate::playback::{PlaybackSnapshot, Status};
            let mut s = state.borrow_mut();
            s.arm.scrub(0.7);
            let mut feed = s.playback.borrow_mut();
            feed.set_enabled(true);
            feed.status = Status::Paused;
            let mut clock = PlaybackSnapshot::demo(Instant::now());
            clock.playing = false;
            clock.position = Duration::from_secs(90);
            feed.snapshot = Some(clock);
        }
        ui.sync(&state.borrow().arm);
        ui.start.set_value(20.0);
        ui.end.set_value(30.0);
        assert_eq!(state.borrow().arm.position, 0.7);
        assert!(state.borrow().needs_redraw());
        state.borrow_mut().advance_arm(Duration::from_millis(33));
        assert!(state.borrow().arm.position < 0.7 && state.borrow().arm.position > 0.3);
        assert!(ui.start.is_sensitive() && ui.end.is_sensitive());
        let weak = (ui.start.downgrade(), ui.end.downgrade());
        drop(ui);
        assert!(weak.0.upgrade().is_none() && weak.1.upgrade().is_none());
    }
}
