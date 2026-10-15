use super::*;

fn context_error(x11: bool) -> anyhow::Error {
    ContextInitializationError::new("injected context creation failure".into(), x11).into()
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
fn recovery_is_only_for_automatic_native_x11_context_creation() {
    let error = context_error(true).context("scene startup");
    assert!(eligible(&error, "auto", false, "gl,gl-shaders", "vulkan"));
    assert!(!eligible(&error, "glx", false, "", ""));
    assert!(!eligible(&error, "auto", true, "", ""));
    assert!(!eligible(&context_error(false), "auto", false, "", ""));
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
    let error = context_error(true);
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
            let error = recover(context_error(true)).unwrap_err();
            assert!(error.to_string().contains("no further backend retries"));
            println!("FINAL_FAILURE");
            std::process::exit(1);
        }
        println!("RECOVERED");
    } else {
        println!("FIRST_ATTEMPT");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        recover(context_error(true)).unwrap();
        panic!("successful exec must not return");
    }
}
