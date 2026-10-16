use super::*;
use crate::playback::mpris::{TransportFixture, wait_for_transport};
use crate::{
    config::{Config, PreviewMode},
    preview::PreviewAsset,
    rendering::Renderer,
};
use std::{
    ffi::{c_int, c_ulong, c_void},
    time::Duration,
};

fn window_properties(window: &gtk::ApplicationWindow) -> String {
    let xid = unsafe {
        let library = libloading::Library::new("libgtk-4.so.1").unwrap();
        let get_xid = library
            .get::<unsafe extern "C" fn(*mut c_void) -> c_ulong>(b"gdk_x11_surface_get_xid\0")
            .unwrap();
        get_xid(window.surface().unwrap().as_ptr().cast())
    };
    let output = std::process::Command::new("xprop")
        .env("LC_ALL", "C")
        .args([
            "-id",
            &format!("0x{xid:x}"),
            "_NET_WM_WINDOW_TYPE",
            "_NET_WM_STATE",
            "_NET_WM_STRUT",
            "WM_HINTS",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    String::from_utf8(output.stdout).unwrap()
}

fn input_shape(window: &gtk::ApplicationWindow) -> Vec<[i32; 4]> {
    #[repr(C)]
    struct XRectangle {
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    }
    unsafe {
        let gtk = libloading::Library::new("libgtk-4.so.1").unwrap();
        let xlib = libloading::Library::new("libX11.so.6").unwrap();
        let shape = libloading::Library::new("libXext.so.6").unwrap();
        let get_display = gtk
            .get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                b"gdk_x11_display_get_xdisplay\0",
            )
            .unwrap();
        let get_xid = gtk
            .get::<unsafe extern "C" fn(*mut c_void) -> c_ulong>(b"gdk_x11_surface_get_xid\0")
            .unwrap();
        let sync = xlib
            .get::<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>(b"XSync\0")
            .unwrap();
        let free = xlib
            .get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"XFree\0")
            .unwrap();
        let rectangles = shape
            .get::<unsafe extern "C" fn(
                *mut c_void,
                c_ulong,
                c_int,
                *mut c_int,
                *mut c_int,
            ) -> *mut XRectangle>(b"XShapeGetRectangles\0")
            .unwrap();
        let surface = window.surface().unwrap();
        let display = get_display(surface.display().as_ptr().cast());
        sync(display, 0);
        let mut count = 0;
        let mut order = 0;
        let rects = rectangles(
            display,
            get_xid(surface.as_ptr().cast()),
            2,
            &mut count,
            &mut order,
        );
        let result = if count > 0 {
            std::slice::from_raw_parts(rects, count as usize)
                .iter()
                .map(|r| [r.x as i32, r.y as i32, r.w as i32, r.h as i32])
                .collect()
        } else {
            Vec::new()
        };
        if !rects.is_null() {
            free(rects.cast());
        }
        result
    }
}

