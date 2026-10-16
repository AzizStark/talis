use super::*;
use crate::{arm::ArmState, config::Config};
use std::time::Instant;

#[test]
fn hidden_control_requires_a_coverage_provider_and_an_interactive_launch() {
    let mut options = Options {
        independent: false,
        fullscreen: false,
        desktop: true,
        experimental_wayland: false,
        monitor: 0,
        smoke_seconds: None,
        benchmark: false,
        gpu_profile: false,
        scene_only: false,
        shell_surface: false,
        regional_presenter: false,
        native_patch: false,
        start_minimized: false,
        size: None,
        capture: None,
        arm_progress: 0.0,
        arm_demo: false,
        now_playing: true,
        player: None,
    };
    assert!(hidden_supported(&options, false), "native X11");
    options.experimental_wayland = true;
    assert!(
        !hidden_supported(&options, false),
        "unimplemented layer-shell coverage"
    );
    assert!(hidden_supported(&options, true), "Plasma adapter");
    options.desktop = false;
    assert!(!hidden_supported(&options, true), "ordinary preview");
    options.shell_surface = true;
    assert!(hidden_supported(&options, false), "owned GNOME bridge");
    options.benchmark = true;
    assert!(!hidden_supported(&options, true));
    options.benchmark = false;
    options.capture = Some("capture.png".into());
    assert!(!hidden_supported(&options, true));
}

