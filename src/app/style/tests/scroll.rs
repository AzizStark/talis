//! Real pointer/key delivery on an explicitly authorized private Xvfb display.
use gtk::{glib, prelude::*};
use std::{
    ffi::{c_int, c_ulong, c_void},
    time::Duration,
};

#[test]
#[ignore = "Requires private Xvfb; TALIS_SETTINGS_TEST_DISPLAY=1"]
fn settings_value_wheel_scrolls_page_without_edits_but_buttons_and_keyboard_work() {
    assert_eq!(
        std::env::var("TALIS_SETTINGS_TEST_DISPLAY").as_deref(),
        Ok("1")
    );
    let display_name = std::env::var("DISPLAY").unwrap();
    assert!(display_name.starts_with(':') && display_name != ":0");
    gtk::init().unwrap();
    let context = glib::MainContext::default();
    context.block_on(async {
        let window = gtk::Window::new();
        window.set_decorated(false);
        window.set_default_size(520, 240);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let combo = gtk::ComboBoxText::new();
        for value in ["One", "Two", "Three"] {
            combo.append_text(value);
        }
        combo.set_active(Some(0));
        body.append(&combo);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_size_request(-1, 1200);
        body.append(&spacer);
        let page = gtk::ScrolledWindow::builder()
            .kinetic_scrolling(false)
            .child(&body)
            .build();
        window.set_child(Some(&page));
        window.present();
        glib::timeout_future(Duration::from_millis(200)).await;

        // Prove the installed GTK actually exhibits the original behavior.
        input(&window, &combo, 5, None);
        glib::timeout_future(Duration::from_millis(100)).await;
        assert_eq!(combo.active(), Some(1), "unprotected GTK scroll baseline");
        combo.set_active(Some(0));
        body.remove(&combo);
        body.prepend(&super::super::row("Test", "", &combo));
        let edits = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = edits.clone();
        combo.connect_changed(move |_| count.set(count.get() + 1));
        page.vadjustment().set_value(0.0);
        glib::timeout_future(Duration::from_millis(100)).await;
        // Repeat both with and without focus: neither should edit/autosave.
        for focused in [false, true] {
            page.vadjustment().set_value(0.0);
            if focused {
                combo.grab_focus();
            } else {
                gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            }
            glib::timeout_future(Duration::from_millis(80)).await;
            input(&window, &combo, 5, None);
            glib::timeout_future(Duration::from_millis(150)).await;
            assert_eq!(combo.active(), Some(0));
            assert_eq!(edits.get(), 0, "wheel must not trigger preference edits");
            assert!(
                page.vadjustment().value() > 0.0,
                "wheel must still scroll page"
            );
        }
        page.vadjustment().set_value(0.0);
        glib::timeout_future(Duration::from_millis(100)).await;
        input(&window, &combo, 1, None);
        glib::timeout_future(Duration::from_millis(100)).await;
        assert!(combo.is_popup_shown(), "click must open dropdown");
        combo.popdown();
        combo.grab_focus();
        glib::timeout_future(Duration::from_millis(100)).await;
        input(&window, &combo, 0, Some(0xff54)); // Down
        glib::timeout_future(Duration::from_millis(100)).await;
        assert_eq!(
            combo.active(),
            Some(1),
            "keyboard selection remains available"
        );
        assert_eq!(edits.get(), 1);

        // Numeric entries use the same guard; test the actual spinbutton path.
        body.remove(&body.first_child().unwrap());
        let spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
        spin.set_value(10.0);
        body.prepend(&spin);
        page.vadjustment().set_value(0.0);
        spin.grab_focus();
        glib::timeout_future(Duration::from_millis(100)).await;
        input(&window, &spin, 5, None);
        glib::timeout_future(Duration::from_millis(100)).await;
        assert_eq!(spin.value(), 9.0, "unprotected numeric scroll baseline");
        spin.set_value(10.0);
        body.remove(&spin);
        body.prepend(&super::super::row("Number", "", &spin));
        let edits = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = edits.clone();
        spin.connect_value_changed(move |_| count.set(count.get() + 1));
        for focused in [false, true] {
            page.vadjustment().set_value(0.0);
            if focused {
                spin.grab_focus();
            } else {
                gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            }
            glib::timeout_future(Duration::from_millis(80)).await;
            input(&window, &spin, 5, None);
            glib::timeout_future(Duration::from_millis(150)).await;
            assert_eq!(spin.value(), 10.0);
            assert_eq!(edits.get(), 0, "numeric wheel must not trigger edits");
            assert!(page.vadjustment().value() > 0.0);
        }
        page.vadjustment().set_value(0.0);
        spin.grab_focus();
        glib::timeout_future(Duration::from_millis(100)).await;
        input(&window, &spin, 0, Some(0xff54)); // Down
        glib::timeout_future(Duration::from_millis(100)).await;
        assert_eq!(spin.value(), 9.0, "numeric keyboard arrow");
        for (class, expected) in [("up", 10.0), ("down", 9.0)] {
            let mut child = spin.first_child().unwrap();
            while !child.has_css_class(class) {
                child = child
                    .next_sibling()
                    .expect("spin increment/decrement button");
            }
            input(&window, &child, 1, None);
            glib::timeout_future(Duration::from_millis(100)).await;
            assert_eq!(spin.value(), expected, "numeric {class} button");
        }
        spin.grab_focus();
        spin.select_region(0, -1);
        for key in [0x34, 0x32, 0xff0d] {
            // Type 42, then Return.
            input(&window, &spin, 0, Some(key));
            glib::timeout_future(Duration::from_millis(60)).await;
        }
        assert_eq!(spin.value(), 42.0, "numeric text entry");
        window.destroy();
    });
}

/// No commands or input are ever sent to the real desktop. The enclosing test
/// requires a private display, and uses the GTK connection for that display.
fn input(window: &gtk::Window, widget: &impl IsA<gtk::Widget>, button: u32, key: Option<c_ulong>) {
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
        let surface = window.surface().unwrap();
        let d = display(surface.display().as_ptr().cast());
        let (mut rx, mut ry, mut child) = (0, 0, 0);
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
        if let Some(key) = key {
            let code = xlib
                .get::<unsafe extern "C" fn(*mut c_void, c_ulong) -> u8>(b"XKeysymToKeycode\0")
                .unwrap()(d, key);
            let send = xtest
                .get::<unsafe extern "C" fn(*mut c_void, u32, c_int, c_ulong) -> c_int>(
                    b"XTestFakeKeyEvent\0",
                )
                .unwrap();
            send(d, code as u32, 1, 0);
            send(d, code as u32, 0, 0);
        } else {
            let point = widget
                .compute_point(
                    window,
                    &gtk::graphene::Point::new(
                        widget.width() as f32 / 2.0,
                        widget.height() as f32 / 2.0,
                    ),
                )
                .unwrap();
            let scale = window.scale_factor() as f32;
            xtest
                .get::<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_ulong) -> c_int>(
                    b"XTestFakeMotionEvent\0",
                )
                .unwrap()(
                d,
                -1,
                rx + (point.x() * scale).round() as i32,
                ry + (point.y() * scale).round() as i32,
                0,
            );
            let send = xtest
                .get::<unsafe extern "C" fn(*mut c_void, u32, c_int, c_ulong) -> c_int>(
                    b"XTestFakeButtonEvent\0",
                )
                .unwrap();
            send(d, button, 1, 0);
            send(d, button, 0, 0);
        }
        xlib.get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"XFlush\0")
            .unwrap()(d);
    }
}
