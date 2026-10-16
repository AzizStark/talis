//! Explicit, read-only support snapshot. No logs, subprocesses, polling or uploads.
mod report;
use super::{Options, lifecycle::SharedRenderer, state::AppState, style};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

pub(super) fn controls() -> gtk::Box {
    let button = gtk::Button::with_label("Debug info");
    button.set_widget_name("debug-info");
    button.add_css_class("subtle-button");
    style::section(
        "Support",
        &style::row(
            "Debug information",
            "Review and copy details for a support report",
            &button,
        ),
    )
}

pub(super) fn attach(
    parent: &gtk::ApplicationWindow,
    area: &gtk::GLArea,
    renderer: &SharedRenderer,
    state: &Rc<RefCell<AppState>>,
    running: &Rc<Cell<bool>>,
    options: &Options,
) {
    let Some(button) =
        style::find_widget(&parent.clone().upcast(), "debug-info").and_downcast::<gtk::Button>()
    else {
        return;
    };
    let (parent, area) = (parent.downgrade(), area.downgrade());
    let (renderer, state, running) = (
        Rc::downgrade(renderer),
        Rc::downgrade(state),
        Rc::downgrade(running),
    );
    let options = options.clone();
    let dialog = Rc::new(RefCell::new(glib::WeakRef::<gtk::Window>::new()));
    button.connect_clicked(move |_| {
        if let Some(window) = dialog.borrow().upgrade() {
            window.present();
            return;
        }
        let (Some(parent), Some(area), Some(renderer), Some(state), Some(running)) = (
            parent.upgrade(),
            area.upgrade(),
            renderer.upgrade(),
            state.upgrade(),
            running.upgrade(),
        ) else {
            return;
        };
        let now = Instant::now();
        let mut report = report::Report::new();
        for (label, _, value) in super::about::system::details() {
            report.field(label, value);
        }
        report.field("Architecture", std::env::consts::ARCH);
        report.field(
            "Build profile",
            if cfg!(debug_assertions) {
                "Debug"
            } else {
                "Release"
            },
        );
        report.field(
            "GL backend override",
            match std::env::var("TALIS_GL_BACKEND").as_deref() {
                Ok("glx") => "GLX",
                Ok("auto") | Err(_) => "Automatic",
                _ => "Unrecognized",
            },
        );
        report.field(
            "GTK",
            format!(
                "{}.{}.{}",
                gtk::major_version(),
                gtk::minor_version(),
                gtk::micro_version()
            ),
        );
        report.field(
            "Session",
            format!("{:?}", crate::desktop::Session::detect()),
        );
        report.field(
            "Display backend",
            gtk::prelude::WidgetExt::display(&area).type_().name(),
        );
        if let Some(settings) = gtk::Settings::default() {
            report.field(
                "GTK theme",
                settings
                    .gtk_theme_name()
                    .as_deref()
                    .unwrap_or("Not reported"),
            );
        }
        report.field(
            "Wayland layer-shell build",
            cfg!(feature = "wayland-desktop"),
        );
        report.field(
            "Scene output (logical)",
            format!(
                "{}x{}; scale {}",
                area.width(),
                area.height(),
                area.scale_factor()
            ),
        );
        report.field(
            "Presentation",
            if options.shell_surface {
                "GNOME companion"
            } else if super::presenter::native_window(&area).is_some() {
                "Native X11 animated patch"
            } else if area.is::<super::presenter::RegionalArea>() {
                "Regional GTK surface"
            } else {
                "Full GTK surface"
            },
        );
        report.field(
            "Pause when hidden available",
            super::settings::hidden_supported(
                &options,
                crate::desktop::DesktopHost::wayland_coverage_session_supported(),
            ),
        );
        report.field("Rendering suspended", super::visibility::suspended(&area));
        report.field("Animation scheduled", running.get());
        report.field("Graphics error present", area.error().is_some());
        {
            let renderer = renderer.borrow();
            report.graphics(renderer.as_ref().map(|renderer| renderer.info()));
        }
        {
            let state = state.borrow();
            report.settings(&state.config);
            report.playback(&state.playback.borrow(), now);
        }
        let window = preview(&parent, report.finish());
        dialog.borrow().set(Some(&window));
        window.present();
    });
}

fn preview(parent: &impl IsA<gtk::Window>, text: String) -> gtk::Window {
    let window = gtk::Window::builder()
        .title("Talis · Debug info")
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(600)
        .default_height(480)
        .icon_name(style::APP_ICON_NAME)
        .build();
    window.set_widget_name("debug-info-window");
    style::bind_window_icon(&window);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("talis-controls");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();
    view.set_widget_name("debug-info-report");
    view.add_css_class("debug-report");
    view.buffer().set_text(&text);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .hexpand(true)
        .build();
    body.append(&scroll);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let close = gtk::Button::with_label("Close");
    close.set_widget_name("debug-info-close");
    close.add_css_class("subtle-button");
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    let copy = gtk::Button::with_label("Copy debug info");
    copy.set_widget_name("debug-info-copy");
    copy.add_css_class("subtle-button");
    copy.set_size_request(156, -1);
    copy.connect_clicked(move |button| {
        button.clipboard().set_text(&text);
        button.set_label("Copied");
    });
    actions.append(&close);
    actions.append(&copy);
    body.append(&actions);
    content.append(&body);
    window.set_child(Some(&content));
    window
}

#[cfg(test)]
mod tests;
