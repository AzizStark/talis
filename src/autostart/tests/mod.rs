pub(super) mod fixtures;
use super::*;
#[test]
fn owned_legacy_startup_migrates_only_this_executable_and_keeps_a_backup() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let mut launch = fixture.launch();
    launch.config.scene.mode = PreviewMode::Arm;
    launch.following = true;
    launch.player = Some("org.mpris.MediaPlayer2.spotify".into());
    launch.config.artwork.size_percent = 42.0;
    manager.enable(&launch).unwrap();
    let new_entry = fs::read_to_string(&manager.entry).unwrap();
    assert!(new_entry.contains("--startup-config") && new_entry.contains("--startup-player"));
    let legacy = new_entry
        .replace("--startup-config", "--config")
        .replace("--startup-player", "--player");
    fs::write(&manager.entry, &legacy).unwrap();
    assert!(
        !manager
            .migrate_preferences(&fixture.root.join("different-executable"))
            .unwrap()
    );
    assert_eq!(fs::read_to_string(&manager.entry).unwrap(), legacy);
    assert!(manager.migrate_preferences(&launch.executable).unwrap());
    let imported = crate::preferences::Store::new(manager.snapshot.with_file_name("settings.toml"))
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(imported.label_size_percent, Some(42.0));
    assert_eq!(
        imported.preferred_player.as_deref(),
        Some("org.mpris.MediaPlayer2.spotify")
    );
    assert_eq!(fs::read_to_string(&manager.entry).unwrap(), new_entry);
    assert_eq!(
        fs::read_to_string(manager.entry.with_extension("desktop.pre-preferences")).unwrap(),
        legacy
    );
    assert!(!manager.migrate_preferences(&launch.executable).unwrap());
    assert!(manager.owns_snapshot(&manager.snapshot).unwrap());
    assert!(
        !manager
            .owns_snapshot(&fixture.root.join("other.toml"))
            .unwrap()
    );
}
#[test]
fn first_run_prompt_state_is_read_only_until_answered_and_never_enables_startup() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    assert!(manager.prompt_pending().unwrap());
    assert!(!fixture.root.join("talis").exists());
    assert!(!fixture.root.join("autostart").exists());
    manager.dismiss_prompt().unwrap();
    assert!(!manager.prompt_pending().unwrap());
    assert!(!manager.enabled().unwrap());
    assert!(!fixture.root.join("autostart").exists());
    assert!(!manager.snapshot.exists());
    manager.dismiss_prompt().unwrap();
    let enabled = Fixture::new();
    enabled.manager().enable(&enabled.launch()).unwrap();
    assert!(!enabled.manager().prompt_pending().unwrap());
    assert!(!enabled.manager().prompt_seen.exists());
}
#[test]
fn prompt_marker_rejects_unmanaged_content_and_symlinks() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    fs::create_dir(fixture.root.join("talis")).unwrap();
    fs::write(&manager.prompt_seen, "not owned").unwrap();
    assert!(manager.prompt_pending().is_err());
    assert!(manager.dismiss_prompt().is_err());
    assert_eq!(
        fs::read_to_string(&manager.prompt_seen).unwrap(),
        "not owned"
    );
    fs::remove_file(&manager.prompt_seen).unwrap();
    std::os::unix::fs::symlink(fixture.root.join("elsewhere"), &manager.prompt_seen).unwrap();
    assert!(manager.prompt_pending().is_err());
    assert!(manager.dismiss_prompt().is_err());
    assert!(!fixture.root.join("elsewhere").exists());
}
#[test]
fn wayland_startup_is_no_build_session_scoped_and_has_explicit_loader() {
    let fixture = Fixture::new();
    let mut launch = fixture.launch();
    let library = fixture.root.join("libgtk4-layer-shell.so");
    fs::write(&library, b"fixture").unwrap();
    launch.wayland_library = Some(library.clone());
    launch.monitor = 2;
    fixture.manager().enable(&launch).unwrap();
    let entry = fs::read_to_string(&fixture.manager().entry).unwrap();
    assert!(entry.contains("--experimental-wayland") && entry.contains("--start-minimized"));
    assert!(entry.contains("OnlyShowIn=KDE;") && entry.contains("GDK_BACKEND=wayland"));
    assert!(entry.contains("LD_PRELOAD=libgtk4-layer-shell.so"));
    assert!(!entry.contains("cargo") && !entry.contains("scripts/wayland-desktop.sh"));
    let invalid = fixture.root.join("bad library.so");
    fs::write(&invalid, b"fixture").unwrap();
    launch.wayland_library = Some(invalid);
    assert!(fixture.manager().enable(&launch).is_err());
    assert_eq!(entry, fs::read_to_string(&fixture.manager().entry).unwrap());
    launch.wayland_library = Some(library);
    launch
        .graphics_environment
        .push(("GDK_DEBUG".into(), "gl-glx".into()));
    assert!(fixture.manager().enable(&launch).is_err());
}

