use super::*;
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn identity_is_stable_scoped_and_valid_without_exposing_display_addresses() {
    let id = identity("GdkX11Display", ":0", 0);
    assert!(gio::Application::id_is_valid(&id));
    assert_eq!(id, identity("GdkX11Display", ":0", 0));
    assert_ne!(id, identity("GdkX11Display", ":0", 1));
    assert_ne!(id, identity("GdkX11Display", ":1", 0));
    assert_ne!(id, identity("GdkWaylandDisplay", ":0", 0));
    assert!(!id.contains(":0"));
}

#[test]
fn activations_wait_for_settings_and_coalesce_during_startup() {
    let activation = Activation::default();
    activation.request();
    activation.request();
    let shown = Rc::new(Cell::new(0));
    let counter = shown.clone();
    activation.bind(move || counter.set(counter.get() + 1));
    assert_eq!(shown.get(), 1);
    activation.request();
    assert_eq!(shown.get(), 2);
}

#[test]
fn lease_is_exclusive_recoverable_and_refuses_symlinks() {
    let fixture = crate::autostart::Fixture::new();
    let path = fixture.root.join("instance.lock");
    let owner = lease::Lease::at(&path).unwrap().unwrap();
    assert!(lease::Lease::at(&path).unwrap().is_none());
    let next = fixture.root.join("other-monitor.lock");
    assert!(lease::Lease::at(&next).unwrap().is_some());
    drop(owner);
    assert!(lease::Lease::at(&path).unwrap().is_some());
    let alias = fixture.root.join("alias.lock");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(lease::Lease::at(&alias).is_err());
    assert_eq!(fs::metadata(path).unwrap().len(), 0);
}

fn private_bus() {
    assert_eq!(
        std::env::var("TALIS_INSTANCE_TEST_BUS").as_deref(),
        Ok("1"),
        "Run only in a private dbus-run-session"
    );
}

