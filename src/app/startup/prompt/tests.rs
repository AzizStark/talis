use super::super::style;
use super::*;

#[test]
fn only_supported_interactive_launches_are_candidates_for_the_prompt() {
    let normal = Options {
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
    };
    assert!(eligible(&normal));
    let mut plasma = normal.clone();
    plasma.experimental_wayland = true;
    assert!(eligible(&plasma)); // Backend support is checked before attaching.
    for excluded in [
        Options {
            desktop: false,
            ..normal.clone()
        },
        Options {
            shell_surface: true,
            ..normal.clone()
        },
        Options {
            scene_only: true,
            ..normal.clone()
        },
        Options {
            start_minimized: true,
            ..normal.clone()
        },
        Options {
            benchmark: true,
            ..normal.clone()
        },
        Options {
            gpu_profile: true,
            ..normal.clone()
        },
        Options {
            arm_demo: true,
            ..normal.clone()
        },
        Options {
            smoke_seconds: Some(3),
            ..normal.clone()
        },
        Options {
            capture: Some("test.png".into()),
            ..normal
        },
    ] {
        assert!(!eligible(&excluded));
    }
}

#[test]
#[ignore = "Requires an isolated GTK display; writes only to a temporary config directory"]
fn first_run_dialog_is_mapped_once_with_settings_and_released_after_response() {
    gtk::init().unwrap();
    let fixture = crate::autostart::Fixture::new();
    let manager = fixture.manager();
    let body = super::super::build(Ok(manager.clone()), || anyhow::bail!("Unused launch"), true);
    let parent = gtk::Window::new();
    parent.set_child(Some(&body));
    assert!(manager.prompt_pending().unwrap());
    let context = glib::MainContext::default();
    parent.present();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let prompt = loop {
        context.iteration(false);
        if let Some(prompt) = gtk::Window::list_toplevels()
            .into_iter()
            .find(|window| window.widget_name() == "startup-first-run-prompt")
        {
            break prompt.downcast::<gtk::MessageDialog>().unwrap();
        }
        assert!(std::time::Instant::now() < until);
    };
    assert_eq!(prompt.transient_for().as_ref(), Some(&parent));
    assert_eq!(
        prompt.text().as_deref(),
        Some("Do you want to start Talis when you sign in?")
    );
    assert_eq!(
        prompt.secondary_text().as_deref(),
        Some("You can change this later in settings.")
    );
    assert!(prompt.is_modal());
    assert!(!manager.enabled().unwrap());
    let weak = prompt.downgrade();
    prompt.response(gtk::ResponseType::Cancel);
    drop(prompt);
    assert!(weak.upgrade().is_none());
    assert!(!manager.prompt_pending().unwrap());
    parent.set_visible(false);
    parent.present();
    while context.pending() {
        context.iteration(false);
    }
    assert!(
        !gtk::Window::list_toplevels()
            .iter()
            .any(|window| window.widget_name() == "startup-first-run-prompt")
    );
    parent.destroy();
}

#[test]
#[ignore = "Requires an isolated GTK display; writes only to temporary config directories"]
fn startup_prompt_requires_consent_and_remembers_decline() {
    gtk::init().unwrap();
    for response in [
        gtk::ResponseType::Cancel,
        gtk::ResponseType::DeleteEvent,
        gtk::ResponseType::Yes,
    ] {
        let fixture = crate::autostart::Fixture::new();
        let manager = fixture.manager();
        let launch = fixture.launch();
        let body = super::super::build(
            Ok(manager.clone()),
            move || {
                Ok(crate::autostart::Launch {
                    executable: launch.executable.clone(),
                    directory: launch.directory.clone(),
                    config: launch.config.clone(),
                    monitor: 0,
                    following: false,
                    player: None,
                    graphics_environment: vec![],
                    wayland_library: None,
                })
            },
            false,
        );
        let toggle = style::find_widget(&body.clone().upcast(), "run-on-startup")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        let parent = gtk::Window::new();
        parent.set_child(Some(&body));
        let prompt = dialog(&parent, &toggle, manager.clone());
        assert!(manager.prompt_pending().unwrap());
        assert!(!manager.enabled().unwrap());
        assert!(!fixture.root.join("autostart").exists());
        prompt.response(response);
        assert_eq!(
            manager.enabled().unwrap(),
            response == gtk::ResponseType::Yes
        );
        assert!(!manager.prompt_pending().unwrap());
        parent.destroy();
    }
    let fixture = crate::autostart::Fixture::new();
    let manager = fixture.manager();
    let body = super::super::build(
        Ok(manager.clone()),
        || anyhow::bail!("Test launch failure"),
        false,
    );
    let toggle = style::find_widget(&body.clone().upcast(), "run-on-startup")
        .unwrap()
        .downcast::<gtk::Switch>()
        .unwrap();
    let parent = gtk::Window::new();
    parent.set_child(Some(&body));
    dialog(&parent, &toggle, manager.clone()).response(gtk::ResponseType::Yes);
    assert!(!manager.enabled().unwrap());
    assert!(manager.prompt_pending().unwrap());
    parent.destroy();
}
