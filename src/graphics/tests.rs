use super::*;

fn context_error(backend: DisplayBackend) -> anyhow::Error {
    ContextInitializationError::new("injected context creation failure".into(), backend).into()
}

#[test]
fn runtime_version_selects_the_right_flag_without_requiring_development_files() {
    assert_eq!(setting("auto", false, (4, 14)).unwrap(), None);
    assert_eq!(
        setting("glx", false, (4, 14)).unwrap(),
        Some(("GDK_DEBUG", "gl-glx"))
    );
    assert_eq!(
        setting("glx", false, (4, 16)).unwrap(),
        Some(("GDK_DISABLE", "egl"))
    );
    assert!(setting("glx", true, (4, 24)).is_err());
    assert!(setting("invalid", false, (4, 14)).is_err());
}

#[test]
fn presentation_uses_version_appropriate_gl_without_overriding_user_choice() {
    for version in [(4, 8), (4, 10), (4, 12)] {
        assert_eq!(presentation_renderer(version, false), Some("gl"));
    }
    for version in [(4, 14), (4, 16), (4, 17)] {
        assert_eq!(presentation_renderer(version, false), Some("ngl"));
    }
    for version in [(4, 18), (4, 22), (4, 24)] {
        assert_eq!(presentation_renderer(version, false), Some("gl"));
    }
    for version in [(4, 8), (4, 14), (4, 22)] {
        assert_eq!(presentation_renderer(version, true), None);
    }
}

#[test]
fn recovery_is_only_for_automatic_native_x11_context_creation() {
    let error = context_error(DisplayBackend::X11).context("scene startup");
    assert!(eligible(&error, "auto", false, "gl,gl-shaders", "vulkan"));
    assert!(!eligible(&error, "glx", false, "", ""));
    assert!(!eligible(&error, "auto", true, "", ""));
    assert!(!eligible(
        &context_error(DisplayBackend::Wayland),
        "auto",
        false,
        "",
        ""
    ));
    for message in [
        "shader compilation failed",
        "asset missing",
        "OpenGL initialization failed",
        "OpenGL error 0x506",
    ] {
        assert!(!eligible(&anyhow::anyhow!(message), "auto", false, "", ""));
    }
}

#[test]
fn manual_gl_choices_and_disables_are_never_overridden() {
    let error = context_error(DisplayBackend::X11);
    for debug in [
        "gl-glx",
        "gl-egl",
        "gl-gles",
        "gl-legacy",
        "gl-disable",
        "all",
        "misc:GL-GLX",
        "gl-shaders;gl-egl",
    ] {
        assert!(!eligible(&error, "auto", false, debug, ""), "{debug}");
    }
    for disabled in [
        "gl",
        "gl-api",
        "gles-api",
        "egl",
        "glx",
        "all",
        "vulkan,glx",
    ] {
        assert!(!eligible(&error, "auto", false, "", disabled), "{disabled}");
    }
}

#[test]
fn wayland_retry_only_handles_initial_context_failure_without_manual_gl_choices() {
    let error = context_error(DisplayBackend::Wayland).context("scene startup");
    assert!(wayland_eligible(
        &error,
        "auto",
        "opengl",
        "vulkan",
        false,
        (4, 22)
    ));
    assert!(!wayland_eligible(&error, "auto", "", "", true, (4, 22)));
    assert!(!wayland_eligible(&error, "auto", "", "", false, (4, 14)));
    assert!(!wayland_eligible(&error, "glx", "", "", false, (4, 22)));
    assert!(!wayland_eligible(
        &context_error(DisplayBackend::X11),
        "auto",
        "",
        "",
        false,
        (4, 22)
    ));
    assert!(!wayland_eligible(
        &context_error(DisplayBackend::Other),
        "auto",
        "",
        "",
        false,
        (4, 22)
    ));
    assert!(!wayland_eligible(
        &anyhow::anyhow!("shader compilation failed"),
        "auto",
        "",
        "",
        false,
        (4, 22)
    ));
    for debug in ["gl-prefer-gl", "gl-gles", "gl-egl", "gl-disable", "all"] {
        assert!(
            !wayland_eligible(&error, "auto", debug, "", false, (4, 22)),
            "{debug}"
        );
    }
    for disabled in ["gles-api", "gl-api", "gl", "egl", "glx", "all"] {
        assert!(
            !wayland_eligible(&error, "auto", "", disabled, false, (4, 22)),
            "{disabled}"
        );
    }
}

#[test]
fn wayland_retry_command_preserves_cwd_arguments_and_unrelated_flags() {
    let command = wayland_recovery_command(
        OsStr::new("/example/Talis app/talis"),
        ["--desktop", "--scene", "night", "a 'quoted' path ü.toml"],
        "vulkan",
    );
    assert_eq!(
        command.get_program(),
        OsStr::new("/example/Talis app/talis")
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["--desktop", "--scene", "night", "a 'quoted' path ü.toml"]
    );
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        [
            (
                OsStr::new("GDK_DISABLE"),
                Some(OsStr::new("vulkan,gles-api"))
            ),
            (OsStr::new("TALIS_WAYLAND_GL_RETRY"), Some(OsStr::new("1"))),
        ]
    );
    assert!(command.get_current_dir().is_none());
}

#[test]
fn retry_command_preserves_arguments_and_only_sets_its_child_environment() {
    let command = recovery_command(
        OsStr::new("/example/Talis app/talis"),
        [
            "--scene",
            "night",
            "--config",
            "a 'quoted' path ü.toml",
            "--desktop",
            "--monitor",
            "2",
        ],
    );
    assert_eq!(
        command.get_program(),
        OsStr::new("/example/Talis app/talis")
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "--scene",
            "night",
            "--config",
            "a 'quoted' path ü.toml",
            "--desktop",
            "--monitor",
            "2"
        ]
    );
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        [(OsStr::new("TALIS_GL_BACKEND"), Some(OsStr::new("glx")))]
    );
    assert!(
        command.get_current_dir().is_none(),
        "inherit the package working directory"
    );
}