fn child(id: &str, path: &std::path::Path, mode: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "app::instance::tests::child_launch",
            "--nocapture",
        ])
        .env("TALIS_INSTANCE_TEST_ID", id)
        .env("TALIS_INSTANCE_TEST_LEASE", path)
        .env("TALIS_INSTANCE_TEST_MODE", mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn finish(mut child: Child) -> std::process::Output {
    let deadline = Instant::now() + Duration::from_secs(12);
    while child.try_wait().unwrap().is_none() {
        while gtk::glib::MainContext::default().iteration(false) {}
        if Instant::now() > deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "child timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn wait(check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        while gtk::glib::MainContext::default().iteration(false) {}
        assert!(Instant::now() < deadline, "activation did not arrive");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "Requires private dbus-run-session and TALIS_INSTANCE_TEST_BUS=1; subprocess helper"]
fn child_launch() {
    private_bus();
    let Ok(id) = std::env::var("TALIS_INSTANCE_TEST_ID") else {
        return;
    };
    let path = std::path::PathBuf::from(std::env::var_os("TALIS_INSTANCE_TEST_LEASE").unwrap());
    match Instance::register(&id, lease::Lease::at(&path).unwrap()).unwrap() {
        Claim::Activated => println!("ACTIVATED"),
        Claim::Primary(instance) => {
            if std::env::var("TALIS_INSTANCE_TEST_MODE").as_deref() == Ok("disconnect") {
                let connection = instance._application.dbus_connection().unwrap();
                assert!(!connection.exits_on_close());
                connection.close_sync(None::<&gio::Cancellable>).unwrap();
                assert!(lease::Lease::at(&path).unwrap().is_none());
                assert!(
                    Instance::register(&id, None).is_err(),
                    "lost IPC must not authorize another renderer"
                );
                println!("DISCONNECTED_OWNER_RETAINED");
                return;
            }
            if std::env::var("TALIS_INSTANCE_TEST_MODE").as_deref() == Ok("hold") {
                fs::write(path.with_extension("ready"), "ready").unwrap();
                std::thread::sleep(Duration::from_secs(5));
                panic!("crash-recovery fixture was not terminated by its parent");
            }
            assert_eq!(
                std::env::var("TALIS_INSTANCE_TEST_MODE").as_deref(),
                Ok("race")
            );
            let shown = Rc::new(Cell::new(0));
            let counter = shown.clone();
            instance
                .activation
                .bind(move || counter.set(counter.get() + 1));
            let deadline = Instant::now() + Duration::from_secs(8);
            while shown.get() < 5 {
                while gtk::glib::MainContext::default().iteration(false) {}
                assert!(
                    Instant::now() < deadline,
                    "not all racing launches activated the owner"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            println!("PRIMARY");
        }
    }
}

#[test]
#[ignore = "Requires private dbus-run-session and TALIS_INSTANCE_TEST_BUS=1"]
fn remote_launches_activate_one_owner_without_reinitializing_it() {
    private_bus();
    let fixture = crate::autostart::Fixture::new();
    let id = identity("test", &fixture.root.to_string_lossy(), 0);
    let path = fixture.root.join("instance.lock");
    let Claim::Primary(instance) =
        Instance::register(&id, lease::Lease::at(&path).unwrap()).unwrap()
    else {
        panic!("first launch must own the scene");
    };
    let first = finish(child(&id, &path, "remote"));
    assert!(String::from_utf8_lossy(&first.stdout).contains("ACTIVATED"));
    wait(|| instance.activation.pending.get());
    let shown = Rc::new(Cell::new(0));
    let counter = shown.clone();
    instance
        .activation
        .bind(move || counter.set(counter.get() + 1));
    assert_eq!(
        shown.get(),
        1,
        "activation while startup was pending is delivered once"
    );
    finish(child(&id, &path, "remote"));
    wait(|| shown.get() == 2);
    assert_eq!(shown.get(), 2);
}

#[test]
#[ignore = "Requires private dbus-run-session and TALIS_INSTANCE_TEST_BUS=1"]
fn simultaneous_cold_launches_elect_exactly_one_owner() {
    private_bus();
    let fixture = crate::autostart::Fixture::new();
    let id = identity("race-test", &fixture.root.to_string_lossy(), 0);
    let path = fixture.root.join("instance.lock");
    let children: Vec<_> = (0..6).map(|_| child(&id, &path, "race")).collect();
    let outputs: Vec<_> = children.into_iter().map(finish).collect();
    assert_eq!(
        outputs
            .iter()
            .filter(|o| String::from_utf8_lossy(&o.stdout).contains("PRIMARY"))
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|o| String::from_utf8_lossy(&o.stdout).contains("ACTIVATED"))
            .count(),
        5
    );
    assert!(
        lease::Lease::at(&path).unwrap().is_some(),
        "exiting releases admission"
    );
}

#[test]
#[ignore = "Requires private dbus-run-session and TALIS_INSTANCE_TEST_BUS=1"]
fn disconnected_bus_does_not_exit_the_owner_or_allow_a_duplicate() {
    private_bus();
    let fixture = crate::autostart::Fixture::new();
    let id = identity("disconnect-test", &fixture.root.to_string_lossy(), 0);
    let path = fixture.root.join("instance.lock");
    let output = finish(child(&id, &path, "disconnect"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("DISCONNECTED_OWNER_RETAINED"));
    assert!(lease::Lease::at(&path).unwrap().is_some());
}

#[test]
#[ignore = "Requires private dbus-run-session and TALIS_INSTANCE_TEST_BUS=1"]
fn process_death_releases_the_lease_without_deleting_the_file() {
    private_bus();
    let fixture = crate::autostart::Fixture::new();
    let id = identity("death-test", &fixture.root.to_string_lossy(), 0);
    let path = fixture.root.join("instance.lock");
    let mut owner = child(&id, &path, "hold");
    wait(|| path.with_extension("ready").is_file());
    assert!(lease::Lease::at(&path).unwrap().is_none());
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(path.is_file());
    let lease = lease::Lease::at(&path).unwrap().unwrap();
    drop(lease);
    assert!(lease::Lease::at(&path).unwrap().is_some());
}

#[test]
#[ignore = "Run individually: requires private GTK display, dbus-run-session and TALIS_INSTANCE_TEST_BUS=1"]
fn remote_activation_restores_only_settings_and_keeps_the_scene_child() {
    use super::super::{state::AppState, tray::TrayUi};
    private_bus();
    gtk::init().unwrap();
    let fixture = crate::autostart::Fixture::new();
    let id = identity("settings-test", &fixture.root.to_string_lossy(), 0);
    let path = fixture.root.join("instance.lock");
    let Claim::Primary(instance) =
        Instance::register(&id, lease::Lease::at(&path).unwrap()).unwrap()
    else {
        panic!("first launch must own the scene");
    };
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.ActivationTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.set_default();
    app.register(None::<&gio::Cancellable>).unwrap();
    let scene = gtk::ApplicationWindow::builder().application(&app).build();
    let controls = gtk::ApplicationWindow::builder().application(&app).build();
    controls.set_child(Some(&gtk::Label::new(Some("Settings UI"))));
    let area = gtk::GLArea::new();
    scene.set_child(Some(&area));
    let state = Rc::new(RefCell::new(AppState {
        config: crate::config::Config::default(),
        previous_frame: Instant::now(),
        mode: crate::config::PreviewMode::Arm,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm: Default::default(),
        arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
        playback: Rc::new(RefCell::new(crate::playback::PlaybackFeed::default())),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    }));
    let tray = TrayUi::new(&controls, state);
    let preview = AppWindows {
        window: scene.clone(),
        controls: Some(controls.clone()),
        area: area.clone(),
        stats: gtk::Label::new(None),
        arm_ui: None,
        media_ui: None,
        scene_ui: None,
        tray_ui: Some(tray.clone()),
        mate_overlay: None,
        preferences_error: gtk::Label::new(None),
        defaults_ui: None,
    };
    instance.bind_settings(&preview);
    for _ in 0..2 {
        controls.set_visible(false);
        finish(child(&id, &path, "remote"));
        wait(|| controls.is_mapped());
        assert_eq!(scene.child(), Some(area.clone().upcast()));
        assert!(controls.child().unwrap().is::<gtk::Label>());
        assert_eq!(
            app.windows().len(),
            2,
            "no extra scene, patch or Settings window"
        );
    }
    tray.stop();
    controls.destroy();
    scene.destroy();
}
