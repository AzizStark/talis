//! Real CLI precedence/isolation checks; all paths are owned temporary fixtures.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "talis-preferences-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn settings(&self, text: &str) -> PathBuf {
        let path = self.0.join("talis/settings.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_talis"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("XDG_CONFIG_HOME", &self.0)
            .env("XDG_SESSION_TYPE", "x11")
            .env_remove("WAYLAND_DISPLAY")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn real_cli_loads_preferences_then_partial_profile_then_temporary_overrides() {
    let fixture = Fixture::new();
    let settings = fixture.settings("schema_version=1\nfps=15\nsurface_opacity=0.4\n");
    assert!(fixture.run(&["--check"]).contains("15 FPS"));
    let profile = fixture.0.join("profile.toml");
    fs::write(&profile, "[artwork]\nrotation_rpm=45\n").unwrap();
    assert!(
        fixture
            .run(&["--config", profile.to_str().unwrap(), "--check"])
            .contains("15 FPS")
    );
    assert!(fixture.run(&["--fps", "30", "--check"]).contains("30 FPS"));
    assert!(
        fixture
            .run(&["--no-user-settings", "--check"])
            .contains("60 FPS")
    );
    assert_eq!(
        fs::read_to_string(settings).unwrap(),
        "schema_version=1\nfps=15\nsurface_opacity=0.4\n"
    );
    assert_eq!(
        fs::read_to_string(profile).unwrap(),
        "[artwork]\nrotation_rpm=45\n"
    );
}

#[test]
fn real_cli_preserves_invalid_settings_and_rejects_unowned_startup_files() {
    let fixture = Fixture::new();
    let path = fixture.settings("schema_version=99\nfps=15\n");
    let text = fixture.run(&["--check"]);
    assert!(text.contains("60 FPS") && text.contains("Preferences unavailable"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "schema_version=99\nfps=15\n"
    );
    #[cfg(feature = "desktop")]
    {
        let output = Command::new(env!("CARGO_BIN_EXE_talis"))
            .env("XDG_CONFIG_HOME", &fixture.0)
            .args([
                "--desktop",
                "--startup-config",
                path.to_str().unwrap(),
                "--check",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Unmanaged startup configuration")
        );
    }
}

#[test]
#[ignore = "Requires private GTK X11 display; checks startup flags without playback access"]
fn owned_startup_and_legacy_launch_read_latest_preferences_and_clear_pinned_source() {
    let fixture = Fixture::new();
    let settings = fixture.settings("schema_version=1\nfps=15\npreferred_player='automatic'\n");
    let snapshot = fixture.0.join("talis/startup.toml");
    let source = include_str!("../config/default.toml").replace("fps = 60", "fps = 30");
    fs::write(
        &snapshot,
        format!("# Managed by Talis for desktop startup\n{source}"),
    )
    .unwrap();
    let entry = fixture
        .0
        .join("autostart/io.github.azizstark.Talis.desktop");
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    fs::write(
        &entry,
        "[Desktop Entry]\nType=Application\nX-Talis-Managed=true\n",
    )
    .unwrap();
    let path = snapshot.to_str().unwrap();
    let modern = fixture.run(&[
        "--desktop",
        "--arm",
        "--now-playing",
        "--startup-config",
        path,
        "--startup-player",
        "spotify",
        "--check",
    ]);
    let legacy = fixture.run(&[
        "--desktop",
        "--arm",
        "--now-playing",
        "--config",
        path,
        "--start-minimized",
        "--player",
        "spotify",
        "--check",
    ]);
    for text in [modern, legacy] {
        assert!(text.contains("15 FPS") && text.contains("MPRIS: player automatic"));
    }
    assert_eq!(
        fs::read_to_string(settings).unwrap(),
        "schema_version=1\nfps=15\npreferred_player='automatic'\n"
    );
}