/// Real X server pointer delivery, not manually emitted GTK signals. Virtual
/// display only: this helper must never send a click on the user's real desktop.
async fn pointer_click(window: &gtk::ApplicationWindow, x: f64, y: f64) {
    pointer_button(window, x, y, 1).await;
}
async fn pointer_button(window: &gtk::ApplicationWindow, x: f64, y: f64, number: u32) {
    pointer_button_at(window, Some((x, y)), number).await;
}
async fn stationary_wheel(window: &gtk::ApplicationWindow, number: u32) {
    assert!(matches!(number, 4 | 5));
    pointer_button_at(window, None, number).await;
}
async fn pointer_button_at(
    window: &gtk::ApplicationWindow,
    position: Option<(f64, f64)>,
    number: u32,
) {
    assert_ne!(std::env::var("DISPLAY").as_deref(), Ok(":0"));
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    unsafe {
        let gtk = libloading::Library::new("libgtk-4.so.1").unwrap();
        let xlib = libloading::Library::new("libX11.so.6").unwrap();
        let xtest = libloading::Library::new("libXtst.so.6").unwrap();
        let display = gtk
            .get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                b"gdk_x11_display_get_xdisplay\0",
            )
            .unwrap();
        let xid = gtk
            .get::<unsafe extern "C" fn(*mut c_void) -> c_ulong>(b"gdk_x11_surface_get_xid\0")
            .unwrap();
        let root = xlib
            .get::<unsafe extern "C" fn(*mut c_void) -> c_ulong>(b"XDefaultRootWindow\0")
            .unwrap();
        let translate = xlib
            .get::<unsafe extern "C" fn(
                *mut c_void,
                c_ulong,
                c_ulong,
                c_int,
                c_int,
                *mut c_int,
                *mut c_int,
                *mut c_ulong,
            ) -> c_int>(b"XTranslateCoordinates\0")
            .unwrap();
        let motion = xtest
            .get::<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_ulong) -> c_int>(
                b"XTestFakeMotionEvent\0",
            )
            .unwrap();
        let button = xtest
            .get::<unsafe extern "C" fn(*mut c_void, u32, c_int, c_ulong) -> c_int>(
                b"XTestFakeButtonEvent\0",
            )
            .unwrap();
        let flush = xlib
            .get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"XFlush\0")
            .unwrap();
        let surface = window.surface().unwrap();
        let d = display(surface.display().as_ptr().cast());
        let mut rx = 0;
        let mut ry = 0;
        let mut child = 0;
        assert_ne!(
            translate(
                d,
                xid(surface.as_ptr().cast()),
                root(d),
                0,
                0,
                &mut rx,
                &mut ry,
                &mut child
            ),
            0
        );
        if let Some((x, y)) = position {
            let scale = window.scale_factor() as f64;
            motion(
                d,
                -1,
                rx + (x * scale).round() as i32,
                ry + (y * scale).round() as i32,
                0,
            );
        }
        button(d, number, 1, 0);
        button(d, number, 0, 0);
        flush(d);
    }
    glib::timeout_future(Duration::from_millis(80)).await;
}
#[test]
#[ignore = "requires isolated Xvfb + window manager/private bus; TALIS_MPRIS_TEST_BUS=1"]
fn scene_gestures_and_desktop_input_shape_control_only_selected_player() {
    exercise_desktop_input(false);
}

#[test]
#[ignore = "requires isolated Xvfb + window manager/private bus; TALIS_MPRIS_TEST_BUS=1"]
fn visual_patch_does_not_intercept_desktop_controls_after_map_resize_or_remap() {
    exercise_desktop_input(true);
}

