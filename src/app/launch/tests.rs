use super::*;

fn normal() -> Options {
    Options {
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
    }
}

#[test]
fn manual_and_existing_autostart_launches_use_the_same_native_presenter() {
    for minimized in [false, true] {
        let mut launch = normal();
        launch.start_minimized = minimized;
        launch.apply_desktop_defaults(PreviewMode::Arm);
        assert!(launch.native_patch);
        assert!(launch.normal_desktop());
    }
    let mut beauty = normal();
    beauty.apply_desktop_defaults(PreviewMode::Beauty);
    assert!(!beauty.native_patch);
}

#[test]
fn diagnostics_and_explicit_presenters_keep_their_existing_behavior() {
    let base = normal();
    for mut launch in [
        Options {
            independent: true,
            ..base.clone()
        },
        Options {
            desktop: false,
            ..base.clone()
        },
        Options {
            scene_only: true,
            ..base.clone()
        },
        Options {
            benchmark: true,
            ..base.clone()
        },
        Options {
            gpu_profile: true,
            ..base.clone()
        },
        Options {
            arm_demo: true,
            ..base.clone()
        },
        Options {
            smoke_seconds: Some(1),
            ..base.clone()
        },
        Options {
            capture: Some("capture.png".into()),
            ..base.clone()
        },
    ] {
        assert!(!launch.normal_desktop());
        launch.apply_desktop_defaults(PreviewMode::Arm);
        assert!(!launch.native_patch);
    }
    for mut launch in [
        Options {
            experimental_wayland: true,
            ..base.clone()
        },
        Options {
            regional_presenter: true,
            ..base.clone()
        },
    ] {
        launch.apply_desktop_defaults(PreviewMode::Arm);
        assert!(!launch.native_patch);
    }
    let mut explicit = Options {
        benchmark: true,
        native_patch: true,
        ..base
    };
    explicit.apply_desktop_defaults(PreviewMode::Arm);
    assert!(
        explicit.native_patch,
        "explicit benchmark presenter remains authoritative"
    );
}

#[test]
fn autostart_arguments_do_not_need_a_presenter_flag_or_entry_migration() {
    let fixture = crate::autostart::Fixture::new();
    let mut launch = fixture.launch();
    launch.config.scene.mode = PreviewMode::Arm;
    let args = launch
        .arguments(&fixture.root.join("startup.toml"))
        .unwrap();
    assert!(args.iter().any(|arg| arg == "--desktop"));
    assert!(args.iter().any(|arg| arg == "--arm"));
    assert!(!args.iter().any(|arg| arg == "--native-patch"));
    let mut options = normal();
    options.start_minimized = true;
    options.apply_desktop_defaults(launch.config.scene.mode);
    assert!(options.native_patch);
}
