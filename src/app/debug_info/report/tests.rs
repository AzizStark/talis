use super::*;
use crate::{
    config::SceneVariant,
    playback::{
        ArtworkSource, PlaybackSnapshot, Status,
        transport::{Capabilities, Target},
    },
};
use std::time::Duration;

#[test]
fn report_uses_an_allowlist_not_raw_settings_player_metadata_or_errors() {
    let now = Instant::now();
    let mut config = Config::default();
    config.scene.background = "/home/private-user/custom-scene.png".into();
    config.scene.variant = SceneVariant::Night;
    let feed = PlaybackFeed {
        enabled: true,
        active: Some("org.mpris.MediaPlayer2.firefox.instance_private123".into()),
        preferred: Some("org.mpris.MediaPlayer2.firefox.instance_private123".into()),
        notice: "Private song title /home/private-user/private.mp3".into(),
        artwork: Some(ArtworkSource {
            track: "Private song title".into(),
            url: "https://secret.example/image?token=private".into(),
        }),
        artwork_owner: Some(":1.12345".into()),
        control_notice: Some("private failure details".into()),
        status: Status::Playing,
        snapshot: Some(PlaybackSnapshot::demo(now)),
        transport: Some(Target {
            player: "org.mpris.MediaPlayer2.firefox.instance_private123".into(),
            owner: ":1.12345".into(),
            status: Status::Playing,
            capabilities: Capabilities {
                control: true,
                play: true,
                pause: true,
                next: true,
                previous: false,
            },
            observed_at: now,
        }),
        ..Default::default()
    };
    let mut report = Report::new();
    report.settings(&config);
    report.playback(&feed, now);
    report.graphics(None);
    let text = report.finish();
    for forbidden in [
        "private",
        "/home/",
        "secret.example",
        ":1.12345",
        "instance_",
    ] {
        assert!(!text.contains(forbidden), "leaked {forbidden}");
    }
    for expected in [
        "Scene: night",
        "Active player application: firefox",
        "Fresh track timing: true",
        "Artwork supplied: true",
        "Play / pause available: true",
        "Previous available: false",
        "Renderer initialized: false",
        "Software rendering: Unavailable",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    let mut stale = Report::new();
    stale.playback(&feed, now + Duration::from_secs(7));
    let text = stale.finish();
    assert!(text.contains("Fresh track timing: false"));
    assert!(text.contains("Play / pause available: false"));
}

#[test]
fn external_values_are_bounded_single_line_and_paths_are_redacted() {
    assert_eq!(safe_value("Mesa\nRenderer\tGPU"), "Mesa Renderer GPU");
    assert_eq!(safe_value("GNU/Linux Mesa/X.org"), "GNU/Linux Mesa/X.org");
    assert_eq!(
        safe_value("(/home/private/gpu) https://host/image user@host C:\\Users\\private"),
        "[redacted] [redacted] [redacted] [redacted]"
    );
    assert_eq!(safe_value("GPU\u{202e}evil\u{2066}\u{0000}"), "GPUevil");
    assert_eq!(safe_value(&"é".repeat(1000)).chars().count(), 256);
}

#[test]
fn graphics_snapshot_includes_loaded_pixels_without_requesting_a_context() {
    let mut report = Report::new();
    report.graphics(Some(&RendererInfo {
        version: "4.6".into(),
        shading_language: "4.60".into(),
        vendor: "Mesa/X.org".into(),
        renderer: "llvmpipe".into(),
        scene_size: (3840, 2160),
    }));
    let text = report.finish();
    assert!(text.contains("GPU renderer: llvmpipe"));
    assert!(text.contains("Software rendering: Detected (CPU)"));
    assert!(text.contains("Loaded scene pixels: 3840x2160"));
    let mut hardware = Report::new();
    hardware.graphics(Some(&RendererInfo {
        version: "4.6".into(),
        shading_language: "4.60".into(),
        vendor: "Mesa/X.org".into(),
        renderer: "SVGA3D; build: RELEASE; LLVM;".into(),
        scene_size: (1920, 1080),
    }));
    assert!(
        hardware
            .finish()
            .contains("Software rendering: Not detected")
    );
}
