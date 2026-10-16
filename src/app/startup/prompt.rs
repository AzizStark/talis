//! One-time consent, sharing the existing startup switch's guarded write path.
use super::{Manager, Options};
use gtk::{glib, prelude::*};
use std::cell::Cell;

pub(super) fn eligible(options: &Options) -> bool {
    options.desktop
        && !options.shell_surface
        && !options.scene_only
        && !options.start_minimized
        && !options.benchmark
        && !options.gpu_profile
        && !options.arm_demo
        && options.smoke_seconds.is_none()
        && options.capture.is_none()
}

pub(super) fn attach(body: &gtk::Box, toggle: &gtk::Switch, manager: Manager) {
    let offered = Cell::new(false);
    let toggle = toggle.downgrade();
    body.connect_map(move |body| {
        if offered.replace(true) {
            return;
        }
        let Some(parent) = body.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let parent = parent.downgrade();
        let toggle = toggle.clone();
        let manager = manager.clone();
        // Let Settings finish mapping before presenting a transient dialog.
        glib::idle_add_local_once(move || {
            let (Some(parent), Some(toggle)) = (parent.upgrade(), toggle.upgrade()) else {
                return;
            };
            if !parent.is_mapped() || !toggle.is_sensitive() || toggle.is_active() {
                return;
            }
            match manager.prompt_pending() {
                Ok(true) => dialog(&parent, &toggle, manager).present(),
                Ok(false) => (),
                Err(error) => tracing::warn!("Startup prompt unavailable: {error:#}"),
            }
        });
    });
}

fn dialog(parent: &gtk::Window, toggle: &gtk::Switch, manager: Manager) -> gtk::MessageDialog {
    // MessageDialog keeps the existing GTK 4.8 baseline; AlertDialog needs 4.10.
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .text("Do you want to start Talis when you sign in?")
        .secondary_text("You can change this later in settings.")
        .build();
    dialog.set_widget_name("startup-first-run-prompt");
    dialog.add_buttons(&[
        ("Not now", gtk::ResponseType::Cancel),
        ("Enable", gtk::ResponseType::Yes),
    ]);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let toggle = toggle.downgrade();
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Yes {
            let Some(toggle) = toggle.upgrade().filter(|toggle| toggle.is_sensitive()) else {
                dialog.destroy();
                return;
            };
            toggle.set_active(true);
            // Failed startup writes roll the switch back; allow a future retry.
            if !toggle.is_active() {
                dialog.destroy();
                return;
            }
        }
        if let Err(error) = manager.dismiss_prompt() {
            tracing::warn!("Could not remember startup prompt response: {error:#}");
        }
        dialog.destroy();
    });
    dialog
}

#[cfg(test)]
mod tests;
