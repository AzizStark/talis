//! Settings assembly; window/scene ownership stays in window.rs.
use super::{
    Options,
    controls::{ArmUi, arm_controls},
    media::{self, MediaUi},
    scheduling::ensure_animation,
    state::AppState,
    style,
};
use crate::config::PreviewMode;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) fn hidden_supported(options: &Options, layer_coverage: bool) -> bool {
    !options.benchmark
        && options.capture.is_none()
        && (options.shell_surface
            || (options.desktop && (!options.experimental_wayland || layer_coverage)))
}

pub(super) struct Context<'a> {
    pub app: &'a gtk::Application,
    pub controls: Option<&'a gtk::ApplicationWindow>,
    pub area: &'a gtk::GLArea,
    pub stats: &'a gtk::Label,
    pub options: &'a Options,
}

pub(super) struct SettingsUi {
    pub arm_ui: Option<ArmUi>,
    pub media_ui: Option<MediaUi>,
    pub scene_ui: Option<super::scenes::SceneUi>,
    pub tray_ui: Option<Rc<super::tray::TrayUi>>,
    pub mate_overlay: Option<gtk::Switch>,
    pub defaults_ui: Option<super::defaults::DefaultsUi>,
    pub preferences_error: gtk::Label,
}

pub(super) fn build(
    context: Context<'_>,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) -> SettingsUi {
    let Context {
        app,
        controls,
        area,
        stats,
        options,
    } = context;
    let config = state.borrow().config.clone();
    let mode = config.scene.mode;
    let mut arm_ui = None;
    if let Some(controls) = controls {
        style::bind_window_icon(controls);
    }
    let mut media_ui = None;
    let mut scene_ui = None;
    let mut tray_ui = None;
    let mut mate_overlay = None;
    let mut defaults_ui = None;
    let preferences_error = style::label("", "quiet-status");
    preferences_error.set_widget_name("preferences-error");
    preferences_error.add_css_class("settings-feedback-source");
    preferences_error.set_visible(false);
    if let Some(controls) = controls {
        style::install(&gtk::prelude::WidgetExt::display(controls));
        let stack = gtk::Stack::new();
        stack.set_widget_name("settings-pages");
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        // Settings navigation must never start an animation in the scene.
        stack.set_transition_type(gtk::StackTransitionType::None);
        let appearance = style::page("Appearance");
        let general = style::page("General");
        general.append(&preferences_error);
        general.append(&super::graphics_notice::source());
        let advanced_page = style::page("Advanced");
        let about = super::about::page();
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        heading.set_widget_name("general-heading");
        let title = general.first_child().expect("General page title");
        general.remove(&title);
        title.set_hexpand(true);
        title.set_valign(gtk::Align::Center);
        heading.append(&title);
        let information = gtk::Box::new(gtk::Orientation::Vertical, 3);
        information.set_valign(gtk::Align::Center);
        if options.desktop {
            let context = style::label(
                &format!("Desktop · Display {}", options.monitor + 1),
                "quiet-status",
            );
            context.set_widget_name("desktop-info");
            context.set_xalign(1.0);
            context.set_wrap(false);
            information.append(&context);
        }
        heading.append(&information);
        general.prepend(&heading);
        let fps = gtk::ComboBoxText::new();
        fps.set_widget_name("frame-rate");
        for value in [15, 30, 60, 120] {
            fps.append(Some(&value.to_string()), &format!("{value} FPS"));
        }
        if ![15, 30, 60, 120].contains(&config.render.fps) {
            fps.append(
                Some(&config.render.fps.to_string()),
                &format!("{} FPS", config.render.fps),
            );
        }
        fps.set_active_id(Some(&config.render.fps.to_string()));
        if mode == PreviewMode::Arm {
            let ui = arm_controls(area, state.clone(), running.clone());
            let media = media::controls(area, state.clone(), running.clone());
            information.append(&media.label);
            heading.append(&media.thumbnail.widget);
            general.append(&style::section("Now playing", &media.widget));
            media_ui = Some(media);
            let cover = super::artwork::controls(area, state.clone(), running.clone());
            appearance.append(&cover);
            appearance.append(&style::section("Tonearm", &ui.widget));
            arm_ui = Some(ui);
        }
        let display = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scene = super::scenes::controls(&state.borrow(), options);
        if options.shell_surface {
            scene.choice.set_sensitive(false);
            scene
                .choice
                .set_tooltip_text(Some("This test bundle contains only the selected scene"));
        }
        display.append(&scene.widget);
        scene_ui = Some(scene);
        style::separator(&display);
        display.append(&style::row(
            "Frame rate",
            "Lower rates reduce GPU usage",
            &fps,
        ));
        style::separator(&display);
        let hidden = gtk::Switch::new();
        hidden.set_widget_name("pause-when-hidden");
        let hidden_available = hidden_supported(
            options,
            crate::desktop::DesktopHost::wayland_coverage_session_supported(),
        );
        // Keep the saved preference for a supported launch; an unavailable
        // control must not look active or rewrite that preference on startup.
        hidden.set_active(hidden_available && config.render.pause_when_hidden);
        hidden.set_sensitive(hidden_available);
        display.append(&style::row(
            "Pause when hidden",
            if hidden_available {
                "Save GPU work when the desktop is hidden"
            } else {
                "Not available in this session"
            },
            &hidden,
        ));
        let hidden_state = state.clone();
        hidden.connect_active_notify(move |switch| {
            if switch.is_sensitive() {
                hidden_state.borrow_mut().config.render.pause_when_hidden = switch.is_active();
            }
        });
        style::separator(&display);
        display.append(&super::startup::controls(options, state.clone()));
        if options.shell_surface {
            // GNOME's Settings launcher remains available on the desktop.
            // Closing this ordinary window must not terminate the scene source.
            controls.connect_close_request(|window| {
                window.set_visible(false);
                glib::Propagation::Stop
            });
        } else {
            let tray = super::tray::TrayUi::new(controls, state.clone());
            display.append(&tray.widget);
            tray_ui = Some(tray);
        }
        general.append(&style::section("Desktop", &display));
        let advanced = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let overlay = gtk::Switch::new();
        overlay.set_widget_name("experimental-mate-overlay");
        let available = options.desktop
            && !options.benchmark
            && crate::desktop::DesktopHost::mate_overlay_available();
        overlay.set_sensitive(available);
        overlay.set_active(available && config.render.experimental_mate_overlay);
        advanced.append(&style::row(
            "MATE compatibility",
            "Required on MATE to show Talis. Hides desktop icons.",
            &overlay,
        ));
        advanced_page.append(&style::section("Compatibility", &advanced));
        advanced_page.append(&super::debug_info::controls());
        mate_overlay = Some(overlay);
        let defaults = super::defaults::controls(controls, state.clone(), scene_ui.as_ref());
        general.append(&style::section("Settings", &defaults.widget));
        defaults_ui = Some(defaults);
        if options.benchmark {
            general.append(stats);
        }
        let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("settings-sidebar");
        sidebar.set_size_request(176, -1);
        // The full-width Quit button must not make the sidebar take extra space.
        sidebar.set_hexpand(false);
        let brand = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        brand.add_css_class("sidebar-brand");
        brand.append(&style::logo());
        sidebar.append(&brand);
        let navigation = gtk::ListBox::new();
        navigation.set_widget_name("settings-navigation");
        navigation.add_css_class("settings-navigation");
        navigation.set_selection_mode(gtk::SelectionMode::Single);
        navigation.set_activate_on_single_click(true);
        let mut initial = None;
        for (id, title, page) in [
            ("general", "General", &general),
            ("appearance", "Appearance", &appearance),
            ("advanced", "Advanced", &advanced_page),
            ("about", "About", &about),
        ] {
            if id == "appearance" && mode != PreviewMode::Arm {
                continue;
            }
            let scroll = gtk::ScrolledWindow::builder()
                .kinetic_scrolling(false)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .child(page)
                .build();
            stack.add_named(&scroll, Some(id));
            let entry = gtk::ListBoxRow::new();
            entry.set_widget_name(id);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.append(&style::navigation_icon(id));
            line.append(&style::label(title, "navigation-title"));
            entry.set_child(Some(&line));
            navigation.append(&entry);
            if id == "general" {
                initial = Some(entry);
            }
        }
        let weak_stack = stack.downgrade();
        navigation.connect_row_selected(move |_, row| {
            if let (Some(stack), Some(row)) = (weak_stack.upgrade(), row) {
                stack.set_visible_child_name(row.widget_name().as_str());
            }
        });
        navigation.select_row(initial.as_ref());
        sidebar.append(&navigation);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        sidebar.append(&spacer);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 12);
        footer.add_css_class("sidebar-footer");
        let quit = gtk::Button::with_label("Quit Talis");
        quit.set_widget_name("quit-talis");
        quit.add_css_class("subtle-button");
        quit.set_halign(gtk::Align::Fill);
        quit.set_hexpand(true);
        let weak = app.downgrade();
        quit.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.quit();
            }
        });
        footer.append(&quit);
        sidebar.append(&footer);
        shell.append(&sidebar);
        shell.append(&stack);
        let content = super::feedback::wrap(&shell);
        // Keep application colors and button states out of native decorations.
        content.add_css_class("talis-controls");
        controls.set_child(Some(&content));
        let fps_area = area.clone();
        fps.connect_changed(move |combo| {
            if let Some(value) = combo
                .active_id()
                .and_then(|value| value.parse::<u32>().ok())
            {
                state.borrow_mut().config.render.fps = value;
                ensure_animation(&fps_area, state.clone(), running.clone());
            }
        });
    }
    SettingsUi {
        arm_ui,
        media_ui,
        scene_ui,
        tray_ui,
        mate_overlay,
        defaults_ui,
        preferences_error,
    }
}