#[test]
#[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
fn scene_and_controls_are_independent_windows() {
    gtk::init().expect("isolated GTK display");
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.WindowTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let mut config = Config::default();
    config.scene.mode = PreviewMode::Arm;
    let state = Rc::new(RefCell::new(AppState {
        config,
        previous_frame: Instant::now(),
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
    let mut options = Options {
        independent: true,
        experimental_wayland: false,
        fullscreen: false,
        desktop: true,
        monitor: 0,
        smoke_seconds: Some(30),
        benchmark: false,
        gpu_profile: false,
        scene_only: false,
        shell_surface: false,
        regional_presenter: false,
        native_patch: false,
        start_minimized: false,
        size: None,
        capture: None,
        arm_progress: 0.0,
        arm_demo: false,
        now_playing: false,
        player: None,
    };
    let build_window =
        |options: &Options| build(&app, options, state.clone(), Rc::new(Cell::new(false)));
    let preview = build_window(&options);
    let controls = preview.controls.as_ref().unwrap();
    assert!(!controls.has_css_class("talis-controls"));
    assert!(controls.child().unwrap().has_css_class("talis-controls"));
    assert_ne!(controls, &preview.window);
    assert!(controls.transient_for().is_none());
    assert_eq!(preview.window.child(), Some(preview.area.clone().upcast()));
    assert!(!preview.area.is_focusable());
    assert!(!preview.area.can_target());
    assert!(preview.arm_ui.is_some());
    fn check_user_controls(widget: &gtk::Widget, spins: &mut usize) {
        assert!(!widget.is::<gtk::Scale>(), "no manual scrubber");
        assert!(!widget.is::<gtk::CheckButton>(), "no testing checkboxes");
        if widget.is::<gtk::ComboBoxText>() || widget.is::<gtk::SpinButton>() {
            let controllers = widget.observe_controllers();
            for index in 0..controllers.n_items() {
                if let Some(scroll) = controllers
                    .item(index)
                    .and_then(|item| item.downcast::<gtk::EventControllerScroll>().ok())
                {
                    assert_eq!(scroll.propagation_phase(), gtk::PropagationPhase::None);
                }
            }
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            assert_ne!(label.text().as_str(), "Scene preview");
            assert!(!label.text().starts_with("No cover art"));
            assert_ne!(label.text().as_str(), "Cover art ready");
        }
        if widget.is::<gtk::Switch>() {
            assert!(matches!(
                widget.widget_name().as_str(),
                "run-on-startup"
                    | "minimize-to-tray"
                    | "pause-when-hidden"
                    | "experimental-mate-overlay"
            ));
        }
        if let Some(spin) = widget.downcast_ref::<gtk::SpinButton>() {
            *spins += 1;
            assert!(matches!(
                spin.widget_name().as_str(),
                "arm-range-start"
                    | "arm-range-end"
                    | "artwork-size"
                    | "artwork-speed"
                    | "sleeve-shadow-lift"
                    | "vinyl-opacity"
                    | "vinyl-brightness"
            ));
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            assert_ne!(button.label().as_deref(), Some("Reset to range start"));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            check_user_controls(&current, spins);
            child = current.next_sibling();
        }
    }
    let mut spins = 0;
    check_user_controls(&controls.clone().upcast(), &mut spins);
    let root = controls.clone().upcast::<gtk::Widget>();
    let overlay = preview.mate_overlay.as_ref().unwrap();
    assert!(!overlay.is_active());
    assert_eq!(
        overlay.is_sensitive(),
        crate::desktop::DesktopHost::mate_overlay_available()
    );
    if overlay.is_sensitive() {
        let host = crate::desktop::DesktopHost::new(0).unwrap();
        host.configure_scene(&preview.window).unwrap();
        bind_mate_overlay(&preview, &host, state.clone());
        preview.window.set_visible(true);
        for enabled in [true, false, true, false] {
            overlay.set_active(enabled);
            let deadline = Instant::now() + std::time::Duration::from_millis(100);
            while Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(
                state.borrow().config.render.experimental_mate_overlay,
                enabled
            );
            let xid = crate::desktop::test_window_id(&preview.window).unwrap();
            let result = std::process::Command::new("xprop")
                .args(["-id", &xid.to_string(), "_NET_WM_WINDOW_TYPE"])
                .output()
                .unwrap();
            let role = if enabled { "DOCK" } else { "DESKTOP" };
            assert!(String::from_utf8_lossy(&result.stdout).contains(role));
            assert!(preview.window.is_mapped());
            assert!(!preview.window.is_focusable());
        }
    }
    let hidden = style::find_widget(&root, "pause-when-hidden")
        .unwrap()
        .downcast::<gtk::Switch>()
        .unwrap();
    assert!(hidden.is_active() && state.borrow().config.render.pause_when_hidden);
    hidden.set_active(false);
    assert!(!state.borrow().config.render.pause_when_hidden);
    hidden.set_active(true);
    assert!(state.borrow().config.render.pause_when_hidden);
    assert!(style::find_widget(&root, "follow-playback").is_none());
    assert!(style::find_widget(&root, "copy-cover-settings").is_none());
    let stack = style::find_widget(&root, "settings-pages")
        .unwrap()
        .downcast::<gtk::Stack>()
        .unwrap();
    assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
    assert!(stack.child_by_name("playback").is_none());
    assert!(stack.child_by_name("tonearm").is_none());
    let appearance = stack.child_by_name("appearance").unwrap();
    for name in ["arm-range-start", "arm-range-end"] {
        assert!(style::find_widget(&appearance, name).is_some());
    }
    let navigation = style::find_widget(&root, "settings-navigation")
        .unwrap()
        .downcast::<gtk::ListBox>()
        .unwrap();
    assert_eq!(
        navigation.row_at_index(0).unwrap().widget_name().as_str(),
        "general"
    );
    assert_eq!(
        navigation.row_at_index(2).unwrap().widget_name().as_str(),
        "advanced"
    );
    assert_eq!(navigation.row_at_index(3).unwrap().widget_name(), "about");
    assert!(navigation.row_at_index(4).is_none());
    assert!(
        style::find_widget(
            &stack.child_by_name("general").unwrap(),
            "experimental-mate-overlay"
        )
        .is_none()
    );
    assert!(
        style::find_widget(
            &stack.child_by_name("advanced").unwrap(),
            "experimental-mate-overlay"
        )
        .is_some()
    );
    assert_eq!(
        spins, 7,
        "range, label size, rotation speed, dust and sleeve shadow lift"
    );
    if std::env::var_os("TALIS_CONTROLS_PLAYING").is_some() {
        let feed = state.borrow().playback.clone();
        let mut feed = feed.borrow_mut();
        feed.set_enabled(true);
        feed.active = Some("org.mpris.MediaPlayer2.spotify".into());
        feed.players = vec!["org.mpris.MediaPlayer2.spotify".into()];
        feed.status = crate::playback::Status::Playing;
        feed.snapshot = Some(crate::playback::PlaybackSnapshot::demo(Instant::now()));
        preview.media_ui.as_ref().unwrap().sync(&feed);
        let demo_cover = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([180 + x as u8, 75 + y as u8, 50, 255])
        });
        preview
            .media_ui
            .as_ref()
            .unwrap()
            .thumbnail
            .sync(1, Some(&demo_cover));
    }
    if std::env::var_os("TALIS_CONTROLS_COMPACT").is_some() {
        controls.set_default_size(780, 620);
    }
    controls.present();
    let context = glib::MainContext::default();
    if let Ok(name) = std::env::var("TALIS_CONTROLS_FOCUS") {
        let root = controls.clone().upcast::<gtk::Widget>();
        let index = if name.starts_with("arm-range")
            || name.starts_with("artwork")
            || name.starts_with("vinyl")
            || name.starts_with("sleeve")
        {
            1
        } else {
            0
        };
        navigation.select_row(navigation.row_at_index(index).as_ref());
        assert!(
            style::find_widget(&root, &name)
                .expect("focus target")
                .grab_focus()
        );
    }
    let deadline = Instant::now() + std::time::Duration::from_millis(500);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let root = controls.clone().upcast::<gtk::Widget>();
    let logo = style::find_widget(&root, "talis-logo")
        .unwrap()
        .downcast::<gtk::Picture>()
        .unwrap();
    assert_eq!(logo.alternative_text().as_deref(), Some("Talis"));
    assert!(logo.paintable().is_some());
    // GTK may round the requested size slightly to preserve image aspect.
    assert!((54..=60).contains(&logo.width()));
    assert!((39..=43).contains(&logo.height()));
    let stack = style::find_widget(&root, "settings-pages")
        .unwrap()
        .downcast::<gtk::Stack>()
        .unwrap();
    let navigation = style::find_widget(&root, "settings-navigation")
        .unwrap()
        .downcast::<gtk::ListBox>()
        .unwrap();
    let before = toml::to_string(&state.borrow().config).unwrap();
    let number_height = style::find_widget(&root, "quit-talis")
        .unwrap()
        .allocation()
        .height();
    assert!(number_height > 0);
    let quit = style::find_widget(&root, "quit-talis").unwrap();
    assert_eq!(quit.halign(), gtk::Align::Fill);
    assert!(quit.hexpands());
    let footer = quit.parent().unwrap();
    let sidebar = footer.parent().unwrap();
    assert!(!sidebar.hexpands());
    assert_eq!(sidebar.allocation().width(), 176);
    assert_eq!(
        quit.allocation().width(),
        footer.width(),
        "fill sidebar footer content width"
    );
    for page in ["general", "appearance", "advanced"] {
        let scroll = stack
            .child_by_name(page)
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        assert!(!scroll.is_kinetic_scrolling());
    }
    for (index, page, names) in [
        (
            1,
            "appearance",
            vec![
                "artwork-size",
                "sleeve-shadow-lift",
                "artwork-speed",
                "vinyl-opacity",
                "vinyl-brightness",
                "arm-range-start",
                "arm-range-end",
            ],
        ),
        (2, "advanced", vec![]),
        (3, "about", vec![]),
        (
            0,
            "general",
            vec![
                "frame-rate",
                "scene-variant",
                "scene-resolution",
                "playback-source",
            ],
        ),
    ] {
        navigation.select_row(navigation.row_at_index(index).as_ref());
        let deadline = Instant::now() + std::time::Duration::from_millis(250);
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(stack.visible_child_name().as_deref(), Some(page));
        if page == "about" {
            for (name, expected) in [
                ("about-version", env!("CARGO_PKG_VERSION")),
                ("about-credit", "Crafted with <3 by Aziz"),
            ] {
                let label = style::find_widget(&root, name)
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                assert_eq!(label.text(), expected);
                assert!(label.is_mapped());
            }
            for name in ["about-os", "about-desktop", "about-wm"] {
                let label = style::find_widget(&root, name)
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                assert!(!label.text().is_empty());
                assert!(label.is_mapped());
            }
            assert!(style::find_widget(&root, "about-author").is_none());
            let credit = style::find_widget(&root, "about-credit")
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
            assert_eq!(credit.xalign(), 0.5);
            let license = style::find_widget(&root, "about-license")
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap();
            let credits = style::find_widget(&root, "about-credits")
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap();
            for (section, title) in [(&license, "License"), (&credits, "Credits")] {
                let heading = section
                    .first_child()
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                assert_eq!(heading.text(), title);
                assert!(heading.has_css_class("section-title"));
                assert!(!heading.is_focusable(), "Section headings are not toggles");
            }
            let license_content = license.last_child().unwrap();
            let credits_content = credits.last_child().unwrap();
            assert!(license_content.has_css_class("settings-card"));
            assert!(credits_content.has_css_class("settings-card"));
            for name in ["about-code-license", "about-scene-license"] {
                assert!(style::find_widget(&license_content, name).is_some());
                assert!(style::find_widget(&credits_content, name).is_none());
            }
            for name in [
                "about-icon-credit",
                "about-artwork-credit",
                "about-photo-credit",
                "about-flower-credit",
            ] {
                assert!(style::find_widget(&credits_content, name).is_some());
                assert!(style::find_widget(&license_content, name).is_none());
            }
            assert_eq!(license.next_sibling().as_ref(), Some(credits.upcast_ref()));
            let until = Instant::now() + std::time::Duration::from_millis(100);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let credits_scroll = stack
                .child_by_name("about")
                .unwrap()
                .downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let adjustment = credits_scroll.vadjustment();
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            let until = Instant::now() + std::time::Duration::from_millis(100);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let artwork = style::find_widget(&root, "about-flower-credit").unwrap();
            let bounds = artwork.compute_bounds(controls).unwrap();
            assert!(
                bounds.y() >= 0.0 && bounds.y() + bounds.height() <= controls.height() as f32,
                "Credits must be reachable by scrolling"
            );
            let bounds = credit.compute_bounds(controls).unwrap();
            assert!(
                bounds.y() >= 0.0 && bounds.y() + bounds.height() <= controls.height() as f32,
                "The centered About footer must be reachable below the credits"
            );
            if let Some(path) = std::env::var_os("TALIS_ABOUT_CREDITS_CAPTURE") {
                let paintable = gtk::WidgetPaintable::new(Some(controls));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, controls.width() as f64, controls.height() as f64);
                let texture = controls
                    .renderer()
                    .unwrap()
                    .render_texture(snapshot.to_node().unwrap(), None);
                texture.save_to_png(std::path::PathBuf::from(path)).unwrap();
            }
            for (name, uri) in [
                (
                    "about-code-license",
                    "https://www.gnu.org/licenses/gpl-3.0.html",
                ),
                (
                    "about-scene-license",
                    "https://creativecommons.org/licenses/by/4.0/",
                ),
                (
                    "about-icon-credit",
                    "https://github.com/tabler/tabler-icons/blob/main/LICENSE",
                ),
                (
                    "about-artwork-credit",
                    "https://unsplash.com/photos/a-winding-path-leads-to-a-cottage-surrounded-by-trees-QSO-ewOX860",
                ),
                (
                    "about-photo-credit",
                    "https://unsplash.com/photos/close-up-of-a-vinyl-record-playing-on-a-turntable-XdRf3vsLh5Y",
                ),
                (
                    "about-flower-credit",
                    "https://www.cgtrader.com/free-3d-models/furniture/tableware/concrete-pot-plant-and-flower",
                ),
            ] {
                let link = style::find_widget(&root, name)
                    .unwrap()
                    .downcast::<gtk::LinkButton>()
                    .unwrap();
                assert_eq!(link.uri(), uri);
                assert!(link.is_mapped());
                let text = link.child().unwrap().downcast::<gtk::Box>().unwrap();
                let title = text
                    .first_child()
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                let detail = title
                    .next_sibling()
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                assert!(title.has_css_class("setting-title") && !title.text().is_empty());
                assert!(detail.has_css_class("setting-detail") && !detail.text().is_empty());
                #[allow(deprecated)]
                let (primary, secondary) = (
                    title.style_context().color(),
                    detail.style_context().color(),
                );
                assert!(
                    secondary.red() < primary.red()
                        && secondary.green() < primary.green()
                        && secondary.blue() < primary.blue(),
                    "{name} secondary details must be muted, not white"
                );
                let bounds = link.compute_bounds(controls).unwrap();
                assert!(
                    bounds.x() >= 0.0 && bounds.x() + bounds.width() <= controls.width() as f32
                );
                let activated = std::rc::Rc::new(std::cell::Cell::new(false));
                let flag = activated.clone();
                link.connect_activate_link(move |_| {
                    flag.set(true);
                    glib::Propagation::Stop
                });
                link.emit_by_name::<bool>("activate-link", &[]);
                assert!(activated.get());
            }
            adjustment.set_value(0.0);
            let until = Instant::now() + std::time::Duration::from_millis(100);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            for (name, expected) in [
                ("about-github", "https://github.com/AzizStark/talis"),
                ("about-email", "mailto:rrahman182@gmail.com"),
                ("about-support", "https://buymeacoffee.com/azizstark"),
            ] {
                let link = style::find_widget(&root, name)
                    .unwrap()
                    .downcast::<gtk::LinkButton>()
                    .unwrap();
                assert_eq!(link.uri(), expected);
                assert!(link.is_mapped() && link.is_focusable());
                let icon = link
                    .child()
                    .unwrap()
                    .first_child()
                    .unwrap()
                    .downcast::<gtk::Image>()
                    .unwrap();
                assert!(
                    icon.paintable().is_some(),
                    "Supplied About SVGs must decode without a fallback icon"
                );
                assert_eq!(icon.pixel_size(), 20);
                assert_eq!(
                    icon.paintable().unwrap().intrinsic_width(),
                    20 * icon.scale_factor()
                );
                let bounds = link.compute_bounds(controls).unwrap();
                assert!(
                    bounds.x() >= 0.0 && bounds.x() + bounds.width() <= controls.width() as f32,
                    "About link must fit horizontally"
                );
                assert!(
                    bounds.y() >= 0.0 && bounds.y() + bounds.height() <= controls.height() as f32,
                    "All About links should fit without scrolling at the default size"
                );
                // Exercise activation without opening a real browser/mail app.
                let invoked = std::rc::Rc::new(std::cell::Cell::new(false));
                let flag = invoked.clone();
                link.connect_activate_link(move |_| {
                    flag.set(true);
                    glib::Propagation::Stop
                });
                link.emit_by_name::<bool>("activate-link", &[]);
                assert!(invoked.get());
            }
        }
        if page == "general" {
            let reset = style::find_widget(&root, "restore-defaults").unwrap();
            assert!(reset.is_ancestor(&stack.child_by_name("general").unwrap()));
            assert!(!reset.is_ancestor(&stack.child_by_name("advanced").unwrap()));
            let heading = style::find_widget(&root, "general-heading").unwrap();
            for name in ["desktop-info", "now-playing-info"] {
                let information = style::find_widget(&root, name).unwrap();
                assert!(
                    information.is_ancestor(&heading),
                    "{name} belongs beside General"
                );
                let bounds = information.compute_bounds(controls).unwrap();
                assert!(bounds.x() + bounds.width() <= controls.width() as f32);
            }
            let cover = style::find_widget(&root, "header-cover-art").unwrap();
            if cover.is_visible() {
                assert!(cover.is_ancestor(&heading));
                assert_eq!((cover.width(), cover.height()), (36, 36));
                let text = style::find_widget(&root, "now-playing-info").unwrap();
                let information = text.parent().unwrap();
                assert!(
                    (information.height() - cover.height()).abs() <= 2,
                    "cover height must match both status lines"
                );
                let cover_bounds = cover.compute_bounds(controls).unwrap();
                let text_bounds = information.compute_bounds(controls).unwrap();
                assert!(cover_bounds.x() >= text_bounds.x() + text_bounds.width());
                assert!(cover_bounds.x() + cover_bounds.width() <= controls.width() as f32);
            }
        }
        let icon = style::find_widget(&root, &format!("{page}-icon"))
            .unwrap()
            .downcast::<gtk::Image>()
            .unwrap();
        assert!(icon.paintable().is_some());
        assert_eq!((icon.width(), icon.height()), (18, 18));
        for name in names.into_iter().chain(["quit-talis"]) {
            let widget = style::find_widget(&root, name).unwrap();
            assert_eq!(
                widget.allocation().height(),
                number_height,
                "{name} must match number-input height"
            );
            let bounds = widget.compute_bounds(controls).expect("allocated control");
            assert!(
                bounds.x() >= 0.0 && bounds.x() + bounds.width() <= controls.width() as f32,
                "{name} must fit horizontally"
            );
        }
        if page == "appearance" {
            let scroll = stack
                .child_by_name(page)
                .unwrap()
                .downcast::<gtk::ScrolledWindow>()
                .unwrap();
            let adjustment = scroll.vadjustment();
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            let deadline = Instant::now() + std::time::Duration::from_millis(150);
            while Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            for name in ["arm-range-start", "arm-range-end"] {
                let widget = style::find_widget(&root, name).unwrap();
                let bounds = widget.compute_bounds(controls).unwrap();
                assert!(
                    bounds.y() >= 0.0 && bounds.y() + bounds.height() <= controls.height() as f32,
                    "{name} must be reachable by scrolling Appearance"
                );
            }
        }
        if let Some(path) = std::env::var_os("TALIS_CONTROLS_CAPTURE") {
            let path = std::path::PathBuf::from(path);
            let path = if page == "appearance" {
                path
            } else {
                path.with_file_name(format!(
                    "{}-{page}.png",
                    path.file_stem().unwrap().to_string_lossy()
                ))
            };
            let paintable = gtk::WidgetPaintable::new(Some(controls));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, controls.width() as f64, controls.height() as f64);
            let node = snapshot.to_node().expect("settings render node");
            let renderer = controls.renderer().expect("native settings renderer");
            let texture = renderer.render_texture(&node, None);
            texture.save_to_png(path).expect("settings screenshot");
        }
    }
    assert_eq!(
        toml::to_string(&state.borrow().config).unwrap(),
        before,
        "navigation must not change scene or playback settings"
    );
    navigation.select_row(navigation.row_at_index(1).as_ref());
    assert_eq!(stack.visible_child_name().as_deref(), Some("appearance"));
    assert_eq!(app.windows().len(), 2);
    controls.destroy();
    preview.window.destroy();

    if !crate::desktop::DesktopHost::wayland_coverage_session_supported() {
        let mut unsupported_options = options.clone();
        unsupported_options.experimental_wayland = true;
        let saved = state.borrow().config.render.pause_when_hidden;
        let unsupported = build_window(&unsupported_options);
        let root = unsupported
            .controls
            .as_ref()
            .unwrap()
            .clone()
            .upcast::<gtk::Widget>();
        let hidden = style::find_widget(&root, "pause-when-hidden")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        assert!(!hidden.is_sensitive() && !hidden.is_active());
        assert_eq!(state.borrow().config.render.pause_when_hidden, saved);
        // Programmatic edits of an unsupported widget must not persist
        // an apparent setting change behind its disabled UI.
        hidden.set_active(!saved);
        assert_eq!(state.borrow().config.render.pause_when_hidden, saved);
        unsupported.controls.as_ref().unwrap().destroy();
        unsupported.window.destroy();
    }

    // Preview and GNOME-owned Settings have no standalone display identity.
    // Omit that row entirely, but retain the independent playback information.
    for shell_surface in [false, true] {
        let mut without_desktop = options.clone();
        without_desktop.desktop = false;
        without_desktop.shell_surface = shell_surface;
        let preview = build_window(&without_desktop);
        let root = preview
            .controls
            .as_ref()
            .unwrap()
            .clone()
            .upcast::<gtk::Widget>();
        assert!(style::find_widget(&root, "desktop-info").is_none());
        assert!(style::find_widget(&root, "now-playing-info").is_some());
        preview.controls.as_ref().unwrap().destroy();
        preview.window.destroy();
    }

    options.scene_only = true;
    let scene_only = build_window(&options);
    assert!(scene_only.controls.is_none());
    assert!(scene_only.arm_ui.is_none());
    assert_eq!(app.windows().len(), 1);
    scene_only.window.destroy();

    options.scene_only = false;
    state.borrow_mut().config.scene.mode = PreviewMode::Beauty;
    let static_preview = build_window(&options);
    let root = static_preview
        .controls
        .as_ref()
        .unwrap()
        .clone()
        .upcast::<gtk::Widget>();
    let stack = style::find_widget(&root, "settings-pages")
        .unwrap()
        .downcast::<gtk::Stack>()
        .unwrap();
    assert_eq!(stack.visible_child_name().as_deref(), Some("general"));
    assert!(stack.child_by_name("appearance").is_none());
    static_preview.controls.as_ref().unwrap().destroy();
    static_preview.window.destroy();
}
