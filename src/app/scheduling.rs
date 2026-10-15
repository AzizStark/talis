//! On-demand GTK frame-clock scheduling.
use super::state::AppState;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) fn ensure_animation(
    area: &gtk::GLArea,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) {
    if running.get()
        || super::visibility::suspended(area)
        || area.error().is_some()
        || !state.borrow().needs_redraw()
    {
        return;
    }
    running.set(true);
    if state.borrow().shell_surface {
        // Match the proven live probe: a minimized source may not initiate its
        // own frame-clock phase. Queue only necessary paints at the configured
        // cadence; end the timer when playback/motion settles. Normal desktop
        // backends retain their existing frame-clock path below.
        let interval = Duration::from_secs_f64(1.0 / state.borrow().config.render.fps as f64);
        let area = area.downgrade();
        glib::timeout_add_local(interval, move || {
            let Some(area) = area.upgrade() else {
                running.set(false);
                return glib::ControlFlow::Break;
            };
            if super::visibility::suspended(&area)
                || area.error().is_some()
                || !state.borrow().needs_redraw()
            {
                running.set(false);
                return glib::ControlFlow::Break;
            }
            super::presenter::queue_render(&area);
            glib::ControlFlow::Continue
        });
        return;
    }
    let next = Cell::new(Instant::now());
    area.add_tick_callback(move |area, _| {
        if super::visibility::suspended(area)
            || area.error().is_some()
            || !state.borrow().needs_redraw()
        {
            running.set(false);
            return glib::ControlFlow::Break;
        }
        let now = Instant::now();
        let interval = Duration::from_secs_f64(1.0 / state.borrow().config.render.fps as f64);
        if now >= next.get() {
            super::presenter::queue_render(area);
            let deadline = next.get() + interval;
            next.set(if deadline < now {
                now + interval
            } else {
                deadline
            });
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires an isolated GTK display"]
    fn external_timer_is_on_demand_and_releases_state_after_stop_or_area_loss() {
        gtk::init().unwrap();
        let state = Rc::new(RefCell::new(AppState {
            config: crate::config::Config::default(),
            previous_frame: Instant::now(),
            mode: crate::config::PreviewMode::Beauty,
            benchmark: false,
            gpu_profile: false,
            shell_surface: true,
            arm: Default::default(),
            arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
            playback: Rc::new(RefCell::new(crate::playback::PlaybackFeed::default())),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
        }));
        let area = gtk::GLArea::new();
        let running = Rc::new(Cell::new(false));
        ensure_animation(&area, state.clone(), running.clone());
        assert!(!running.get(), "idle source must not have a polling timer");
        state.borrow_mut().benchmark = true;
        ensure_animation(&area, state.clone(), running.clone());
        assert!(running.get());
        let references = Rc::strong_count(&state);
        ensure_animation(&area, state.clone(), running.clone());
        assert_eq!(
            Rc::strong_count(&state),
            references,
            "one timer per active source"
        );
        state.borrow_mut().benchmark = false;
        wait_idle(&running);
        assert_eq!(Rc::strong_count(&state), 1);
        state.borrow_mut().benchmark = true;
        ensure_animation(&area, state.clone(), running.clone());
        assert!(running.get(), "activity can restart after settling");
        drop(area);
        wait_idle(&running);
        assert_eq!(Rc::strong_count(&state), 1);
    }

    fn wait_idle(running: &Cell<bool>) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while running.get() {
            assert!(
                Instant::now() < deadline,
                "external timer did not terminate"
            );
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
