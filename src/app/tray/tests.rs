//! Execute only on a private dbus-run-session bus; never the user's tray.
use super::*;
use crate::{
    arm::ArmState,
    config::{Config, PreviewMode},
    playback::PlaybackFeed,
};
use gtk::gio;
use std::time::{Duration, Instant};

fn test_state() -> Rc<RefCell<AppState>> {
    Rc::new(RefCell::new(AppState {
        config: Config {
            minimize_to_tray: false,
            ..Config::default()
        },
        previous_frame: Instant::now(),
        mode: PreviewMode::Beauty,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm: ArmState::default(),
        arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
        playback: Rc::new(RefCell::new(PlaybackFeed::default())),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    }))
}
#[track_caller]
fn wait_until(check: impl Fn() -> bool) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move {
        let until = Instant::now() + Duration::from_secs(3);
        while !check() {
            assert!(
                Instant::now() < until,
                "tray state did not settle at {caller}"
            );
            glib::timeout_future(Duration::from_millis(10)).await;
        }
    }
}
async fn call(
    connection: &gio::DBusConnection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: glib::Variant,
) -> Result<glib::Variant, glib::Error> {
    connection
        .call_future(
            Some(destination),
            path,
            interface,
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            2000,
        )
        .await
}

#[test]
fn tray_preference_defaults_on_and_preserves_explicit_opt_out() {
    let mut config: Config = toml::from_str("").unwrap();
    assert!(config.minimize_to_tray);
    config.minimize_to_tray = false;
    let copy: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert!(!copy.minimize_to_tray);
}

#[test]
#[ignore = "Requires GTK and TALIS_TRAY_TEST_BUS=1 inside dbus-run-session"]
fn tray_default_without_host_preserves_taskbar_and_restores_after_host_loss() {
    assert_eq!(std::env::var("TALIS_TRAY_TEST_BUS").as_deref(), Ok("1"));
    gtk::init().unwrap();
    glib::MainContext::default().block_on(async {
        let app = gtk::Application::builder()
            .application_id("io.github.azizstark.Talis.TrayDefaultTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = gtk::ApplicationWindow::builder()
            .application(&app)
            .title("Talis no tray test")
            .default_width(320)
            .default_height(180)
            .build();
        let state = test_state();
        state.borrow_mut().config.minimize_to_tray = Config::default().minimize_to_tray;
        let ui = TrayUi::new(&window, state);
        assert!(ui.toggle.is_active() && ui.toggle.is_sensitive());
        assert!(!ui.available.get() && !ui.hide());
        window.present();
        wait_until(|| window.is_mapped()).await;
        glib::timeout_future(Duration::from_millis(500)).await;
        window.close();
        wait_until(|| {
            window
                .surface()
                .and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok())
                .is_some_and(|s| s.state().contains(gtk::gdk::ToplevelState::MINIMIZED))
        })
        .await;
        assert!(
            window.is_visible() && !ui.hidden.get(),
            "no host must keep a taskbar window"
        );
        ui.set_available(true);
        assert!(ui.hidden.get() && !window.is_visible());
        ui.set_available(false);
        wait_until(|| window.is_mapped() && !ui.restoring.get()).await;
        assert!(window.is_visible() && !ui.hidden.get());
        ui.stop();
        window.destroy();
    });
}

