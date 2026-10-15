use super::*;
use crate::{
    app::{Options, lifecycle, metrics, window},
    arm::ArmState,
    config::{Config, PreviewMode, ScenePackage},
    playback::PlaybackFeed,
    preview::PreviewAsset,
};

#[test]
#[ignore = "requires isolated GTK/OpenGL and both complete scene packages"]
fn real_arm_packages_switch_with_shared_artwork() {
    gtk::init().expect("isolated GTK display");
    let fixture = crate::autostart::Fixture::new();
    let mut config = std::env::var_os("TALIS_SCENE_TEST_CONFIG")
        .map(|path| Config::load(std::path::Path::new(&path)).unwrap())
        .unwrap_or_default();
    config.scene.mode = PreviewMode::Arm;
    config.scene.variant = SceneVariant::Day;
    // This fixture builds GTK directly, bypassing launch-time Automatic
    // selection. Pin the initial 1440p tier for its byte-exact comparisons;
    // the later resolution sequence tests Automatic explicitly.
    if config.scene.resolution == SceneResolution::Automatic {
        config.scene.resolution = SceneResolution::Qhd;
    }
    config.render.arm_interpolation = true;
    config.render.rotate_artwork = false;
    let initial = PreviewAsset::load(&config).unwrap();
    assert!(
        initial.sleeve.is_some(),
        "Day must have matching sleeve lighting"
    );
    assert!(
        initial.record_lighting.is_some(),
        "Day record calibration must match"
    );
    let mut night_config = config.clone();
    night_config.scene.variant = SceneVariant::Night;
    let night_assets = PreviewAsset::load(&night_config).unwrap();
    assert!(
        night_assets.sleeve.is_some(),
        "Night must have matching sleeve lighting"
    );
    assert!(
        night_assets.record_lighting.is_some(),
        "Night record calibration must match"
    );
    drop(night_assets);
    let blue = image::Rgba([25, 100, 230, 255]);
    let cover = fixture.root.join("cover.png");
    image::RgbaImage::from_pixel(128, 128, blue)
        .save(&cover)
        .unwrap();
    let mut feed = PlaybackFeed::default();
    feed.set_enabled(true);
    feed.active = Some("org.mpris.MediaPlayer2.scene_fixture".into());
    feed.status = crate::playback::Status::Paused;
    feed.artwork = Some(crate::playback::ArtworkSource {
        track: "scene-switch-fixture".into(),
        url: format!("file://{}", cover.display()).into(),
    });
    let now = Instant::now();
    let mut arm = ArmState::default();
    arm.scrub(0.4);
    let state = Rc::new(RefCell::new(AppState {
        config: config.clone(),
        previous_frame: now,
        mode: PreviewMode::Arm,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm,
        arm_motion: crate::arm::motion::ArmMotion::new(0.4, 0.6),
        playback: Rc::new(RefCell::new(feed)),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    }));
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.RealSceneTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let options = Options {
        independent: true,
        experimental_wayland: false,
        fullscreen: false,
        desktop: false,
        monitor: 0,
        smoke_seconds: None,
        benchmark: false,
        gpu_profile: false,
        scene_only: false,
        shell_surface: false,
        regional_presenter: false,
        native_patch: false,
        start_minimized: false,
        size: Some((1024, 576)),
        capture: None,
        arm_progress: 0.4,
        arm_demo: false,
        now_playing: false,
        player: None,
    };
    let running = Rc::new(Cell::new(false));
    let preview = window::build(&app, &options, state.clone(), running.clone());
    let ui = preview.scene_ui.clone().unwrap();
    let weak_area = preview.area.downgrade();
    let artwork = crate::artwork::Loader::new(move || {
        if let Some(area) = weak_area.upgrade() {
            crate::app::presenter::queue_render(&area);
        }
    });
    let failure = Rc::new(RefCell::new(None));
    let renderer = lifecycle::attach(
        &preview.area,
        lifecycle::Lifecycle {
            pending_asset: Rc::new(RefCell::new(Some(initial))),
            state: state.clone(),
            metrics: Rc::new(RefCell::new(metrics::Metrics::default())),
            failure: failure.clone(),
            stats: preview.stats.clone(),
            capture: None,
            artwork: artwork.clone(),
            header_artwork: None,
            scene_ui: Some(ui.clone()),
            running,
        },
    );
    fn wait_until(label: &str, test: impl Fn() -> bool) {
        let context = glib::MainContext::default();
        // Production/release decoding is fast; debug PNG decoding of all
        // 16-bit masters can take considerably longer on a test machine.
        let deadline =
            Instant::now() + Duration::from_secs(if cfg!(debug_assertions) { 60 } else { 15 });
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if test() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "real scene load timed out: {label}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    preview.window.present();
    preview.controls.as_ref().unwrap().present();
    wait_until("initial artwork", || {
        artwork.snapshot().1.is_some() && state.borrow().artwork_ready
    });
    let (revision, art, _) = artwork.snapshot();
    let art = art.unwrap();
    let frame = fixture.root.join("frame.png");
    let capture = || {
        preview.area.make_current();
        preview.area.attach_buffers();
        let width = preview.area.width() * preview.area.scale_factor();
        let height = preview.area.height() * preview.area.scale_factor();
        let mut r = renderer.borrow_mut();
        let r = r.as_mut().unwrap();
        unsafe {
            r.render(width, height, 0.4).unwrap();
            r.check_error().unwrap();
            r.capture(width, height, &frame).unwrap();
        }
        let image = image::open(&frame).unwrap().into_rgba8();
        let (x, y, w, h) = crate::rendering::fitted_viewport(width, height, 16.0 / 9.0);
        let pixel = image.get_pixel(
            (x as f64 + w as f64 * 0.475) as u32,
            (y as f64 + h as f64 * 0.557) as u32,
        );
        assert_ne!(
            pixel, &blue,
            "record must use the selected scene's baked lighting"
        );
        assert!(
            pixel[2] > pixel[0],
            "shared blue artwork must remain blue: {:?}/{:?}, {pixel:?}",
            state.borrow().config.scene.variant,
            state.borrow().playback.borrow().status
        );
        assert!(failure.borrow().is_none());
        assert!(preview.area.error().is_none());
        assert_eq!(state.borrow().arm.position, 0.4);
        assert_eq!(state.borrow().artwork_rotation.angle(), 0.0);
        let (current_revision, current_art, _) = artwork.snapshot();
        assert_eq!(current_revision, revision);
        assert!(std::sync::Arc::ptr_eq(&art, &current_art.unwrap()));
        image
    };
    let check_shadow_lift = || {
        let set_lift = |lift| {
            state.borrow_mut().config.artwork.sleeve_shadow_lift = lift;
            let settings = state.borrow().config.artwork.clone();
            preview.area.make_current();
            unsafe {
                renderer
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .set_artwork(revision, Some(&art), &settings)
                    .unwrap();
            }
        };
        let counts = || renderer.borrow().as_ref().unwrap().cache_update_counts();
        set_lift(0.0);
        let original = capture();
        let initial = counts();
        set_lift(0.25);
        let lifted = capture();
        assert_eq!(
            counts(),
            (initial.0, initial.1, initial.2 + 1),
            "a shadow adjustment refreshes only the sleeve patch"
        );
        let cached = counts();
        assert_eq!(capture(), lifted);
        assert_eq!(counts(), cached, "unchanged shading is cached");
        let width = original.width() as i32;
        let height = original.height() as i32;
        let (x, y, w, h) = crate::rendering::fitted_viewport(width, height, 16.0 / 9.0);
        let top = height - y - h;
        let mut changed = 0;
        // This package's sleeve crop is fixed at [60,360,700,720]. Expand
        // for viewport resampling; all other pixels must remain bit-exact.
        for ((px, py, before), (_, _, after)) in
            original.enumerate_pixels().zip(lifted.enumerate_pixels())
        {
            if before != after {
                changed += 1;
                assert!(
                    (px as i32) >= x + w * 60 / 2560 - 3
                        && (px as i32) <= x + w * 760 / 2560 + 3
                        && (py as i32) >= top + h * 360 / 1440 - 3
                        && (py as i32) <= top + h * 1080 / 1440 + 3,
                    "shadow lift changed a pixel outside the sleeve: {px},{py}"
                );
            }
        }
        assert!(changed > 100, "shadow lift must visibly affect the sleeve");
        set_lift(0.0);
        assert_eq!(
            capture(),
            original,
            "zero restores authored shading exactly"
        );
        set_lift(0.25);
        assert_eq!(
            capture(),
            lifted,
            "lifting does not accumulate across redraws"
        );
        lifted
    };
    let day = check_shadow_lift();
    ui.choice.set_active_id(Some("night"));
    wait_until("Night", || {
        state.borrow().config.scene.variant == SceneVariant::Night
    });
    let night = check_shadow_lift();
    assert_ne!(
        day.as_raw(),
        night.as_raw(),
        "lighting package must actually change"
    );
    ui.choice.set_active_id(Some("day"));
    wait_until("Day", || {
        state.borrow().config.scene.variant == SceneVariant::Day
    });
    assert!(
        capture().as_raw() == day.as_raw(),
        "return to Day is pixel-exact"
    );
    // A replacement renderer must inherit the current visibility before
    // its first draw; stopped/no-player must not flash the white sleeve.
    state.borrow().playback.borrow_mut().status = crate::playback::Status::Stopped;
    let before = renderer.borrow().as_ref().unwrap().cache_update_counts().0;
    preview.area.queue_render();
    wait_until("stopped background", || {
        renderer.borrow().as_ref().unwrap().cache_update_counts().0 > before
    });
    let empty_day = capture();
    assert_ne!(empty_day, day);
    ui.choice.set_active_id(Some("night"));
    wait_until("stopped Night", || {
        state.borrow().config.scene.variant == SceneVariant::Night
    });
    let empty_night = capture();
    assert_ne!(empty_night, night);
    ui.choice.set_active_id(Some("day"));
    wait_until("stopped Day", || {
        state.borrow().config.scene.variant == SceneVariant::Day
    });
    assert_eq!(capture(), empty_day);
    {
        let s = state.borrow();
        let mut feed = s.playback.borrow_mut();
        feed.active = Some("org.mpris.MediaPlayer2.scene_fixture".into());
        feed.status = crate::playback::Status::Paused;
    }
    let before = renderer.borrow().as_ref().unwrap().cache_update_counts().0;
    preview.area.queue_render();
    wait_until("paused sleeve restored", || {
        renderer.borrow().as_ref().unwrap().cache_update_counts().0 > before
    });
    assert_eq!(capture(), day);
    if std::env::var_os("TALIS_SCENE_RESOLUTION_TEST").is_some() {
        assert!(
            ui.resolution.is_sensitive(),
            "requires installed bundled tiers"
        );
        let store = crate::preferences::Store::new(fixture.root.join("settings.toml")).unwrap();
        let saver = super::super::preferences::attach(
            &preview,
            &state.borrow(),
            crate::preferences::Session {
                store: Some(store.clone()),
                issue: None,
            },
        )
        .unwrap()
        .unwrap();
        // Same effective tier commits immediately without a GPU rebuild.
        let counts = renderer.borrow().as_ref().unwrap().cache_update_counts();
        ui.resolution.set_active_id(Some("1440p"));
        assert_eq!(state.borrow().config.scene.resolution, SceneResolution::Qhd);
        assert_eq!(
            renderer.borrow().as_ref().unwrap().cache_update_counts(),
            counts
        );
        ui.resolution.set_active_id(Some("4k"));
        assert_eq!(
            state.borrow().config.scene.resolution,
            SceneResolution::Qhd,
            "request is not a successful commit"
        );
        wait_until("4K Day", || {
            state.borrow().config.scene.resolution == SceneResolution::FourK
        });
        assert!(
            state
                .borrow()
                .config
                .scene
                .background
                .ends_with("4k/day/beauty.png")
        );
        let high_day = capture();
        assert_ne!(
            high_day, day,
            "tier must actually change the rendered assets"
        );
        ui.choice.set_active_id(Some("night"));
        wait_until("4K Night", || {
            state.borrow().config.scene.variant == SceneVariant::Night
        });
        let high_night = capture();
        assert_ne!(high_day, high_night);
        wait_until("saved 4K", || {
            store.read().unwrap().scene_resolution == Some(SceneResolution::FourK)
        });
        ui.resolution.set_active_id(Some("1080p"));
        wait_until("1080p Night", || {
            state.borrow().config.scene.resolution == SceneResolution::FullHd
        });
        assert!(
            state
                .borrow()
                .config
                .scene
                .background
                .ends_with("1080p/day/beauty.png")
        );
        let low_night = capture();
        ui.choice.set_active_id(Some("day"));
        wait_until("1080p Day", || {
            state.borrow().config.scene.variant == SceneVariant::Day
        });
        let low_day = capture();
        assert_ne!(low_day, low_night);
        wait_until("saved 1080p", || {
            store.read().unwrap().scene_resolution == Some(SceneResolution::FullHd)
        });
        // Interleaved edits coalesce into the final resolution AND variant.
        ui.resolution.set_active_id(Some("1440p"));
        ui.choice.set_active_id(Some("day"));
        ui.resolution.set_active_id(Some("4k"));
        ui.resolution.set_active_id(Some("automatic"));
        wait_until("Automatic Day", || {
            let s = state.borrow();
            s.config.scene.variant == SceneVariant::Day
                && s.config.scene.resolution == SceneResolution::Automatic
        });
        assert_eq!(
            capture(),
            low_day,
            "Automatic returns to 1080p pixel-exactly on this small target"
        );
        wait_until("saved Automatic", || {
            store.read().unwrap().scene_resolution == Some(SceneResolution::Automatic)
        });
        let restored = crate::preferences::load(Config::default(), None, Some(store)).unwrap();
        assert_eq!(restored.config.scene.resolution, SceneResolution::Automatic);
        ui.resolution.set_active_id(Some("1440p"));
        wait_until("return 1440p", || {
            state.borrow().config.scene.resolution == SceneResolution::Qhd
        });
        assert_eq!(
            capture(),
            day,
            "return to original 1440p tier is pixel-exact"
        );
        saver.stop();
    }
    artwork.stop();
    preview.controls.unwrap().destroy();
    preview.window.destroy();
}

#[test]
#[ignore = "requires an isolated GTK/OpenGL display"]
fn scene_switch_retains_state_and_recovers_from_missing_assets() {
    gtk::init().expect("isolated GTK display");
    let fixture = crate::autostart::Fixture::new();
    let day = fixture.root.join("day.png");
    let night = fixture.root.join("night.png");
    let red = image::Rgba([220, 40, 30, 255]);
    let blue = image::Rgba([25, 100, 230, 255]);
    image::RgbaImage::from_pixel(32, 18, red)
        .save(&day)
        .unwrap();
    image::RgbaImage::from_pixel(32, 18, blue)
        .save(&night)
        .unwrap();
    let mut config = Config::default();
    config.scene.background = day.clone();
    config.scene.sleeve = None;
    let mut package = ScenePackage::at(&fixture.root);
    package.background = night;
    package.sleeve = None;
    config.scene.night = Some(package);
    let now = Instant::now();
    let mut arm = ArmState::default();
    arm.set_range(0.1, 0.9);
    arm.scrub(0.4);
    let mut phase = crate::artwork_rotation::ArtworkRotation::default();
    phase.set_rotating(now, true);
    phase.advance(now + Duration::from_millis(50));
    phase.set_rotating(now + Duration::from_millis(50), false);
    let angle = phase.angle();
    let state = Rc::new(RefCell::new(AppState {
        config: config.clone(),
        previous_frame: now,
        mode: PreviewMode::Beauty,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm,
        arm_motion: crate::arm::motion::ArmMotion::new(0.4, 0.6),
        playback: Rc::new(RefCell::new(PlaybackFeed::default())),
        artwork_rotation: phase,
        artwork_ready: false,
        vinyl_ready: false,
    }));
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.SceneTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let options = Options {
        independent: true,
        experimental_wayland: false,
        fullscreen: false,
        desktop: false,
        monitor: 0,
        smoke_seconds: None,
        benchmark: false,
        gpu_profile: false,
        scene_only: false,
        shell_surface: false,
        regional_presenter: false,
        native_patch: false,
        start_minimized: false,
        size: Some((320, 180)),
        capture: None,
        arm_progress: 0.4,
        arm_demo: false,
        now_playing: false,
        player: None,
    };
    let running = Rc::new(Cell::new(false));
    let preview = window::build(&app, &options, state.clone(), running.clone());
    let ui = preview.scene_ui.clone().unwrap();
    let artwork = crate::artwork::Loader::new(|| {});
    let failure = Rc::new(RefCell::new(None));
    let rendered = Rc::new(Cell::new(0));
    let preference_store =
        crate::preferences::Store::new(fixture.root.join("preferences/settings.toml")).unwrap();
    let saver = crate::app::preferences::attach(
        &preview,
        &state.borrow(),
        crate::preferences::Session {
            store: Some(preference_store.clone()),
            issue: None,
        },
    )
    .unwrap()
    .unwrap();
    let counter = rendered.clone();
    preview.area.connect_render(move |_, _| {
        counter.set(counter.get() + 1);
        glib::Propagation::Proceed
    });
    let renderer = lifecycle::attach(
        &preview.area,
        lifecycle::Lifecycle {
            pending_asset: Rc::new(RefCell::new(Some(PreviewAsset::load(&config).unwrap()))),
            state: state.clone(),
            metrics: Rc::new(RefCell::new(metrics::Metrics::default())),
            failure: failure.clone(),
            stats: preview.stats.clone(),
            capture: None,
            artwork: artwork.clone(),
            header_artwork: None,
            scene_ui: Some(ui.clone()),
            running,
        },
    );
    fn wait_until(test: impl Fn() -> bool) {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if test() {
                return;
            }
            assert!(Instant::now() < deadline, "scene switch timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    preview.window.present();
    preview.controls.as_ref().unwrap().present();
    wait_until(|| renderer.borrow().is_some());
    let frame = fixture.root.join("frame.png");
    let assert_frame = |colour: image::Rgba<u8>| {
        preview.area.make_current();
        preview.area.attach_buffers();
        let mut r = renderer.borrow_mut();
        let r = r.as_mut().unwrap();
        let width = preview.area.width() * preview.area.scale_factor();
        let height = preview.area.height() * preview.area.scale_factor();
        unsafe {
            r.render(width, height, 0.4).unwrap();
            r.check_error().unwrap();
            r.capture(width, height, &frame).unwrap();
        }
        let image = image::open(&frame).unwrap().into_rgba8();
        let pixel = image.get_pixel(image.width() / 2, image.height() / 2);
        assert_eq!(pixel, &colour);
        let s = state.borrow();
        assert_eq!(s.arm.position, 0.4);
        assert_eq!((s.arm.start, s.arm.end), (0.1, 0.9));
        assert_eq!(s.artwork_rotation.angle(), angle);
        assert!(preview.area.error().is_none());
        assert!(failure.borrow().is_none());
    };
    assert_frame(red);
    ui.choice.set_active_id(Some("night"));
    wait_until(|| state.borrow().config.scene.variant == SceneVariant::Night);
    assert_frame(blue);
    wait_until(|| preference_store.read().unwrap().scene == Some(SceneVariant::Night));
    // A recreated GLArea must reload the selected variant, not the original
    // startup config captured by the realization callback.
    preview.window.set_child(None::<&gtk::Widget>);
    wait_until(|| renderer.borrow().is_none());
    let previous_frames = rendered.get();
    preview.window.set_child(Some(&preview.area));
    wait_until(|| renderer.borrow().is_some() && rendered.get() > previous_frames);
    assert_frame(blue);
    // Returning to the current choice cancels a pending load rather than
    // letting a stale completion replace it later.
    ui.choice.set_active_id(Some("day"));
    ui.choice.set_active_id(Some("night"));
    wait_until(|| !ui.status.is_visible());
    assert_frame(blue);
    state.borrow_mut().config.scene.background = fixture.root.join("missing.png");
    ui.restore_defaults();
    wait_until(|| ui.status.text().contains("Could not load scene"));
    assert_eq!(ui.choice.active_id().as_deref(), Some("night"));
    assert!(ui.status.is_visible());
    assert!(ui.status.text().contains("Could not load scene"));
    assert_frame(blue);
    assert_eq!(
        preference_store.read().unwrap().scene,
        Some(SceneVariant::Night)
    );
    // Reject oversized input before reading it; leave the existing GPU
    // renderer, selected scene, arm position and artwork phase unchanged.
    let oversized = fixture.root.join("oversized.png");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(crate::asset_validation::MAX_ASSET_BYTES + 1)
        .unwrap();
    state.borrow_mut().config.scene.background = oversized;
    ui.choice.set_active_id(Some("day"));
    wait_until(|| ui.status.text().contains("Could not load scene"));
    assert_eq!(ui.choice.active_id().as_deref(), Some("night"));
    assert_frame(blue);
    state.borrow_mut().config.scene.background = day;
    ui.restore_defaults();
    wait_until(|| state.borrow().config.scene.variant == SceneVariant::Day);
    assert_frame(red);
    wait_until(|| preference_store.read().unwrap().scene == Some(SceneVariant::Day));
    // A saved launch override can differ even when the runtime is already
    // at defaults. Explicit reset confirms and persists the retained scene.
    preference_store
        .write(&crate::preferences::Patch::from_iter([(
            "scene".into(),
            "night".into(),
        )]))
        .unwrap();
    ui.choice.set_active_id(Some("night"));
    ui.restore_defaults();
    wait_until(|| preference_store.read().unwrap().scene == Some(SceneVariant::Day));
    assert_frame(red);
    // Closing immediately after a request must not upload into a dead GL
    // context even if the decoder finishes after window destruction.
    ui.choice.set_active_id(Some("night"));
    artwork.stop();
    if let Some(controls) = preview.controls {
        controls.destroy();
    }
    preview.window.destroy();
    wait_until(|| renderer.borrow().is_none());
    wait_until(|| ui.choice.active_id().as_deref() == Some("day"));
    saver.stop();
    assert_eq!(
        preference_store.read().unwrap().scene,
        Some(SceneVariant::Day)
    );
}
