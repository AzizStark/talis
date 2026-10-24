//! Confirmed reset of user preferences through the existing live UI callbacks.
use super::{scenes::SceneUi, state::AppState, style};
use crate::{
    config::Config,
    preferences::{Document, Patch},
};
use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

type ResetCommit = Rc<RefCell<Option<Box<dyn Fn(Patch)>>>>;
pub(super) struct DefaultsUi {
    pub widget: gtk::Box,
    pub committed: ResetCommit,
}

pub(super) fn controls(
    window: &gtk::ApplicationWindow,
    state: Rc<RefCell<AppState>>,
    scene: Option<&SceneUi>,
) -> DefaultsUi {
    let button = gtk::Button::with_label("Restore defaults");
    button.set_widget_name("restore-defaults");
    button.add_css_class("subtle-button");
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    widget.append(&style::row(
        "Default settings",
        "Reset settings to their original values",
        &button,
    ));
    let committed: ResetCommit = Rc::new(RefCell::new(None));
    let weak_window = window.downgrade();
    let scene = scene.cloned();
    let commit = committed.clone();
    let dialog_ref = Rc::new(RefCell::new(glib::WeakRef::<gtk::MessageDialog>::new()));
    button.connect_clicked(move |_| {
        if let Some(dialog) = dialog_ref.borrow().upgrade() {
            dialog.present();
            return;
        }
        let Some(window) = weak_window.upgrade() else { return; };
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&window)
            .modal(true)
            .destroy_with_parent(true)
            .message_type(gtk::MessageType::Question)
            .text("Restore default settings?")
            .secondary_text("This resets appearance, player selection and desktop settings, and turns off Run on startup. Music playback and scene files are not changed.")
            .build();
        dialog.set_widget_name("restore-defaults-confirmation");
        dialog.content_area().add_css_class("talis-controls");
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Restore defaults", gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Cancel);
        dialog_ref.borrow().set(Some(&dialog));
        let (state, scene, commit, window) =
            (state.clone(), scene.clone(), commit.clone(), window.downgrade());
        let dialog_ref = dialog_ref.clone();
        dialog.connect_response(move |dialog, response| {
            dialog_ref.borrow().set(None::<&gtk::MessageDialog>);
            if response == gtk::ResponseType::Accept {
                if let Some(window) = window.upgrade() {
                    let root = window.upcast::<gtk::Widget>();
                    let patch = restore(&root, &state);
                    if let Some(commit) = commit.borrow().as_ref() { commit(patch); }
                    // Scene preferences are saved separately, only after the
                    // existing loader confirms a successful renderer commit.
                    if let Some(scene) = &scene { scene.restore_defaults(); }
                }
            }
            dialog.destroy();
        });
        dialog.present();
    });
    DefaultsUi { widget, committed }
}

fn restore(root: &gtk::Widget, state: &Rc<RefCell<AppState>>) -> Patch {
    let config = Config::default();
    let mut missing = Document::from_config(&config, None);
    // Package paths, launch/backend options and playback state are not reset.
    missing.scene = None;
    missing.scene_resolution = None;
    missing.preferred_player = None;
    macro_rules! spin {
        ($name:literal, $field:ident, $scale:expr) => {
            if let Some(spin) =
                style::find_widget(root, $name).and_then(|w| w.downcast::<gtk::SpinButton>().ok())
            {
                spin.set_value(missing.$field.unwrap() as f64 * $scale);
                missing.$field = None;
            }
        };
    }
    spin!("artwork-size", label_size_percent, 1.0);
    spin!("artwork-speed", rotation_rpm, 1.0);
    spin!("vinyl-opacity", surface_opacity, 100.0);
    spin!("vinyl-brightness", surface_brightness, 100.0);
    spin!("sleeve-shadow-lift", sleeve_shadow_lift, 100.0);
    // Expand the end before resetting the start; avoid range clamping.
    spin!("arm-range-end", arm_range_end, 100.0);
    spin!("arm-range-start", arm_range_start, 100.0);
    for (name, value) in [
        ("pause-when-hidden", config.render.pause_when_hidden),
        ("minimize-to-tray", config.minimize_to_tray),
        (
            "experimental-mate-overlay",
            config.render.experimental_mate_overlay,
        ),
        ("run-on-startup", false),
    ] {
        if let Some(toggle) =
            style::find_widget(root, name).and_then(|w| w.downcast::<gtk::Switch>().ok())
        {
            // Startup and host placement have ownership/rollback callbacks.
            if toggle.is_sensitive() || name == "minimize-to-tray" {
                toggle.set_active(value);
            }
            match name {
                "pause-when-hidden" if toggle.is_sensitive() => missing.pause_when_hidden = None,
                "minimize-to-tray" => missing.minimize_to_tray = None,
                "experimental-mate-overlay" if toggle.is_sensitive() => {
                    missing.experimental_mate_overlay = None
                }
                _ => (),
            }
        }
    }
    for (name, id) in [
        ("frame-rate", config.render.fps.to_string()),
        ("playback-source", "auto".into()),
        ("desktop-layer", config.render.wayland_layer.id().into()),
    ] {
        if let Some(combo) =
            style::find_widget(root, name).and_then(|w| w.downcast::<gtk::ComboBoxText>().ok())
        {
            if name == "desktop-layer" && !combo.is_sensitive() {
                // Clear the saved preference below, without changing the
                // displayed environment override or its active scene layer.
                continue;
            }
            combo.set_active_id(Some(&id));
            if name == "frame-rate" {
                missing.fps = None;
            } else if name == "desktop-layer" {
                missing.wayland_layer = None;
            }
        }
    }
    // Beauty mode has no artwork/range widgets; reset those saved preferences
    // too, without replacing the renderer or its configured scene package.
    missing.apply(&mut state.borrow_mut().config);
    if state.borrow().playback.borrow().preferred.is_some() {
        state.borrow().playback.borrow_mut().select(None);
    }
    let s = state.borrow();
    let mut snapshot = Document::from_config(&s.config, s.playback.borrow().preferred.as_deref());
    snapshot.scene = None;
    snapshot.scene_resolution = None;
    let mut patch = snapshot.values();
    patch.remove("schema_version");
    patch
}
