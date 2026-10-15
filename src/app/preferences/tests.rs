use super::*;
use crate::{
    app::Options,
    arm::ArmState,
    config::{Config, PreviewMode},
    playback::PlaybackFeed,
    preferences::Store,
};
use std::{fs, time::Instant};
fn wait(check: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_secs(3);
    while !check() {
        while context.pending() {
            context.iteration(false);
        }
        assert!(Instant::now() < until, "autosave did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "Requires private GTK display/bus; writes only temporary preferences"]
fn ui_autosave_tracks_edits_not_cli_values_polling_or_uncommitted_scenes() {
    gtk::init().unwrap();
    let fixture = crate::autostart::Fixture::new();
    let store = Store::new(fixture.root.join("talis/settings.toml")).unwrap();
    store.write(&one("fps", 60_i64)).unwrap();
    store
        .write(&one("preferred_player", "org.mpris.MediaPlayer2.spotify"))
        .unwrap();
    let mut config = Config::default();
    config.scene.mode = PreviewMode::Arm;
    config.render.fps = 30; // A temporary launch override, not a user edit.
    let feed = Rc::new(RefCell::new(PlaybackFeed {
        preferred: Some("org.mpris.MediaPlayer2.firefox".into()),
        ..PlaybackFeed::default()
    }));
    feed.borrow_mut().set_enabled(true);
    let state = Rc::new(RefCell::new(AppState {
        config,
        previous_frame: Instant::now(),
        mode: PreviewMode::Arm,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm: ArmState::default(),
        arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
        playback: feed.clone(),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    }));
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.PreferencesTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let options = Options {
        independent: true,
        fullscreen: false,
        desktop: false,
        experimental_wayland: false,
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
        now_playing: true,
        player: None,
    };
    let preview =
        super::super::window::build(&app, &options, state.clone(), Rc::new(Cell::new(false)));
    let saver = attach(
        &preview,
        &state.borrow(),
        Session {
            store: Some(store.clone()),
            issue: None,
        },
    )
    .unwrap()
    .unwrap();
    let root = preview
        .controls
        .as_ref()
        .unwrap()
        .clone()
        .upcast::<gtk::Widget>();
    let spin = |name| {
        style::find_widget(&root, name)
            .unwrap()
            .downcast::<gtk::SpinButton>()
            .unwrap()
    };
    // Mirror the real MPRIS callback's borrowed-state/programmatic synchronization.
    {
        let s = state.borrow_mut();
        preview
            .media_ui
            .as_ref()
            .unwrap()
            .sync(&s.playback.borrow());
        preview.arm_ui.as_ref().unwrap().sync(&s.arm);
    }
    assert!(saver.dirty.borrow().is_empty());
    spin("vinyl-opacity").set_value(40.0);
    spin("artwork-size").set_value(38.0);
    spin("arm-range-start").set_value(90.0);
    preview
        .scene_ui
        .as_ref()
        .unwrap()
        .choice
        .set_active_id(Some("night"));
    preview
        .scene_ui
        .as_ref()
        .unwrap()
        .resolution
        .set_active_id(Some("4k"));
    wait(|| store.read().unwrap().surface_opacity == Some(0.4));
    let saved = store.read().unwrap();
    assert_eq!(saved.fps, Some(60));
    assert_eq!(
        saved.preferred_player.as_deref(),
        Some("org.mpris.MediaPlayer2.spotify")
    );
    assert_eq!(
        saved.scene, None,
        "scene request must not save before load succeeds"
    );
    assert_eq!(
        saved.scene_resolution, None,
        "resolution requests must not save before load succeeds"
    );
    assert_eq!(saved.label_size_percent, Some(38.0));
    assert!((saved.arm_range_start.unwrap() - 0.9).abs() < 0.00001);
    let player = style::find_widget(&root, "playback-source")
        .unwrap()
        .downcast::<gtk::ComboBoxText>()
        .unwrap();
    player.set_active_id(Some("auto"));
    spin("artwork-speed").set_value(45.0);
    // Quit before debounce: pending edits must still be durable.
    saver.stop();
    assert_eq!(store.read().unwrap().rotation_rpm, Some(45.0));
    assert_eq!(
        store.read().unwrap().preferred_player.as_deref(),
        Some("automatic")
    );
    assert!(saver.debounce.borrow().is_none() && saver.completion.borrow().is_none());
    let restored = crate::preferences::load(Config::default(), None, Some(store.clone())).unwrap();
    assert_eq!(restored.config.artwork.size_percent, 38.0);
    assert_eq!(restored.config.artwork.rotation_rpm, 45.0);
    assert_eq!(restored.config.render.fps, 60);
    drop(player);
    drop(root);
    drop(saver);
    preview.controls.unwrap().destroy();
    preview.window.destroy();
    {
        let mut s = state.borrow_mut();
        s.config = restored.config;
        s.config.scene.mode = PreviewMode::Arm; // The normal launch's --arm flag.
        let (start, end) = (
            s.config.render.arm_range_start,
            s.config.render.arm_range_end,
        );
        s.arm.set_range(start, end);
    }
    let reopened =
        super::super::window::build(&app, &options, state.clone(), Rc::new(Cell::new(false)));
    let root = reopened
        .controls
        .as_ref()
        .unwrap()
        .clone()
        .upcast::<gtk::Widget>();
    assert_eq!(
        style::find_widget(&root, "artwork-size")
            .unwrap()
            .downcast::<gtk::SpinButton>()
            .unwrap()
            .value(),
        38.0
    );
    assert_eq!(
        style::find_widget(&root, "artwork-speed")
            .unwrap()
            .downcast::<gtk::SpinButton>()
            .unwrap()
            .value(),
        45.0
    );
    assert_eq!(
        style::find_widget(&root, "frame-rate")
            .unwrap()
            .downcast::<gtk::ComboBoxText>()
            .unwrap()
            .active_id()
            .as_deref(),
        Some("60")
    );
    let reset_saver = attach(
        &reopened,
        &state.borrow(),
        Session {
            store: Some(store.clone()),
            issue: None,
        },
    )
    .unwrap()
    .unwrap();
    // The runtime is already 60 FPS, but disk contains a different override.
    store.write(&one("fps", 120_i64)).unwrap();
    let reset = style::find_widget(&root, "restore-defaults")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let confirmation = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .find(|window| window.widget_name() == "restore-defaults-confirmation")
            .unwrap()
            .downcast::<gtk::MessageDialog>()
            .unwrap()
    };
    reset.emit_clicked();
    let dialog = confirmation();
    reset.emit_clicked();
    assert_eq!(confirmation(), dialog, "only one confirmation dialog");
    dialog.response(gtk::ResponseType::Cancel);
    assert_eq!(state.borrow().config.artwork.size_percent, 38.0);
    assert_eq!(store.read().unwrap().fps, Some(120));
    let before_scene = state.borrow().config.scene.clone();
    let spin = |name| {
        style::find_widget(&root, name)
            .unwrap()
            .downcast::<gtk::SpinButton>()
            .unwrap()
    };
    spin("arm-range-end").set_value(95.0);
    spin("arm-range-start").set_value(90.0);
    spin("artwork-speed").set_value(52.0); // A pending pre-reset autosave.
    reset.emit_clicked();
    confirmation().response(gtk::ResponseType::Accept);
    wait(|| store.read().unwrap().rotation_rpm == Some(33.33));
    let defaults = Config::default();
    let mut expected = Document::from_config(&defaults, None).values();
    expected.remove("scene");
    expected.remove("scene_resolution");
    let mut saved = store.read().unwrap().values();
    saved.remove("scene");
    saved.remove("scene_resolution");
    assert_eq!(saved, expected);
    assert_eq!(
        state.borrow().config.scene,
        before_scene,
        "no loader, no commit"
    );
    assert_eq!(state.borrow().config.scene.mode, PreviewMode::Arm);
    assert!(state.borrow().playback.borrow().enabled);
    assert_eq!(
        (state.borrow().arm.start, state.borrow().arm.end),
        (0.0, 1.0)
    );
    assert_eq!(spin("artwork-size").value(), 36.0);
    assert_eq!(spin("vinyl-brightness").value(), 100.0);
    let hidden = style::find_widget(&root, "pause-when-hidden")
        .unwrap()
        .downcast::<gtk::Switch>()
        .unwrap();
    assert!(
        !hidden.is_sensitive() && !hidden.is_active(),
        "preview has no coverage provider"
    );
    assert!(
        state.borrow().config.render.pause_when_hidden,
        "reset keeps the default for supported sessions"
    );
    // Edits made immediately after reset must supersede its in-flight write.
    spin("artwork-speed").set_value(40.0);
    reset_saver.stop();
    assert_eq!(store.read().unwrap().rotation_rpm, Some(40.0));
    assert!(reset_saver.debounce.borrow().is_none() && reset_saver.completion.borrow().is_none());
    // Session-only launches still reset at runtime without a preferences writer.
    reopened
        .defaults_ui
        .as_ref()
        .unwrap()
        .committed
        .borrow_mut()
        .take();
    reset.emit_clicked();
    confirmation().response(gtk::ResponseType::Accept);
    assert_eq!(state.borrow().config.artwork.rotation_rpm, 33.33);
    assert_eq!(store.read().unwrap().rotation_rpm, Some(40.0));
    reopened.controls.unwrap().destroy();
    reopened.window.destroy();
}

#[test]
#[ignore = "Requires private GTK display; writes only temporary preferences"]
fn autosave_debounces_retains_failed_edits_and_has_no_idle_poll() {
    gtk::init().unwrap();
    let fixture = crate::autostart::Fixture::new();
    let store = Store::new(fixture.root.join("talis/settings.toml")).unwrap();
    let error = gtk::Label::new(None);
    error.set_visible(false);
    let saver = Autosave::new(
        store.clone(),
        Document::from_config(&Config::default(), None),
        &error,
    )
    .unwrap();
    saver.update(one("rotation_rpm", 44.0_f64));
    saver.update(one("rotation_rpm", 45.0_f64));
    assert!(!store.path().exists());
    wait(|| store.read().unwrap().rotation_rpm == Some(45.0));
    wait(|| saver.completion.borrow().is_none());
    assert!(saver.requests.borrow().is_empty() && saver.dirty.borrow().is_empty());
    fs::write(store.path(), "schema_version=99").unwrap();
    saver.restore(one("surface_opacity", 0.5_f64));
    wait(|| error.is_visible());
    assert_eq!(
        fs::read_to_string(store.path()).unwrap(),
        "schema_version=99"
    );
    assert!(saver.dirty.borrow().contains_key("surface_opacity"));
    // User repairs the file. A new edit retries previously unsaved fields too.
    fs::write(store.path(), "schema_version=1\nrotation_rpm=45\n").unwrap();
    saver.update(one("surface_brightness", 1.2_f64));
    wait(|| store.read().unwrap().surface_brightness == Some(1.2));
    assert_eq!(store.read().unwrap().surface_opacity, Some(0.5));
    saver.stop();
    assert!(!error.is_visible());
    let weak = Rc::downgrade(&saver);
    drop(saver);
    assert!(weak.upgrade().is_none());
}

#[test]
#[ignore = "Run individually under a private GTK display; writes only temporary preferences"]
fn autosave_recovers_busy_locks_retains_failures_and_flushes_latest_edits() {
    use std::os::fd::AsRawFd;
    gtk::init().unwrap();
    let fixture = crate::autostart::Fixture::new();
    let store = Store::new(fixture.root.join("settings.toml")).unwrap();
    store.write(&one("fps", 60_i64)).unwrap();
    let hold = || {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.path().with_extension("lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        file
    };
    let error = gtk::Label::new(None);
    error.set_visible(false);
    let ever_failed = Rc::new(Cell::new(false));
    let failed = ever_failed.clone();
    error.connect_visible_notify(move |label| {
        if label.is_visible() {
            failed.set(true);
        }
    });
    let saver = Autosave::new(
        store.clone(),
        Document::from_config(&Config::default(), None),
        &error,
    )
    .unwrap();

    // Outlast the former 750 ms lock timeout while GTK remains responsive.
    let lock = hold();
    saver.restore(one("rotation_rpm", 45.0_f64));
    glib::timeout_add_local_once(Duration::from_secs(1), move || drop(lock));
    wait(|| store.read().unwrap().rotation_rpm == Some(45.0));
    wait(|| saver.completion.borrow().is_none());
    assert!(
        !ever_failed.get(),
        "temporary contention must not flash a save error"
    );
    assert!(saver.dirty.borrow().is_empty() && saver.requests.borrow().is_empty());

    // An exhausted retry reports failure once but keeps the pending edit.
    let lock = hold();
    saver.restore(one("surface_brightness", 1.2_f64));
    wait(|| error.is_visible());
    wait(|| saver.completion.borrow().is_none());
    assert!(saver.dirty.borrow().contains_key("surface_brightness"));
    assert!(saver.requests.borrow().is_empty() && saver.debounce.borrow().is_none());
    assert_eq!(store.read().unwrap().surface_brightness, None);

    // Quit during a subsequent retry with a newer edit still debouncing.
    // Release from a test thread, not a GTK timer: shutdown flush is synchronous.
    saver.restore(one("rotation_rpm", 46.0_f64));
    saver.update(one("rotation_rpm", 47.0_f64));
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(lock);
    });
    saver.stop();
    release.join().unwrap();
    assert_eq!(store.read().unwrap().rotation_rpm, Some(47.0));
    assert_eq!(store.read().unwrap().surface_brightness, Some(1.2));
    assert!(!error.is_visible());
    assert!(saver.dirty.borrow().is_empty() && saver.requests.borrow().is_empty());
    assert!(saver.debounce.borrow().is_none() && saver.completion.borrow().is_none());
    let weak = Rc::downgrade(&saver);
    drop(saver);
    assert!(weak.upgrade().is_none());
}