#[test]
fn wayland_startup_supports_bundle_directories_with_spaces() {
    let fixture = Fixture::new();
    let directory = fixture.root.join("portable with spaces");
    fs::create_dir(&directory).unwrap();
    let library = directory.join("libgtk4-layer-shell.so.0");
    fs::write(&library, b"fixture").unwrap();
    let mut launch = fixture.launch();
    launch.wayland_library = Some(library);
    fixture.manager().enable(&launch).unwrap();
    let entry = fs::read_to_string(&fixture.manager().entry).unwrap();
    assert!(entry.contains("\"LD_PRELOAD=libgtk4-layer-shell.so.0\""));
    assert!(entry.contains(&format!("\"LD_LIBRARY_PATH={}\"", directory.display())));
    assert!(!entry.contains("cargo") && !entry.contains("scripts/run.sh"));
}
#[test]
fn enable_disable_is_idempotent_and_snapshots_absolute_paths_and_settings() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    assert!(!manager.enabled().unwrap());
    assert!(
        !fixture.root.join("autostart").exists(),
        "Reading must not create startup files"
    );
    let mut launch = fixture.launch();
    launch.config.scene.mode = PreviewMode::Arm;
    launch.config.scene.variant = crate::config::SceneVariant::Night;
    launch.config.artwork.size_percent = 42.0;
    launch.config.artwork.vinyl_opacity = 0.4;
    launch.config.artwork.vinyl_brightness = 1.5;
    launch.config.render.arm_range_start = 0.2;
    launch.config.render.arm_range_end = 0.8;
    launch.following = true;
    launch.player = Some("org.mpris.MediaPlayer2.spotify".into());
    manager.enable(&launch).unwrap();
    manager.enable(&launch).unwrap();
    assert!(manager.enabled().unwrap());
    let saved = Config::load(&manager.snapshot).unwrap();
    assert_eq!(saved.artwork.size_percent, 42.0);
    assert_eq!(saved.artwork.vinyl_opacity, 0.4);
    assert_eq!(saved.artwork.vinyl_brightness, 1.5);
    assert_eq!(
        (saved.render.arm_range_start, saved.render.arm_range_end),
        (0.2, 0.8)
    );
    assert!(saved.scene.background.is_absolute());
    assert!(saved.scene.sleeve.as_ref().unwrap().is_absolute());
    assert!(saved.scene.record_lighting.as_ref().unwrap().is_absolute());
    assert!(saved.scene.vinyl_detail.as_ref().unwrap().is_absolute());
    assert!(saved.scene.buttons.as_ref().unwrap().is_absolute());
    assert_eq!(saved.scene.variant, crate::config::SceneVariant::Night);
    let night = saved.scene.night.as_ref().unwrap();
    assert!(
        night.background.is_absolute()
            && night.arm_poses.is_absolute()
            && night.arm_flow.is_absolute()
    );
    assert!(night.sleeve.as_ref().unwrap().is_absolute());
    assert!(night.record_lighting.as_ref().unwrap().is_absolute());
    assert!(night.vinyl_detail.as_ref().unwrap().is_absolute());
    assert!(night.buttons.as_ref().unwrap().is_absolute());
    let entry = fs::read_to_string(&manager.entry).unwrap();
    assert!(entry.contains("--start-minimized") && entry.contains("--now-playing"));
    assert!(!entry.contains("cargo") && !entry.contains("--benchmark"));
    fs::write(&manager.entry, format!("{entry}Hidden=true\n")).unwrap();
    assert!(!manager.enabled().unwrap());
    manager.disable().unwrap();
    manager.disable().unwrap();
    assert!(!manager.enabled().unwrap());
    assert!(
        manager.snapshot.exists(),
        "Keep the snapshot recoverable when disabling"
    );
}
#[test]
fn unmanaged_symlink_and_failed_paths_are_preserved() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    fs::create_dir_all(manager.entry.parent().unwrap()).unwrap();
    fs::write(&manager.entry, "[Desktop Entry]\nName=Other\n").unwrap();
    assert!(manager.enable(&fixture.launch()).is_err());
    assert!(manager.disable().is_err());
    assert_eq!(
        fs::read_to_string(&manager.entry).unwrap(),
        "[Desktop Entry]\nName=Other\n"
    );
    fs::remove_file(&manager.entry).unwrap();
    fs::create_dir_all(manager.snapshot.parent().unwrap()).unwrap();
    fs::write(&manager.snapshot, "# User config\n").unwrap();
    assert!(manager.enable(&fixture.launch()).is_err());
    fs::remove_file(&manager.snapshot).unwrap();
    #[cfg(unix)]
    {
        let target = fixture.root.join("unrelated");
        fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, &manager.entry).unwrap();
        assert!(manager.enable(&fixture.launch()).is_err());
        assert!(manager.disable().is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    }
    assert!(Manager::new("relative".into()).is_err());
    assert!(quote_exec("line\nbreak").is_err());
}
#[test]
fn exec_values_escape_reserved_characters_without_a_shell() {
    assert_eq!(
        quote_exec("a b$`\"\\%c").unwrap(),
        "\"a b\\\\$\\\\`\\\\\"\\\\\\\\%%c\""
    );
    let fixture = Fixture::new();
    let mut launch = fixture.launch();
    launch.graphics_environment = vec![("LD_PRELOAD".into(), "bad".into())];
    assert!(fixture.manager().enable(&launch).is_err());
    assert!(!fixture.manager().entry.exists());
}
#[cfg(feature = "desktop")]
#[test]
#[ignore = "Requires a private dbus-run-session; set TALIS_AUTOSTART_TEST_BUS=1"]
fn desktop_launcher_preserves_reserved_paths_arguments_and_graphics_flags() {
    use gtk::gio::prelude::*;
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::env::var("TALIS_AUTOSTART_TEST_BUS").as_deref(),
        Ok("1")
    );
    let fixture = Fixture::new();
    let root = fixture.root.join("space $ ` quote\" slash\\ percent% ü");
    fs::create_dir(&root).unwrap();
    let output = fixture.root.join("arguments.json");
    let helper = root.join("fake talis");
    fs::write(&helper, format!("#!/usr/bin/python3\nimport json, os, sys\nfrom pathlib import Path\nPath({}).write_text(json.dumps([sys.argv[1:], os.getcwd(), os.environ.get('GDK_DEBUG')]))\n", serde_json::to_string(&output).unwrap())).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let manager = Manager::new(root.clone()).unwrap();
    let mut launch = fixture.launch();
    launch.executable = helper;
    launch.directory = root.clone();
    launch.config.scene.mode = PreviewMode::Arm;
    launch.following = true;
    launch.player = Some("org.mpris.MediaPlayer2.spotify".into());
    launch.graphics_environment = vec![("GDK_DEBUG".into(), "gl-glx".into())];
    manager.enable(&launch).unwrap();
    let app = gtk::gio::DesktopAppInfo::from_filename(&manager.entry).expect("valid desktop entry");
    app.launch(&[], None::<&gtk::gio::AppLaunchContext>)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !output.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let captured: (Vec<String>, String, String) =
        serde_json::from_str(&fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(captured.0, launch.arguments(&manager.snapshot).unwrap());
    assert_eq!(captured.1, root.to_str().unwrap());
    assert_eq!(captured.2, "gl-glx");
}