#[test]
fn process_replacement_preserves_cwd_arguments_and_stops_after_one_retry() {
    let fixture = crate::autostart::Fixture::new();
    for both_fail in [false, true] {
        let output = Command::new(std::env::current_exe().unwrap())
            .current_dir(&fixture.root)
            .args([
                "--ignored",
                "--exact",
                "graphics::tests::recovery_child",
                "--nocapture",
                "--skip",
                "a 'quoted' argument ü",
            ])
            .env("TALIS_RECOVERY_TEST_CHILD", "1")
            .env(
                "TALIS_RECOVERY_TEST_BOTH_FAIL",
                if both_fail { "1" } else { "0" },
            )
            .env("TALIS_RECOVERY_TEST_CWD", &fixture.root)
            .env("TALIS_GL_BACKEND", "auto")
            .env("XDG_SESSION_TYPE", "x11")
            .env("GDK_DEBUG", "")
            .env("GDK_DISABLE", "")
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            output.status.success(),
            !both_fail,
            "{text}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(text.matches("FIRST_ATTEMPT").count(), 1);
        assert_eq!(text.matches("GLX_ATTEMPT").count(), 1);
        assert!(text.contains(if both_fail {
            "FINAL_FAILURE"
        } else {
            "RECOVERED"
        }));
    }
}

#[test]
fn wayland_process_replacement_preserves_flags_and_stops_after_one_retry() {
    let fixture = crate::autostart::Fixture::new();
    for both_fail in [false, true] {
        let output = Command::new(std::env::current_exe().unwrap())
            .current_dir(&fixture.root)
            .args([
                "--ignored",
                "--exact",
                "graphics::tests::wayland_recovery_child",
                "--nocapture",
                "--skip",
                "a 'quoted' argument ü",
            ])
            .env("TALIS_WAYLAND_RETRY_TEST_CHILD", "1")
            .env(
                "TALIS_WAYLAND_RETRY_TEST_BOTH_FAIL",
                if both_fail { "1" } else { "0" },
            )
            .env("TALIS_WAYLAND_RETRY_TEST_CWD", &fixture.root)
            .env("TALIS_GL_BACKEND", "auto")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("GDK_DEBUG", "opengl")
            .env("GDK_DISABLE", "vulkan")
            .env_remove("TALIS_WAYLAND_GL_RETRY")
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            output.status.success(),
            !both_fail,
            "{text}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(text.matches("FIRST_WAYLAND_ATTEMPT").count(), 1);
        assert_eq!(text.matches("DESKTOP_GL_ATTEMPT").count(), 1);
        assert!(text.contains(if both_fail {
            "FINAL_WAYLAND_FAILURE"
        } else {
            "WAYLAND_RECOVERED"
        }));
    }
}

#[test]
#[ignore = "Subprocess helper; invoked by the Wayland graphics recovery test"]
fn wayland_recovery_child() {
    if std::env::var("TALIS_WAYLAND_RETRY_TEST_CHILD").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::path::PathBuf::from(std::env::var_os("TALIS_WAYLAND_RETRY_TEST_CWD").unwrap())
    );
    assert!(std::env::args_os().any(|arg| arg == OsStr::new("a 'quoted' argument ü")));
    if std::env::var_os("TALIS_WAYLAND_GL_RETRY").is_some() {
        assert_eq!(
            std::env::var("GDK_DISABLE").as_deref(),
            Ok("vulkan,gles-api")
        );
        println!("DESKTOP_GL_ATTEMPT");
        if std::env::var("TALIS_WAYLAND_RETRY_TEST_BOTH_FAIL").as_deref() == Ok("1") {
            let error =
                recover_with_version(context_error(DisplayBackend::Wayland), (4, 22)).unwrap_err();
            assert!(error.to_string().contains("no further backend retries"));
            println!("FINAL_WAYLAND_FAILURE");
            std::process::exit(1);
        }
        println!("WAYLAND_RECOVERED");
    } else {
        assert_eq!(std::env::var("GDK_DISABLE").as_deref(), Ok("vulkan"));
        println!("FIRST_WAYLAND_ATTEMPT");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        recover_with_version(context_error(DisplayBackend::Wayland), (4, 22)).unwrap();
        panic!("successful exec must not return");
    }
}

#[test]
#[ignore = "Subprocess helper; invoked by the graphics recovery test"]
fn recovery_child() {
    if std::env::var("TALIS_RECOVERY_TEST_CHILD").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::path::PathBuf::from(std::env::var_os("TALIS_RECOVERY_TEST_CWD").unwrap())
    );
    assert!(std::env::args_os().any(|arg| arg == OsStr::new("a 'quoted' argument ü")));
    if std::env::var("TALIS_GL_BACKEND").as_deref() == Ok("glx") {
        println!("GLX_ATTEMPT");
        if std::env::var("TALIS_RECOVERY_TEST_BOTH_FAIL").as_deref() == Ok("1") {
            let error = recover(context_error(DisplayBackend::X11)).unwrap_err();
            assert!(error.to_string().contains("no further backend retries"));
            println!("FINAL_FAILURE");
            std::process::exit(1);
        }
        println!("RECOVERED");
    } else {
        println!("FIRST_ATTEMPT");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        recover(context_error(DisplayBackend::X11)).unwrap();
        panic!("successful exec must not return");
    }
}