#[test]
#[ignore = "Requires GTK and TALIS_TRAY_TEST_BUS=1 inside dbus-run-session"]
fn tray_lifecycle_and_protocol_on_private_bus() {
    assert_eq!(std::env::var("TALIS_TRAY_TEST_BUS").as_deref(), Ok("1"));
    gtk::init().unwrap();
    let context = glib::MainContext::default();
    context.block_on(async {
        let app = gtk::Application::builder().application_id("io.github.azizstark.Talis.TrayTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE).build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = gtk::ApplicationWindow::builder().application(&app).title("Talis tray test").default_width(320).default_height(180).build();
        let scene = gtk::ApplicationWindow::builder().application(&app).title("Talis scene test").default_width(320).default_height(180).build();
        window.present(); scene.present();
        let state = test_state();
        let ui = TrayUi::new(&window, state.clone());
        ui.start();
        wait_until(|| ui.backend.borrow().is_some()).await;
        assert!(!ui.available.get()); assert!(!ui.hide()); assert!(!ui.toggle.is_sensitive());
        let fixture = gio::DBusConnection::for_address_sync(&std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,None::<&gio::Cancellable>).unwrap();
        let host = Rc::new(Cell::new(false));
        let registrations = Rc::new(RefCell::new(Vec::<(String,String)>::new()));
        let watcher_xml = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.kde.StatusNotifierWatcher">
            <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
            <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
            <signal name="StatusNotifierHostRegistered"/><signal name="StatusNotifierHostUnregistered"/>
        </interface></node>"#).unwrap();
        let registration = fixture.register_object("/StatusNotifierWatcher",&watcher_xml.interfaces()[0])
            .property({ let host = host.clone(); move |_,_,_,_,_| host.get().to_variant() })
            .method_call({ let registrations = registrations.clone(); move |_,sender,_,_,_,args,invocation| {
                registrations.borrow_mut().push((sender.unwrap().into(),args.get::<(String,)>().unwrap().0));
                invocation.return_value(Some(&().to_variant()));
            }}).build().unwrap();
        call(&fixture,"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus","RequestName",
            ("org.kde.StatusNotifierWatcher",0_u32).to_variant()).await.unwrap();
        wait_until(|| !registrations.borrow().is_empty()).await;
        glib::timeout_future(Duration::from_millis(30)).await;
        assert!(!ui.available.get(), "a watcher without a panel must not hide settings");
        host.set(true);
        fixture.emit_signal(None,"/StatusNotifierWatcher","org.kde.StatusNotifierWatcher","StatusNotifierHostRegistered",None).unwrap();
        wait_until(|| ui.available.get()).await;
        assert!(ui.toggle.is_sensitive()); ui.toggle.set_active(true);
        assert!(state.borrow().config.minimize_to_tray);
        assert!(ui.hide()); assert!(!window.is_visible()); assert!(scene.is_visible());
        // The scene-logo route must restore the tray's bookkeeping too. A raw
        // window.present() leaves hidden=true and makes the next close a no-op.
        let logo_area = gtk::GLArea::new();
        let logo_feed = state.borrow().playback.clone();
        let logo_monitor = crate::playback::mpris::Monitor::start(logo_feed, || {});
        let logo_input = super::super::transport::TransportUi::attach(
            &logo_area, &window, state.clone(), Rc::new(RefCell::new(None)),
            logo_monitor.clone(), None).unwrap();
        logo_input.set_settings(Some(&window), Some(&ui));
        for _ in 0..2 {
            logo_input.open_settings();
            assert!(window.is_visible() && !ui.hidden.get());
            window.close();
            assert!(!window.is_visible() && ui.hidden.get(), "logo-opened Settings did not close to tray");
        }
        logo_monitor.stop();
        drop(logo_input);
        drop(logo_area);
        let destination = registrations.borrow()[0].0.clone();
        assert_eq!(registrations.borrow()[0].1, "/StatusNotifierItem");
        let props = call(&fixture,&destination,"/StatusNotifierItem","org.freedesktop.DBus.Properties","GetAll",
            ("org.kde.StatusNotifierItem",).to_variant()).await.unwrap().get::<(std::collections::HashMap<String,glib::Variant>,)>().unwrap().0;
        assert_eq!(props["Status"].get::<String>().unwrap(), "Active");
        assert_eq!(props["IconPixmap"].type_().as_str(), "a(iiay)");
        let icon_path = std::path::PathBuf::from(props["IconThemePath"].str().unwrap());
        if protocol::prefer_pixmaps(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) {
            assert_eq!(props["IconName"].str().unwrap(), "");
            assert_eq!(props["IconThemePath"].str().unwrap(), "");
            let pixels = props["IconPixmap"].get::<Vec<(i32, i32, Vec<u8>)>>().unwrap();
            assert_eq!(pixels.len(), 6);
            assert!(pixels.iter().all(|(w, h, p)| p.len() == (w * h * 4) as usize && p.chunks_exact(4).any(|p| p[0] > 0)));
        } else {
            assert_eq!(props["IconName"].str().unwrap(), "io.github.azizstark.TalisTray-symbolic");
            assert!(icon_path.join("io.github.azizstark.TalisTray-symbolic.svg").is_file());
            assert!(icon_path.join("hicolor/index.theme").is_file());
            assert!(icon_path.join("hicolor/scalable/status/io.github.azizstark.TalisTray-symbolic.svg").is_file());
        }
        assert_eq!(props["Menu"].str().unwrap(), "/TalisMenu");
        let layout = call(&fixture,&destination,"/TalisMenu","com.canonical.dbusmenu","GetLayout",
            (0_i32,-1_i32,Vec::<String>::new()).to_variant()).await.unwrap();
        assert_eq!(layout.type_().as_str(), "(u(ia{sv}av))");
        call(&fixture,&destination,"/StatusNotifierItem","org.kde.StatusNotifierItem","Activate",
            (0_i32,0_i32).to_variant()).await.unwrap();
        assert!(window.is_visible()); assert!(!ui.hidden.get());
        window.close(); assert!(!window.is_visible()); assert!(scene.is_visible());
        call(&fixture,&destination,"/TalisMenu","com.canonical.dbusmenu","Event",
            (1_i32,"clicked",0_i32.to_variant(),0_u32).to_variant()).await.unwrap();
        assert!(window.is_visible());
        assert!(ui.hide()); host.set(false);
        let changed = std::collections::HashMap::from([("IsStatusNotifierHostRegistered".to_string(),false.to_variant())]);
        fixture.emit_signal(None,"/StatusNotifierWatcher","org.freedesktop.DBus.Properties","PropertiesChanged",
            Some(&("org.kde.StatusNotifierWatcher",changed,Vec::<String>::new()).to_variant())).unwrap();
        wait_until(|| !ui.available.get()).await;
        assert!(window.is_visible()); assert!(!ui.hide()); assert!(ui.status.is_visible());
        host.set(true);
        fixture.emit_signal(None,"/StatusNotifierWatcher","org.kde.StatusNotifierWatcher","StatusNotifierHostRegistered",None).unwrap();
        wait_until(|| ui.available.get()).await;
        assert!(ui.hide()); ui.toggle.set_active(false);
        assert!(window.is_visible()); assert!(!ui.hide()); assert!(!state.borrow().config.minimize_to_tray);
        let minimized = gtk::ApplicationWindow::builder().application(&app)
            .title("Talis minimize test").default_width(320).default_height(180).build();
        let minimized_ui = TrayUi::new(&minimized, test_state());
        minimized_ui.set_available(true);
        minimized_ui.toggle.set_active(true);
        minimized.present();
        wait_until(|| minimized.is_mapped()).await;
        // Allow the WM to manage the fresh window before requesting minimization.
        glib::timeout_future(Duration::from_millis(500)).await;
        minimized.minimize();
        wait_until(|| minimized_ui.hidden.get()).await;
        assert!(!minimized.is_visible()); assert!(scene.is_visible());
        minimized_ui.show();
        wait_until(|| minimized.is_mapped() && !minimized_ui.restoring.get()).await;
        // Reproduce Cinnamon delivering old iconify evidence just after map.
        // Only the owned private-test drawable is modified, never the desktop.
        minimized_ui.hide();
        minimized_ui.show();
        assert!(minimized_ui.restoring.get());
        let gtk_library = unsafe { libloading::Library::new("libgtk-4.so.1") }.unwrap();
        let xid = unsafe {
            let get_xid = gtk_library.get::<unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_ulong>(b"gdk_x11_surface_get_xid\0").unwrap();
            get_xid(minimized.surface().unwrap().as_ptr().cast())
        };
        let set_state = |value: &str| {
            assert!(std::process::Command::new("xprop").args(["-id", &xid.to_string(), "-f", "_NET_WM_STATE", "32a", "-set", "_NET_WM_STATE", value]).status().unwrap().success());
        };
        let late_min_seen = Rc::new(Cell::new(false));
        minimized.surface().unwrap().downcast::<gtk::gdk::Toplevel>().unwrap().connect_state_notify({
            let seen = late_min_seen.clone();
            move |surface| { if surface.state().contains(gtk::gdk::ToplevelState::MINIMIZED) { seen.set(true); } }
        });
        set_state("_NET_WM_STATE_HIDDEN");
        glib::timeout_future(Duration::from_millis(50)).await;
        assert!(late_min_seen.get(), "fixture did not deliver its late minimize event");
        assert!(minimized.is_visible() && !minimized_ui.hidden.get(), "late iconify event re-hid restored Settings");
        set_state("_NET_WM_STATE_FOCUSED");
        wait_until(|| !minimized_ui.restoring.get()).await;
        // Also restore before the WM has acknowledged a new minimize request:
        // GTK's persisted initial-minimize flag must not be replayed on show.
        minimized.minimize();
        assert!(minimized_ui.hide());
        minimized_ui.show();
        glib::timeout_future(Duration::from_millis(500)).await;
        assert!(minimized.is_visible());
        assert!(!minimized_ui.hidden.get());
        assert!(!minimized.surface().unwrap().downcast::<gtk::gdk::Toplevel>().unwrap()
            .state().contains(gtk::gdk::ToplevelState::MINIMIZED));
        minimized.close();
        assert!(minimized_ui.hidden.get() && !minimized.is_visible());
        minimized_ui.stop(); minimized.destroy();
        ui.toggle.set_active(true); assert!(ui.hide());
        call(&fixture,"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus","ReleaseName",
            ("org.kde.StatusNotifierWatcher",).to_variant()).await.unwrap();
        wait_until(|| !ui.available.get()).await; assert!(window.is_visible());
        let count = registrations.borrow().len();
        call(&fixture,"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus","RequestName",
            ("org.kde.StatusNotifierWatcher",0_u32).to_variant()).await.unwrap();
        wait_until(|| ui.available.get() && registrations.borrow().len() > count).await;
        ui.stop(); assert!(ui.backend.borrow().is_none()); assert!(!ui.hide());
        if !icon_path.as_os_str().is_empty() {
            assert!(!icon_path.exists(), "normal tray shutdown must remove only its owned icon files");
        }
        let result = call(&fixture,&destination,"/TalisMenu","com.canonical.dbusmenu","GetLayout",
            (0_i32,-1_i32,Vec::<String>::new()).to_variant()).await;
        assert!(result.is_err(), "shutdown unexports the menu");
        fixture.unregister_object(registration).unwrap();
        fixture.close_future().await.unwrap(); window.destroy(); scene.destroy();
    });
}

#[test]
#[ignore = "Requires the owned temporary session-bus fixture; terminates only its own daemon"]
fn private_bus_tray_reconnect_and_shutdown() {
    use crate::session_bus::tests::{Bus, name, wait};
    let mut bus = Bus::new();
    gtk::init().unwrap();
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let window = gtk::ApplicationWindow::builder()
                    .title("Private bus tray recovery")
                    .build();
                window.present();
                let state = test_state();
                let ui = TrayUi::new(&window, state.clone());
                ui.toggle.set_active(true);
                ui.start();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert!(ui.backend.borrow().is_none());
                assert!(!ui.available.get());
                bus.start();
                let watch = |bus: &Bus| {
                    let connection = bus.fixture();
                    let info = gio::DBusNodeInfo::for_xml(
                        r#"<node><interface name="org.kde.StatusNotifierWatcher">
              <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
              <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
            </interface></node>"#,
                    )
                    .unwrap();
                    let registrations = Rc::new(RefCell::new(Vec::<String>::new()));
                    let registration = connection
                        .register_object("/StatusNotifierWatcher", &info.interfaces()[0])
                        .property(|_, _, _, _, _| true.to_variant())
                        .method_call({
                            let registrations = registrations.clone();
                            move |_, sender, _, _, _, _, invocation| {
                                registrations.borrow_mut().push(sender.unwrap().into());
                                invocation.return_value(Some(&().to_variant()));
                            }
                        })
                        .build()
                        .unwrap();
                    (connection, registration, registrations)
                };
                let (fixture, registration, registrations) = watch(&bus);
                name(&fixture, "RequestName", "org.kde.StatusNotifierWatcher").await;
                wait(|| ui.available.get()).await;
                let old = crate::session_bus::connect().await.unwrap();
                assert!(!old.property::<bool>("exit-on-close"));
                assert!(ui.hide());
                bus.stop();
                wait(|| !ui.available.get() && ui.backend.borrow().is_none()).await;
                assert!(
                    window.is_visible(),
                    "failed tray must not strand the settings window"
                );
                assert!(
                    state.borrow().config.minimize_to_tray,
                    "preserve the user preference"
                );
                assert!(old.is_closed());
                assert!(!registrations.borrow().is_empty());
                let _ = fixture.unregister_object(registration);
                bus.start();
                let (fixture, registration, registrations) = watch(&bus);
                name(&fixture, "RequestName", "org.kde.StatusNotifierWatcher").await;
                wait(|| ui.available.get()).await;
                let new = crate::session_bus::connect().await.unwrap();
                assert_ne!(new, old);
                let destination = registrations.borrow()[0].clone();
                // Recovery re-exports the objects, not just the availability flag.
                call(
                    &fixture,
                    &destination,
                    "/StatusNotifierItem",
                    "org.kde.StatusNotifierItem",
                    "Activate",
                    (0_i32, 0_i32).to_variant(),
                )
                .await
                .unwrap();
                assert!(ui.hide());
                ui.stop();
                assert!(ui.task.borrow().is_none());
                assert!(ui.backend.borrow().is_none());
                assert!(
                    !new.is_closed(),
                    "tray teardown must not close another observer's connection"
                );
                assert!(
                    call(
                        &fixture,
                        &destination,
                        "/TalisMenu",
                        "com.canonical.dbusmenu",
                        "GetLayout",
                        (0_i32, -1_i32, Vec::<String>::new()).to_variant()
                    )
                    .await
                    .is_err()
                );
                fixture.unregister_object(registration).unwrap();
                bus.stop();
                let stopped = TrayUi::new(&window, test_state());
                stopped.start();
                glib::timeout_future(Duration::from_millis(100)).await;
                stopped.stop();
                bus.start();
                glib::timeout_future(crate::session_bus::RETRY + Duration::from_millis(100)).await;
                assert!(
                    stopped.backend.borrow().is_none(),
                    "shutdown cancels pending retries"
                );
                window.destroy();
            })
        })
        .unwrap();
}