fn exercise_desktop_input(with_visual_patch: bool) {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    assert_ne!(std::env::var("DISPLAY").as_deref(), Ok(":0"));
    gtk::init().unwrap();
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_scene_buttons";
                let fixture = TransportFixture::new(name, "Paused", 20_000_000, 100_000_000).await;
                let mut config = Config::default();
                config.scene.mode = PreviewMode::Arm;
                if let Some(root) = std::env::var_os("TALIS_TRANSPORT_TEST_SCENE_ROOT") {
                    let root = std::path::PathBuf::from(root);
                    let package = crate::config::ScenePackage::at(&root);
                    if root.file_name().is_some_and(|n| n == "night") {
                        config.scene.variant = crate::config::SceneVariant::Night;
                        config.scene.night = Some(package);
                    } else {
                        config.scene.background = package.background;
                        config.scene.arm_poses = package.arm_poses;
                        config.scene.arm_flow = package.arm_flow;
                        config.scene.sleeve = package.sleeve;
                        config.scene.record_lighting = package.record_lighting;
                        config.scene.vinyl_detail = package.vinyl_detail;
                        config.scene.buttons = package.buttons;
                    }
                    config.render.arm_interpolation = true;
                }
                let app = gtk::Application::builder()
                    .application_id("io.github.azizstark.Talis.TransportTest")
                    .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                    .build();
                app.register(None::<&gtk::gio::Cancellable>).unwrap();
                let window = gtk::ApplicationWindow::builder().application(&app).build();
                let area = gtk::GLArea::builder()
                    .hexpand(true)
                    .vexpand(true)
                    .auto_render(false)
                    .build();
                area.set_required_version(3, 3);
                area.set_focusable(false);
                window.set_child(Some(&area));
                let host = crate::desktop::DesktopHost::new(0).unwrap();
                host.set_initial_mate_overlay(
                    std::env::var_os("TALIS_MATE_OVERLAY_TEST").is_some(),
                );
                host.configure_scene(&window).unwrap();
                window.set_visible(true);
                glib::timeout_future(Duration::from_millis(150)).await;
                area.make_current();
                area.attach_buffers();
                assert!(area.error().is_none(), "{:?}", area.error());
                let renderer = Rc::new(RefCell::new(Some(unsafe {
                    Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap()
                })));
                assert!(renderer.borrow().as_ref().unwrap().buttons_supported());
                area.connect_render({
                    let renderer = renderer.clone();
                    move |area, _| {
                        let scale = area.scale_factor();
                        if let Some(r) = renderer.borrow_mut().as_mut() {
                            unsafe {
                                r.render(area.width() * scale, area.height() * scale, 0.0)
                                    .unwrap();
                                r.check_error().unwrap();
                            }
                        }
                        glib::Propagation::Stop
                    }
                });
                area.queue_render();
                // Reproduce Nemo: a separate desktop-type window ABOVE the
                // beauty, accepting input even in visually transparent areas.
                let icons = gtk::ApplicationWindow::builder()
                    .application(&app)
                    .title("Talis test desktop icons")
                    .default_width(2560)
                    .default_height(1440)
                    .build();
                let icon_area = gtk::DrawingArea::new();
                icons.set_child(Some(&icon_area));
                let icon_placement = host.configure_desktop_fixture(&icons).unwrap();
                icons
                    .surface()
                    .unwrap()
                    .set_input_region(&gtk::cairo::Region::create_rectangle(
                        &gtk::cairo::RectangleInt::new(0, 0, 2560, 1440),
                    ));
                let icon_clicks = Rc::new(Cell::new(0));
                let icon_gesture = gtk::GestureClick::new();
                icon_gesture.connect_released({
                    let count = icon_clicks.clone();
                    move |_, _, _, _| count.set(count.get() + 1)
                });
                icon_area.add_controller(icon_gesture);
                icons.set_visible(true);
                icon_placement
                    .position(&icons, &window, [0, 0, 2560, 1440])
                    .unwrap();
                glib::timeout_future(Duration::from_millis(150)).await;
                let icon_hints = window_properties(&icons);
                assert!(icon_hints.contains("_NET_WM_WINDOW_TYPE_DESKTOP"));
                assert!(
                    !icon_hints.contains("_NET_WM_STATE_BELOW"),
                    "icon fixture must stay in the desktop layer: {icon_hints}"
                );
                let feed = Rc::new(RefCell::new(crate::playback::PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                feed.borrow_mut().select(Some(name.into()));
                let state = Rc::new(RefCell::new(AppState {
                    config,
                    previous_frame: Instant::now(),
                    mode: PreviewMode::Arm,
                    benchmark: false,
                    gpu_profile: false,
                    shell_surface: false,
                    arm: Default::default(),
                    arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
                    playback: feed.clone(),
                    artwork_rotation: Default::default(),
                    artwork_ready: false,
                    vinyl_ready: false,
                }));
                let slot = Rc::new(RefCell::new(None::<std::rc::Weak<TransportUi>>));
                let monitor = Monitor::start(feed.clone(), {
                    let slot = slot.clone();
                    move || {
                        if let Some(ui) = slot.borrow().as_ref().and_then(|w| w.upgrade()) {
                            ui.sync();
                        }
                    }
                });
                let ui = TransportUi::attach(
                    &area,
                    &window,
                    state.clone(),
                    renderer.clone(),
                    monitor.clone(),
                    Some(&host),
                )
                .unwrap();
                *slot.borrow_mut() = Some(Rc::downgrade(&ui));
                wait_for_transport(&feed, "scene controls ready", |f| {
                    f.command_target(Command::PlayPause, Instant::now())
                        .is_some()
                })
                .await;
                ui.sync();
                assert!(!window.is_focusable());
                assert!(
                    input_shape(&window).is_empty(),
                    "beauty scene must remain fully click-through"
                );
                let receiver = ui.receiver.as_ref().unwrap();
                let input_window = receiver.window.upgrade().unwrap();
                // Match native_patch::install: configure before adding content,
                // then map the visual surface above the scene. An empty client
                // input shape alone does not prove its WM frame is click-through.
                let visual_patch = with_visual_patch.then(|| {
                    let patch = gtk::ApplicationWindow::builder()
                        .application(&app)
                        .title("Talis test animated patch")
                        .build();
                    let placement = host.configure_animation(&patch).unwrap();
                    patch.set_child(Some(&host.input_widget()));
                    patch.set_default_size(2560, 1440);
                    patch.set_visible(true);
                    placement
                        .position(&patch, &window, [0, 0, 2560, 1440])
                        .unwrap();
                    placement.stack_above_scene(&patch, &window).unwrap();
                    (patch, placement)
                });
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(!input_window.is_focusable());
                assert_eq!(input_window.opacity(), 1.0);
                let shape = input_shape(&input_window);
                assert!(!shape.is_empty());
                let ellipses: Vec<_> = Command::ALL
                    .into_iter()
                    .map(|c| ui.ellipse(c).unwrap())
                    .collect();
                let scale = input_window.scale_factor() as f64;
                let [origin_x, origin_y] = receiver.origin.get();
                for [x, y, w, h] in shape {
                    for y in y..y + h {
                        for x in x..x + w {
                            // XShape rectangles are physical pixels, but the
                            // input mask is rasterized in GTK logical cells.
                            let logical_x = (x as f64 / scale).floor() + 0.5;
                            let logical_y = (y as f64 / scale).floor() + 0.5;
                            assert!(
                                ellipses.iter().any(|e| e.contains(
                                    logical_x + origin_x as f64,
                                    logical_y + origin_y as f64
                                )),
                                "input leaked outside buttons: {x},{y}"
                            );
                        }
                    }
                }
                let e = ui.ellipse(Command::PlayPause).unwrap();
                let centre = (e.x + e.w / 2.0, e.y + e.h / 2.0);
                ui.pointer.set(Some(centre));
                ui.sync();
                let event_widget = ui.event_widget().unwrap();
                assert!(event_widget.tooltip_text().is_none());
                assert!(
                    !event_widget.has_tooltip(),
                    "turntable hover must not show tooltips"
                );
                drop(event_widget);
                assert!(!ui.press(1, 0.5, 0.5));
                assert!(!ui.press(2, centre.0, centre.1));
                assert!(ui.press(1, centre.0, centre.1));
                glib::timeout_future(Duration::from_millis(180)).await;
                assert_eq!(ui.animations.borrow()[0].position(), 1.0);
                assert!(
                    !ui.animation_running.get(),
                    "holding must not keep a frame timer running"
                );
                let held_updates = renderer.borrow().as_ref().unwrap().button_update_count();
                glib::timeout_future(Duration::from_millis(80)).await;
                assert_eq!(
                    held_updates,
                    renderer.borrow().as_ref().unwrap().button_update_count()
                );
                ui.release(1, 0.0, 0.0);
                glib::timeout_future(Duration::from_millis(220)).await;
                assert_eq!(ui.animations.borrow()[0].position(), 0.0);
                assert!(!ui.animation_running.get());
                assert!(fixture.commands.borrow().is_empty());
                assert!(ui.press(1, centre.0, centre.1));
                state.borrow_mut().config.transport.enabled = false;
                ui.sync();
                assert!(!input_window.is_visible());
                ui.release(1, centre.0, centre.1);
                assert!(fixture.commands.borrow().is_empty());
                state.borrow_mut().config.transport.enabled = true;
                ui.sync();
                let controllers = ui.event_widget().unwrap().observe_controllers();
                let gesture = (0..controllers.n_items())
                    .find_map(|i| {
                        controllers
                            .item(i)
                            .and_then(|v| v.downcast::<gtk::GestureClick>().ok())
                    })
                    .unwrap();
                let event_centre = (
                    centre.0 - receiver.origin.get()[0] as f64,
                    centre.1 - receiver.origin.get()[1] as f64,
                );
                // A double-click's n_press=2 is still a primary-button event,
                // not mouse button 2. Exercise the real connected callbacks.
                assert_eq!(gesture.button(), 1);
                gesture.emit_by_name::<()>("pressed", &[&2_i32, &event_centre.0, &event_centre.1]);
                assert!(
                    fixture.commands.borrow().is_empty(),
                    "press sent a command before release"
                );
                gesture.emit_by_name::<()>("released", &[&2_i32, &event_centre.0, &event_centre.1]);
                assert!(feed.borrow().control_pending);
                assert!(input_shape(&input_window).is_empty());
                wait_for_transport(&feed, "scene click plays", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.status == crate::playback::Status::Playing)
                })
                .await;
                assert_eq!(*fixture.commands.borrow(), ["Play"]);
                glib::timeout_future(Duration::from_millis(500)).await;
                assert_eq!(ui.animations.borrow()[0].position(), 0.0);
                assert!(
                    !ui.animation_running.get(),
                    "quick click did not return to idle"
                );
                let local_centre = (
                    centre.0 - receiver.origin.get()[0] as f64,
                    centre.1 - receiver.origin.get()[1] as f64,
                );
                assert!(ui.event_widget().unwrap().width() > 0);
                assert!(ui.event_widget().unwrap().height() > 0);
                assert!(
                    input_window
                        .pick(local_centre.0, local_centre.1, gtk::PickFlags::DEFAULT)
                        .is_some()
                );
                assert_eq!(
                    receiver
                        .placement
                        .nontransparent_pixels(&input_window)
                        .unwrap(),
                    0,
                    "input receiver contains nontransparent native pixels"
                );
                pointer_click(&input_window, local_centre.0, local_centre.1).await;
                wait_for_transport(
                    &feed,
                    "real pointer pauses through desktop icon layer",
                    |f| {
                        !f.control_pending
                            && f.transport
                                .as_ref()
                                .is_some_and(|t| t.status == crate::playback::Status::Paused)
                    },
                )
                .await;
                assert_eq!(*fixture.commands.borrow(), ["Play", "Pause"]);
                assert_eq!(icon_clicks.get(), 0, "desktop swallowed the button click");
                if let Some((patch, placement)) = &visual_patch {
                    assert!(input_shape(patch).is_empty(), "visual client accepts input");
                    patch.set_visible(false);
                    glib::timeout_future(Duration::from_millis(30)).await;
                    patch.set_default_size(2528, 1408);
                    patch.set_visible(true);
                    placement
                        .position(patch, &window, [16, 16, 2528, 1408])
                        .unwrap();
                    placement.stack_above_scene(patch, &window).unwrap();
                    glib::timeout_future(Duration::from_millis(150)).await;
                }
                // Stay clear of real desktop panels in the private Xfce/MATE
                // sessions; a top-panel click is not a click-through failure.
                pointer_click(&icons, 80.0, 80.0).await;
                assert_eq!(icon_clicks.get(), 1, "outside button did not reach desktop");
                for command in [Command::Previous, Command::Next] {
                    let expected_count = fixture.commands.borrow().len() + 1;
                    let e = ui.ellipse(command).unwrap();
                    let (x, y) = (e.x + e.w / 2.0, e.y + e.h / 2.0);
                    pointer_click(
                        &input_window,
                        x - receiver.origin.get()[0] as f64,
                        y - receiver.origin.get()[1] as f64,
                    )
                    .await;
                    wait_for_transport(&feed, "skip completes", |f| {
                        fixture.commands.borrow().len() == expected_count
                            && !f.control_pending
                            && f.command_target(command, Instant::now()).is_some()
                    })
                    .await;
                }
                assert_eq!(
                    *fixture.commands.borrow(),
                    ["Play", "Pause", "Previous", "Next"]
                );
                assert_eq!(icon_clicks.get(), 1, "desktop raised over button receiver");
                // A regular app covering the scene must get the click instead.
                let normal = gtk::ApplicationWindow::builder()
                    .application(&app)
                    .title("Talis test normal application")
                    .decorated(false)
                    .default_width(240)
                    .default_height(100)
                    .build();
                let normal_area = gtk::DrawingArea::new();
                normal.set_child(Some(&normal_area));
                let normal_clicks = Rc::new(Cell::new(0));
                let normal_gesture = gtk::GestureClick::new();
                normal_gesture.connect_released({
                    let count = normal_clicks.clone();
                    move |_, _, _, _| count.set(count.get() + 1)
                });
                normal_area.add_controller(normal_gesture);
                normal.set_visible(true);
                glib::timeout_future(Duration::from_millis(150)).await;
                let [left, top] = receiver.origin.get();
                // Position only; do not change the normal app's type/state.
                icon_placement
                    .position(&normal, &window, [left - 10, top - 10, 240, 100])
                    .unwrap();
                glib::timeout_future(Duration::from_millis(150)).await;
                pointer_click(&input_window, local_centre.0, local_centre.1).await;
                let hints = window_properties(&input_window);
                assert!(hints.contains("_NET_WM_WINDOW_TYPE_DOCK"), "{hints}");
                assert!(hints.contains("_NET_WM_STATE_BELOW"), "{hints}");
                assert!(hints.contains("_NET_WM_STRUT:  not found"), "{hints}");
                assert!(
                    hints.contains("Client accepts input or input focus: False"),
                    "{hints}"
                );
                assert_eq!(
                    normal_clicks.get(),
                    1,
                    "receiver intercepted a normal application"
                );
                assert_eq!(
                    *fixture.commands.borrow(),
                    ["Play", "Pause", "Previous", "Next"]
                );
                normal.destroy();
                glib::timeout_future(Duration::from_millis(150)).await;
                fixture.volume_changed(0.5);
                wait_for_transport(&feed, "knob volume ready", |f| {
                    f.volume_target(Instant::now()).is_some()
                })
                .await;
                ui.sync();
                let knob = ui.volume_ellipse().unwrap();
                let knob_centre = (knob.x + knob.w / 2.0, knob.y + knob.h / 2.0);
                let knob_local = (
                    knob_centre.0 - receiver.origin.get()[0] as f64,
                    knob_centre.1 - receiver.origin.get()[1] as f64,
                );
                let counts = renderer.borrow().as_ref().unwrap().cache_update_counts();
                let button_updates = renderer.borrow().as_ref().unwrap().button_update_count();
                assert!(renderer.borrow().as_ref().unwrap().knob_supported());
                let knob_updates = renderer.borrow().as_ref().unwrap().knob_update_count();
                fixture.volume_apply_delay.set(350);
                let available_shape = input_shape(&input_window);
                pointer_button(&input_window, knob_local.0, knob_local.1, 4).await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(
                    feed.borrow().volume_target(Instant::now()).is_some(),
                    "delayed volume publication removed knob availability"
                );
                assert_eq!(input_shape(&input_window), available_shape);
                assert!(ui.pointer.get().is_some());
                assert_eq!(
                    ui.event_widget()
                        .unwrap()
                        .cursor()
                        .unwrap()
                        .name()
                        .as_deref(),
                    Some("pointer"),
                    "delayed volume publication lost the hover cursor"
                );
                wait_for_transport(&feed, "real wheel increases volume", |f| {
                    f.volume
                        .as_ref()
                        .is_some_and(|v| (v.value - 0.55).abs() < 1e-8)
                })
                .await;
                assert!(
                    ui.pointer.get().is_some(),
                    "wheel must retain knob hover without pointer motion"
                );
                stationary_wheel(&input_window, 5).await;
                wait_for_transport(&feed, "real wheel decreases volume", |f| {
                    f.volume
                        .as_ref()
                        .is_some_and(|v| (v.value - 0.5).abs() < 1e-8)
                })
                .await;
                for (number, expected) in [(4, 0.55), (5, 0.5), (4, 0.55), (5, 0.5)] {
                    stationary_wheel(&input_window, number).await;
                    wait_for_transport(&feed, "stationary wheel remains usable", |f| {
                        f.volume
                            .as_ref()
                            .is_some_and(|v| (v.value - expected).abs() < 1e-8)
                    })
                    .await;
                    assert!(
                        ui.pointer.get().is_some(),
                        "stationary wheel lost knob hover"
                    );
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while ui.animation_running.get() {
                    assert!(Instant::now() < deadline, "knob animation did not settle");
                    glib::timeout_future(Duration::from_millis(20)).await;
                }
                assert_eq!(fixture.volume_writes.borrow().len(), 6);
                assert!(
                    renderer.borrow().as_ref().unwrap().knob_update_count() > knob_updates,
                    "wheel volume never moved the marker"
                );
                assert_eq!(
                    *fixture.commands.borrow(),
                    ["Play", "Pause", "Previous", "Next"]
                );
                assert_eq!(
                    renderer.borrow().as_ref().unwrap().cache_update_counts(),
                    counts,
                    "marker animation repainted the static/arm/sleeve scene"
                );
                assert_eq!(
                    renderer.borrow().as_ref().unwrap().button_update_count(),
                    button_updates
                );
                assert!(!ui.animation_running.get());
                assert!(ui.event_widget().unwrap().tooltip_text().is_none());
                assert!(!ui.event_widget().unwrap().has_tooltip());
                assert!(
                    feed.borrow()
                        .control_notice
                        .as_deref()
                        .is_none_or(|s| !s.contains("Volume"))
                );
                ui.pointer.set(Some(centre));
                assert!(
                    !ui.scroll(-1.0),
                    "scroll on a playback button adjusted volume"
                );
                assert!(
                    !ui.press(1, knob_centre.0, knob_centre.1),
                    "knob click sent a playback command"
                );
                fixture
                    .data
                    .borrow_mut()
                    .insert("CanControl".into(), false.to_variant());
                fixture.changed();
                wait_for_transport(&feed, "unsupported desktop controls pass through", |f| {
                    f.transport
                        .as_ref()
                        .is_some_and(|t| !t.capabilities.control)
                })
                .await;
                assert!(input_shape(&input_window).is_empty());
                assert!(!ui.press(1, centre.0, centre.1));
                fixture
                    .data
                    .borrow_mut()
                    .insert("CanControl".into(), true.to_variant());
                fixture.changed();
                wait_for_transport(&feed, "controls recover", |f| {
                    f.command_target(Command::PlayPause, Instant::now())
                        .is_some()
                })
                .await;
                assert!(!input_shape(&input_window).is_empty());
                // Restoring a region under a stationary pointer generates an
                // enter, not necessarily motion. Hover and wheel must recover.
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(
                    ui.pointer.get().is_some_and(|(x, y)| knob.contains(x, y)),
                    "restored knob region must recover hover without pointer motion"
                );
                stationary_wheel(&input_window, 4).await;
                wait_for_transport(&feed, "restored knob accepts stationary wheel", |f| {
                    f.volume
                        .as_ref()
                        .is_some_and(|v| (v.value - 0.55).abs() < 1e-8)
                })
                .await;
                feed.borrow_mut().set_enabled(false);
                // No direct ui.sync(): monitor teardown must notify the
                // desktop adapter even though ordinary publish is disabled.
                glib::timeout_future(Duration::from_millis(350)).await;
                assert!(!input_window.is_visible());
                assert!(!ui.press(1, centre.0, centre.1));
                feed.borrow_mut().set_enabled(true);
                wait_for_transport(&feed, "following re-enables buttons", |f| {
                    f.command_target(Command::PlayPause, Instant::now())
                        .is_some()
                })
                .await;
                assert!(!input_shape(&input_window).is_empty());
                // Clear survives a hidden/remapped receiver and a changed
                // native buffer size, not only the first paint.
                glib::timeout_future(Duration::from_millis(100)).await;
                assert_eq!(
                    receiver
                        .placement
                        .nontransparent_pixels(&input_window)
                        .unwrap(),
                    0
                );
                let old_width = input_window.width();
                state.borrow_mut().config.transport.next[0] += 0.001;
                state.borrow_mut().config.transport.volume[0] += 0.001;
                ui.sync();
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(input_window.width() > old_width);
                assert_eq!(
                    receiver
                        .placement
                        .nontransparent_pixels(&input_window)
                        .unwrap(),
                    0
                );
                // Also exercise the real desktop manager in full-component
                // sessions, rather than only our simulated icon window. In a
                // bare-WM fixture these outside clicks naturally reach root.
                icons.set_visible(false);
                glib::timeout_future(Duration::from_millis(150)).await;
                let icon_clicks_before = icon_clicks.get();
                for command in [
                    Command::PlayPause,
                    Command::PlayPause,
                    Command::Previous,
                    Command::Next,
                ] {
                    // Caja places its Hard Drive icon near (80,80). Repeated
                    // clicks there open a normal window over the buttons,
                    // correctly intercepting input rather than testing desktop
                    // click-through. Use blank space away from icons/panels.
                    pointer_click(&window, 2200.0, 160.0).await;
                    assert_eq!(icon_clicks.get(), icon_clicks_before);
                    let expected_count = fixture.commands.borrow().len() + 1;
                    let ellipse = ui.ellipse(command).unwrap();
                    pointer_click(
                        &input_window,
                        ellipse.x + ellipse.w / 2.0 - receiver.origin.get()[0] as f64,
                        ellipse.y + ellipse.h / 2.0 - receiver.origin.get()[1] as f64,
                    )
                    .await;
                    wait_for_transport(&feed, "control after real desktop click", |f| {
                        fixture.commands.borrow().len() == expected_count
                            && !f.control_pending
                            && f.command_target(command, Instant::now()).is_some()
                    })
                    .await;
                }
                assert_eq!(
                    *fixture.commands.borrow(),
                    [
                        "Play", "Pause", "Previous", "Next", "Play", "Pause", "Previous", "Next"
                    ]
                );
                // Settings access does not depend on a live/controllable player.
                let settings = gtk::ApplicationWindow::builder().application(&app).build();
                state.borrow_mut().config.transport.enabled = false;
                feed.borrow_mut().set_enabled(false);
                ui.set_settings(Some(&settings), None);
                let logo = ui.logo_ellipse().unwrap();
                let (x, y) = (logo.x + logo.w / 2.0, logo.y + logo.h / 2.0);
                assert!(!ui.press(3, x, y));
                assert!(ui.press(1, x, y));
                ui.release(1, 0.0, 0.0);
                assert!(!settings.is_visible());
                assert!(ui.press(1, x, y));
                ui.cancel_capture();
                ui.release(1, x, y);
                assert!(!settings.is_visible());
                pointer_click(
                    &input_window,
                    x - receiver.origin.get()[0] as f64,
                    y - receiver.origin.get()[1] as f64,
                )
                .await;
                assert!(
                    settings.is_visible(),
                    "logo did not open Settings without a player"
                );
                assert_eq!(fixture.commands.borrow().len(), 8);
                settings.destroy();
                if let Some((patch, _)) = visual_patch {
                    patch.destroy();
                }
                monitor.stop();
                area.make_current();
                unsafe {
                    renderer.borrow_mut().take().unwrap().destroy();
                }
                let weak = Rc::downgrade(&ui);
                drop(gesture);
                drop(controllers);
                drop(ui);
                window.destroy();
                icons.destroy();
                input_window.destroy();
                drop(input_window);
                drop(area);
                drop(window);
                assert!(
                    weak.upgrade().is_none(),
                    "transport callbacks leaked scene widgets/controller"
                );
            })
        })
        .unwrap();
}
