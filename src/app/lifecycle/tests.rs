use super::super::state::AppState;
use super::*;

#[test]
#[ignore = "Run individually under a private GTK display and session bus"]
fn actual_glarea_creation_failure_stops_startup_and_retains_typed_cause() {
    gtk::init().unwrap();
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.ContextFailureTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let failure: Failure = Rc::new(RefCell::new(None));
    let result = failure.clone();
    app.connect_activate(move |app| {
        let area = gtk::GLArea::new();
        area.set_required_version(3, 3);
        area.set_use_es(false);
        area.connect_create_context(|area| {
            area.set_error(Some(&glib::Error::new(
                gtk::gdk::GLError::NotAvailable,
                "injected context failure",
            )));
            None
        });
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .child(&area)
            .build();
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
        // No assets supplied: context failure must be identified before decoding.
        connect_realize(
            &area,
            Rc::new(RefCell::new(None)),
            state,
            Rc::new(RefCell::new(None)),
            result.clone(),
        );
        window.present();
    });
    app.connect_shutdown(|app| {
        for window in app.windows() {
            window.destroy();
        }
    });
    app.run_with_args(&["context-failure-test"]);
    let error = failure.borrow_mut().take().unwrap();
    assert!(error.is::<crate::graphics::ContextInitializationError>());
    assert!(error.to_string().contains("injected context failure"));
}
