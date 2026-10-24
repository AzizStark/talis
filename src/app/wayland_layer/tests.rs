use super::*;

fn options() -> Options {
    Options {
        independent: true,
        fullscreen: false,
        desktop: true,
        experimental_wayland: true,
        monitor: 0,
        smoke_seconds: Some(3),
        benchmark: false,
        gpu_profile: false,
        scene_only: false,
        shell_surface: false,
        regional_presenter: false,
        native_patch: false,
        start_minimized: false,
        size: Some((320, 180)),
        capture: None,
        arm_progress: 0.0,
        arm_demo: false,
        now_playing: false,
        player: None,
    }
}

#[test]
fn layer_control_is_only_for_native_wayland_desktop_sessions() {
    let mut options = options();
    assert_eq!(available(&options), cfg!(feature = "wayland-desktop"));
    options.experimental_wayland = false;
    assert!(!available(&options), "X11");
    options.experimental_wayland = true;
    options.desktop = false;
    assert!(!available(&options), "preview");
    options.shell_surface = true;
    assert!(!available(&options), "GNOME companion");
    options.desktop = true;
    assert!(!available(&options), "even with conflicting shell flags");
    options.shell_surface = false;
    options.benchmark = true;
    assert!(!available(&options), "benchmark");
}

#[cfg(feature = "wayland-desktop")]
mod graphical {
    use super::*;
    use crate::{
        arm::ArmState,
        config::{Config, PreviewMode},
        preferences::{Session, Store},
    };
    use gtk::glib;
    use std::{
        cell::Cell,
        fs,
        time::{Duration, Instant},
    };

    fn state(config: Config) -> Rc<RefCell<AppState>> {
        Rc::new(RefCell::new(AppState {
            config,
            previous_frame: Instant::now(),
            mode: PreviewMode::Beauty,
            benchmark: false,
            gpu_profile: false,
            shell_surface: false,
            arm: ArmState::default(),
            arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
            playback: Default::default(),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
        }))
    }

    fn combo(root: &impl IsA<gtk::Widget>) -> gtk::ComboBoxText {
        style::find_widget(root.as_ref(), "desktop-layer")
            .unwrap()
            .downcast()
            .unwrap()
    }

    fn restore(window: &gtk::ApplicationWindow) {
        style::find_widget(&window.clone().upcast(), "restore-defaults")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .find(|window| window.widget_name() == "restore-defaults-confirmation")
            .unwrap()
            .downcast::<gtk::MessageDialog>()
            .unwrap();
        dialog.response(gtk::ResponseType::Accept);
    }

    fn capture(window: &gtk::ApplicationWindow, name: &str) {
        let Some(directory) = std::env::var_os("TALIS_LAYER_UI_CAPTURE_DIRECTORY") else {
            return;
        };
        let root = window.clone().upcast::<gtk::Widget>();
        let navigation = style::find_widget(&root, "settings-navigation")
            .unwrap()
            .downcast::<gtk::ListBox>()
            .unwrap();
        let row = style::find_widget(&root, "advanced")
            .unwrap()
            .downcast::<gtk::ListBoxRow>()
            .unwrap();
        navigation.select_row(Some(&row));
        window.present();
        let context = glib::MainContext::default();
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        window
            .renderer()
            .unwrap()
            .render_texture(snapshot.to_node().unwrap(), None)
            .save_to_png(std::path::PathBuf::from(directory).join(name))
            .unwrap();
    }

