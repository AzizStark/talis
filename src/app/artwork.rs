//! Cover and vinyl surface adjustments; authored center and perspective stay fixed.
use super::{scheduling::ensure_animation, state::AppState, style};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

pub(super) fn controls(
    area: &gtk::GLArea,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 20);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&style::section("Record label", &row));
    let size = gtk::SpinButton::with_range(5.0, 100.0, 0.5);
    size.set_widget_name("artwork-size");
    size.set_digits(1);
    size.set_value(state.borrow().config.artwork.size_percent);
    row.append(&style::row(
        "Label size (%)",
        "Size compared to the record",
        &size,
    ));
    {
        let state = state.clone();
        let area = area.downgrade();
        size.connect_value_changed(move |spin| {
            state.borrow_mut().config.artwork.size_percent = spin.value();
            if let Some(area) = area.upgrade() {
                super::presenter::queue_layout_render(&area);
            }
        });
    }
    style::separator(&row);
    let speed = gtk::SpinButton::with_range(0.0, crate::artwork_rotation::MAX_RPM, 1.0);
    speed.set_widget_name("artwork-speed");
    speed.set_digits(2);
    speed.set_tooltip_text(Some("Record spin speed in revolutions per minute."));
    speed.set_value(state.borrow().config.artwork.rotation_rpm);
    row.append(&style::row("Rotation (RPM)", "Record spin speed", &speed));
    {
        let state = state.clone();
        let area = area.downgrade();
        let running = running.clone();
        speed.connect_value_changed(move |spin| {
            {
                let mut state = state.borrow_mut();
                state.config.artwork.rotation_rpm = spin.value();
                state.sync_artwork(Instant::now());
            }
            if let Some(area) = area.upgrade() {
                super::presenter::queue_content_render(&area);
                ensure_animation(&area, state.clone(), running.clone());
            }
        });
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&style::section("Vinyl surface", &row));
    for (name, title, detail, max, is_opacity) in [
        (
            "vinyl-opacity",
            "Dust/stain opacity (%)",
            "Visibility of surface marks",
            100.0,
            true,
        ),
        (
            "vinyl-brightness",
            "Dust/stain brightness (%)",
            "Brightness of surface marks",
            200.0,
            false,
        ),
    ] {
        let spin = gtk::SpinButton::with_range(0.0, max, 5.0);
        spin.set_widget_name(name);
        spin.set_digits(0);
        let value = if is_opacity {
            state.borrow().config.artwork.vinyl_opacity
        } else {
            state.borrow().config.artwork.vinyl_brightness
        };
        spin.set_value(value * 100.0);
        spin.set_tooltip_text(Some(if is_opacity {
            "Adjust dust/stain and scratch visibility without changing the record or shadows."
        } else {
            "Brighten or dim dust/stain and scratches only. 100 is the original brightness."
        }));
        row.append(&style::row(title, detail, &spin));
        let state = state.clone();
        let area = area.downgrade();
        let running = running.clone();
        spin.connect_value_changed(move |spin| {
            {
                let mut s = state.borrow_mut();
                if is_opacity {
                    s.config.artwork.vinyl_opacity = spin.value() / 100.0;
                    s.sync_artwork(Instant::now());
                } else {
                    s.config.artwork.vinyl_brightness = spin.value() / 100.0;
                }
            }
            if let Some(area) = area.upgrade() {
                super::presenter::queue_content_render(&area);
                if is_opacity {
                    ensure_animation(&area, state.clone(), running.clone());
                }
            }
        });
        style::separator(&row);
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&style::section("Album sleeve", &row));
    let shadows = gtk::SpinButton::with_range(0.0, 100.0, 5.0);
    shadows.set_widget_name("sleeve-shadow-lift");
    shadows.set_digits(0);
    shadows.set_value(state.borrow().config.artwork.sleeve_shadow_lift * 100.0);
    shadows.set_tooltip_text(Some(
        "Lighten shadows on the sleeve only. Set to 0 for the original shading.",
    ));
    row.append(&style::row(
        "Shadow lift (%)",
        "Lighten shadows on the sleeve",
        &shadows,
    ));
    {
        let state = state.clone();
        let area = area.downgrade();
        shadows.connect_value_changed(move |spin| {
            state.borrow_mut().config.artwork.sleeve_shadow_lift = spin.value() / 100.0;
            if let Some(area) = area.upgrade() {
                super::presenter::queue_content_render(&area);
            }
        });
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
    fn size_and_speed_controls_preserve_fit_and_release_widgets() {
        gtk::init().unwrap();
        let state = Rc::new(RefCell::new(AppState {
            config: crate::config::Config::default(),
            previous_frame: std::time::Instant::now(),
            mode: crate::config::PreviewMode::Arm,
            benchmark: false,
            gpu_profile: false,
            shell_surface: false,
            arm: crate::arm::ArmState::default(),
            arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
            playback: Rc::new(RefCell::new(crate::playback::PlaybackFeed::default())),
        }));
        let area = gtk::GLArea::new();
        let widget = controls(&area, state.clone(), Rc::new(Cell::new(false)));
        assert!(style::find_widget(&widget.clone().upcast(), "copy-cover-settings").is_none());
        let weak = {
            let size = style::find_widget(&widget.clone().upcast(), "artwork-size")
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            assert_eq!(size.value(), 36.0);
            let fit = state.borrow().config.artwork.corners;
            size.set_value(30.0);
            assert_eq!(state.borrow().config.artwork.size_percent, 30.0);
            assert_eq!(state.borrow().config.artwork.corners, fit);
            assert!(style::find_widget(&widget.clone().upcast(), "vinyl-rotation").is_none());
            let opacity = style::find_widget(&widget.clone().upcast(), "vinyl-opacity")
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            let brightness = style::find_widget(&widget.clone().upcast(), "vinyl-brightness")
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            assert_eq!((opacity.value(), brightness.value()), (100.0, 100.0));
            opacity.set_value(40.0);
            brightness.set_value(150.0);
            assert_eq!(state.borrow().config.artwork.vinyl_opacity, 0.4);
            assert_eq!(state.borrow().config.artwork.vinyl_brightness, 1.5);
            assert_eq!(state.borrow().config.artwork.corners, fit);
            assert_eq!(state.borrow().config.artwork.sleeve_shadow_lift, 0.25);
            assert_eq!(state.borrow().artwork_rotation.angle(), 0.0);
            opacity.set_value(0.0);
            brightness.set_value(200.0);
            assert_eq!(state.borrow().config.artwork.vinyl_opacity, 0.0);
            assert_eq!(state.borrow().config.artwork.vinyl_brightness, 2.0);
            let shadows = style::find_widget(&widget.clone().upcast(), "sleeve-shadow-lift")
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            assert_eq!(shadows.value(), 25.0);
            shadows.set_value(40.0);
            assert_eq!(state.borrow().config.artwork.sleeve_shadow_lift, 0.4);
            assert_eq!(state.borrow().config.artwork.corners, fit);
            shadows.set_value(0.0);
            assert_eq!(state.borrow().config.artwork.sleeve_shadow_lift, 0.0);
            let speed = style::find_widget(&widget.clone().upcast(), "artwork-speed")
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            speed.set_value(45.0);
            assert_eq!(state.borrow().config.artwork.rotation_rpm, 45.0);
            speed.set_value(0.0);
            assert_eq!(state.borrow().config.artwork.rotation_rpm, 0.0);
            speed.set_value(78.0);
            assert_eq!(state.borrow().config.artwork.rotation_rpm, 78.0);
            assert_eq!(state.borrow().config.artwork.corners, fit);
            (
                size.downgrade(),
                speed.downgrade(),
                shadows.downgrade(),
                opacity.downgrade(),
                brightness.downgrade(),
            )
        };
        drop(widget);
        assert!(weak.0.upgrade().is_none());
        assert!(weak.1.upgrade().is_none());
        assert!(weak.2.upgrade().is_none());
        assert!(weak.3.upgrade().is_none());
        assert!(weak.4.upgrade().is_none());
    }
}
