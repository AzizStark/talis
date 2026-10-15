use super::*;
#[test]
#[ignore = "Opt-in native WM probe, empty input only; TALIS_DESKTOP_LAYER_PROBE=1"]
fn transparent_receiver_is_below_apps_above_desktop_without_focus() {
    assert_eq!(
        std::env::var("TALIS_DESKTOP_LAYER_PROBE").as_deref(),
        Ok("1")
    );
    gtk::init().unwrap();
    fn property(args: &[&str]) -> String {
        let output = std::process::Command::new("xprop")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }
    fn ids(value: &str) -> Vec<c_ulong> {
        value
            .split("0x")
            .skip(1)
            .filter_map(|part| {
                c_ulong::from_str_radix(part.split(|c: char| !c.is_ascii_hexdigit()).next()?, 16)
                    .ok()
            })
            .collect()
    }
    let active_before = property(&["-root", "_NET_ACTIVE_WINDOW"]);
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.LayerProbe")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let window = gtk::ApplicationWindow::builder()
        .application(&app)
        .title("Talis transparent layer probe")
        .default_width(8)
        .default_height(8)
        .build();
    let host = crate::desktop::DesktopHost::new(0).unwrap();
    window.set_child(Some(&host.input_widget()));
    let placement = host.configure_input(&window).unwrap();
    let xid = window_id(&window).unwrap() as c_ulong;
    // configure() leaves input empty; no pointer events or MPRIS calls.
    window.set_visible(true);
    let main_loop = gtk::glib::MainLoop::new(None, false);
    let end = main_loop.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || end.quit());
    main_loop.run();
    let confirmed = placement.can_receive(&window).unwrap();
    let nontransparent_pixels = placement.nontransparent_pixels(&window).unwrap();
    let active_after = property(&["-root", "_NET_ACTIVE_WINDOW"]);
    let stack = ids(&property(&["-root", "_NET_CLIENT_LIST_STACKING"]));
    let receiver_index = stack.iter().position(|id| *id == xid).unwrap();
    let active_index = ids(&active_before)
        .first()
        .and_then(|active| stack.iter().position(|id| id == active));
    let desktop_indices: Vec<_> = stack
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let hints = property(&["-id", &format!("0x{id:x}"), "_NET_WM_WINDOW_TYPE"]);
            hints
                .contains("_NET_WM_WINDOW_TYPE_DESKTOP")
                .then_some(index)
        })
        .collect();
    // Dispose before asserting so a failed diagnostic can't leave a window.
    window.destroy();
    assert_eq!(
        nontransparent_pixels, 0,
        "native input receiver painted opaque pixels"
    );
    assert!(confirmed, "WM did not acknowledge BELOW");
    assert_eq!(active_before, active_after, "receiver stole focus");
    assert!(
        !desktop_indices.is_empty(),
        "probe needs an existing desktop window"
    );
    assert!(
        desktop_indices.iter().all(|index| *index < receiver_index),
        "receiver below icons"
    );
    assert!(
        active_index.is_some_and(|index| index > receiver_index),
        "receiver above active app"
    );
}
#[test]
#[ignore = "Requires an isolated X11 display/WM; reserve a panel work area to test the regression"]
fn desktop_size_ignores_reserved_panel_work_area() {
    gtk::init().expect("isolated GTK display");
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.DesktopSizeTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let placement = DesktopPlacement::new(0).unwrap();
    let geometry = placement.monitor.geometry();
    let window = gtk::ApplicationWindow::builder().application(&app).build();
    window.set_child(Some(&gtk::Label::new(Some("Desktop sizing regression"))));
    placement.configure(&window).unwrap();
    window.set_visible(true);
    let main_loop = gtk::glib::MainLoop::new(None, false);
    let end = main_loop.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || end.quit());
    main_loop.run();
    assert_eq!(
        (window.width(), window.height()),
        (geometry.width(), geometry.height())
    );
    let surface = window.surface().unwrap();
    assert_eq!(
        (surface.width(), surface.height()),
        (geometry.width(), geometry.height())
    );
    assert!(!window.is_decorated());
    window.destroy();
}
#[test]
fn monitor_geometry_preserves_negative_offsets_and_hidpi() {
    assert_eq!(
        physical_geometry(&gdk::Rectangle::new(-1280, 0, 1280, 720), 2).unwrap(),
        (-2560, 0, 2560, 1440)
    );
    assert!(physical_geometry(&gdk::Rectangle::new(0, 0, 0, 720), 1).is_err());
    assert!(physical_geometry(&gdk::Rectangle::new(0, 0, 1280, 720), 0).is_err());
    assert!(physical_geometry(&gdk::Rectangle::new(i32::MAX, 0, 1280, 720), 2).is_err());
}