    #[test]
    #[ignore = "Requires private GTK display/bus and no TALIS_WAYLAND_LAYER; writes only temporary preferences"]
    fn saved_layer_autosaves_resets_and_preserves_environment_override() {
        gtk::init().unwrap();
        assert!(layer::environment_override().unwrap().is_none());
        let fixture = crate::autostart::Fixture::new();
        let store = Store::new(fixture.root.join("talis/settings.toml")).unwrap();
        let patch = |value: &str| {
            crate::preferences::Patch::from_iter([(
                "wayland_layer".into(),
                toml::Value::String(value.into()),
            )])
        };
        store.write(&patch("background")).unwrap();
        let before = fs::read(store.path()).unwrap();
        let config = crate::preferences::load(Config::default(), None, Some(store.clone()))
            .unwrap()
            .config;
        let state = state(config);
        let app = gtk::Application::builder()
            .application_id("io.github.azizstark.Talis.LayerPreferenceTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let preview =
            crate::app::window::build(&app, &options(), state.clone(), Rc::new(Cell::new(false)));
        let window = preview.controls.as_ref().unwrap();
        let choice = combo(window);
        assert_eq!(choice.active_id().as_deref(), Some("background"));
        assert!(choice.is_sensitive());
        let saver = crate::app::preferences::attach(
            &preview,
            &state.borrow(),
            Session {
                store: Some(store.clone()),
                issue: None,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            fs::read(store.path()).unwrap(),
            before,
            "opening Settings must not write"
        );

        // A debug snapshot keeps the launch layer separate from the pending choice.
        let debug_renderer = Rc::new(RefCell::new(None));
        let debug_running = Rc::new(Cell::new(false));
        crate::app::debug_info::attach(
            window,
            &preview.area,
            &debug_renderer,
            &state,
            &debug_running,
            &options(),
        );
        choice.set_active_id(Some("bottom"));
        assert_eq!(
            state.borrow().config.render.wayland_layer,
            WaylandLayer::Bottom
        );
        let until = Instant::now() + Duration::from_secs(3);
        while store.read().unwrap().wayland_layer != Some(WaylandLayer::Bottom) {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            assert!(Instant::now() < until, "autosave did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            crate::preferences::load(Config::default(), None, Some(store.clone()))
                .unwrap()
                .config
                .render
                .wayland_layer,
            WaylandLayer::Bottom
        );
        let detail = style::find_widget(&window.clone().upcast(), "desktop-layer-detail")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(detail.text().contains("Restart Talis"));
        style::find_widget(&window.clone().upcast(), "debug-info")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        let debug_window = gtk::Window::list_toplevels()
            .into_iter()
            .find(|window| window.widget_name() == "debug-info-window")
            .unwrap();
        let text = style::find_widget(&debug_window, "debug-info-report")
            .unwrap()
            .downcast::<gtk::TextView>()
            .unwrap()
            .buffer();
        let text = text.text(&text.start_iter(), &text.end_iter(), false);
        assert!(text.contains("Wayland layer at launch: background"));
        assert!(text.contains("Wayland layer preference: bottom"));
        debug_window.downcast::<gtk::Window>().unwrap().destroy();
        capture(window, "desktop-layer.png");
        restore(window);
        assert_eq!(choice.active_id().as_deref(), Some("automatic"));
        saver.stop();
        assert_eq!(
            store.read().unwrap().wayland_layer,
            Some(WaylandLayer::Automatic)
        );
        window.destroy();
        preview.window.destroy();

        store.write(&patch("bottom")).unwrap();
        state.borrow_mut().config.render.wayland_layer = WaylandLayer::Bottom;
        let preview =
            crate::app::window::build(&app, &options(), state.clone(), Rc::new(Cell::new(false)));
        let window = preview.controls.as_ref().unwrap();
        let old_row = combo(window)
            .parent()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap();
        let section = old_row.parent().unwrap().downcast::<gtk::Box>().unwrap();
        section.remove(&old_row);
        // Inject the typed override; never mutate a running GTK process's environment.
        section.append(&build(state.clone(), Ok(Some(WaylandLayer::Background))));
        let choice = combo(window);
        assert_eq!(choice.active_id().as_deref(), Some("background"));
        assert!(!choice.is_sensitive());
        capture(window, "desktop-layer-override.png");
        let before = fs::read(store.path()).unwrap();
        let saver = crate::app::preferences::attach(
            &preview,
            &state.borrow(),
            Session {
                store: Some(store.clone()),
                issue: None,
            },
        )
        .unwrap()
        .unwrap();
        choice.set_active_id(Some("bottom"));
        choice.set_active_id(Some("background"));
        assert_eq!(
            state.borrow().config.render.wayland_layer,
            WaylandLayer::Bottom
        );
        assert_eq!(fs::read(store.path()).unwrap(), before);
        restore(window);
        assert_eq!(
            choice.active_id().as_deref(),
            Some("background"),
            "override remains displayed after reset"
        );
        assert_eq!(
            state.borrow().config.render.wayland_layer,
            WaylandLayer::Automatic
        );
        saver.stop();
        assert_eq!(
            store.read().unwrap().wayland_layer,
            Some(WaylandLayer::Automatic)
        );
        window.destroy();
        preview.window.destroy();
    }
}
