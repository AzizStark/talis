#[path = "scroll.rs"]
mod scroll_tests;
#[test]
#[ignore = "Requires a GTK display and SVG icon loader"]
fn app_icon_is_available_to_native_windows() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    let display = gtk::gdk::Display::default().unwrap();
    super::install_app_icon(&display);
    assert!(gtk::IconTheme::for_display(&display).has_icon(super::APP_ICON_NAME));
    let window = gtk::Window::new();
    super::bind_window_icon(&window);
    window.set_icon_name(Some(super::APP_ICON_NAME));
    assert_eq!(window.icon_name().as_deref(), Some(super::APP_ICON_NAME));
    window.present();
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    if let Some(display_name) = std::env::var_os("TALIS_SETTINGS_TEST_DISPLAY") {
        assert_eq!(display_name, "1");
        assert_ne!(std::env::var("DISPLAY").as_deref(), Ok(":0"));
        // Inspect the real X server property, not just GTK's icon-name.
        let gtk = unsafe { libloading::Library::new("libgtk-4.so.1") }.unwrap();
        let xid = unsafe {
            let get_xid = gtk
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_ulong>(
                    b"gdk_x11_surface_get_xid\0",
                )
                .unwrap();
            get_xid(window.surface().unwrap().as_ptr().cast())
        };
        let result = std::process::Command::new("xprop")
            .args(["-id", &xid.to_string(), "_NET_WM_ICON"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&result.stdout);
        assert!(
            result.status.success() && text.contains("Icon (16 x 16)"),
            "{text}"
        );
        assert!(text.contains("Icon (128 x 128)"), "{text}");
    }
    window.destroy();
}
#[test]
#[ignore = "Requires a GTK display and its SVG loader"]
fn embedded_navigation_svgs_render_at_monitor_scales() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    for (page, svg) in super::NAVIGATION_ICONS {
        for scale in [1, 2] {
            let texture = super::icon_texture(svg, scale, super::NAVIGATION_ICON_SIZE, "#b8c3c6")
                .expect(page);
            let size = super::NAVIGATION_ICON_SIZE * scale;
            assert_eq!((texture.width(), texture.height()), (size, size));
            let mut pixels = vec![0u8; (size * size * 4) as usize];
            texture.download(&mut pixels, size as usize * 4);
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel[3] != 0),
                "{page}: visible strokes"
            );
            assert!(
                pixels.chunks_exact(4).any(|pixel| pixel[3] == 0),
                "{page}: transparent background"
            );
        }
        let image = super::navigation_icon(page);
        assert!(image.paintable().is_some());
        assert!(
            image.icon_name().is_none(),
            "must use embedded SVG, not theme lookup"
        );
    }
}

#[test]
fn bundled_logo_is_a_transparent_monochrome_png() {
    let pixels = image::load_from_memory(super::LOGO_PNG)
        .unwrap()
        .into_rgba8();
    assert_eq!(pixels.dimensions(), (278, 200));
    assert!(pixels.pixels().any(|pixel| pixel.0[3] == 0));
    assert!(pixels.pixels().any(|pixel| pixel.0[3] == 255));
    assert!(pixels.pixels().all(|pixel| pixel.0[..3] == [0, 0, 0]));
}

#[test]
#[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
#[allow(deprecated)]
fn compact_wordmark_and_numeric_foreground_survive_light_theme() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    let display = gtk::gdk::Display::default().unwrap();
    // Reproduce a light theme assigning black foregrounds to child nodes.
    let theme = gtk::CssProvider::new();
    theme.load_from_data(
        "spinbutton text, spinbutton button image, combobox button label { color: #101010; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &theme,
        gtk::STYLE_PROVIDER_PRIORITY_THEME,
    );
    super::install(&display);
    let window = gtk::Window::new();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("talis-controls");
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let logo = super::logo();
    brand.append(&logo);
    root.append(&brand);
    let spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    spin.add_css_class("setting-control");
    root.append(&spin);
    window.set_child(Some(&root));
    window.set_default_size(400, 180);
    window.present();
    let context = gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(logo.measure(gtk::Orientation::Horizontal, -1).1, 54);
    assert_eq!((logo.width(), logo.height()), (54, 39));
    let mut text = spin.first_child().unwrap();
    while text.css_name() != "text" {
        text = text.next_sibling().expect("spinbutton text node");
    }
    let color = text.style_context().color();
    assert!(
        color.red() > 0.8 && color.green() > 0.8 && color.blue() > 0.8,
        "{color}"
    );
    spin.set_sensitive(false);
    let color = text.style_context().color();
    assert!(
        color.red() > 0.5 && color.green() > 0.5 && color.blue() > 0.5,
        "{color}"
    );
    window.destroy();
}

#[test]
#[ignore = "Requires private Xvfb/WM; TALIS_SETTINGS_TEST_DISPLAY=1"]
#[allow(deprecated)]
fn settings_foregrounds_survive_real_focus_loss() {
    use gtk::prelude::*;
    assert_eq!(
        std::env::var("TALIS_SETTINGS_TEST_DISPLAY").as_deref(),
        Ok("1")
    );
    gtk::init().unwrap();
    let display = gtk::gdk::Display::default().unwrap();
    let theme = gtk::CssProvider::new();
    theme.load_from_data("label:backdrop, spinbutton text:backdrop { color: #101010; }");
    gtk::style_context_add_provider_for_display(
        &display,
        &theme,
        gtk::STYLE_PROVIDER_PRIORITY_THEME,
    );
    super::install(&display);
    let settings = gtk::Window::new();
    settings.add_css_class("talis-controls");
    settings.set_default_size(400, 400);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let mut labels = Vec::new();
    for class in [
        "page-title",
        "setting-title",
        "section-title",
        "setting-detail",
        "quiet-status",
    ] {
        let label = super::label(class, class);
        body.append(&label);
        labels.push(label);
    }
    let nav = gtk::ListBox::new();
    nav.add_css_class("settings-navigation");
    let row = gtk::ListBoxRow::new();
    let nav_label = gtk::Label::new(Some("General"));
    row.set_child(Some(&nav_label));
    nav.append(&row);
    nav.select_row(Some(&row));
    body.append(&nav);
    labels.push(nav_label);
    let quit = gtk::Button::with_label("Quit Talis");
    quit.add_css_class("subtle-button");
    labels.push(quit.child().unwrap().downcast::<gtk::Label>().unwrap());
    body.append(&quit);
    settings.set_child(Some(&body));
    let other = gtk::Window::new();
    other.set_default_size(160, 120);
    other.set_title(Some("Owned focus test"));
    fn wait(window: &gtk::Window, active: bool) {
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if window.is_active() == active && window.is_mapped() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "private WM did not set expected focus"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    settings.present();
    wait(&settings, true);
    let active: Vec<_> = labels
        .iter()
        .map(|label| label.style_context().color())
        .collect();
    for _ in 0..2 {
        other.present();
        wait(&other, true);
        wait(&settings, false);
        assert!(settings.state_flags().contains(gtk::StateFlags::BACKDROP));
        for (label, expected) in labels.iter().zip(&active) {
            assert_eq!(
                label.style_context().color(),
                *expected,
                "{} foreground changed on focus loss",
                label.text()
            );
        }
        settings.present();
        wait(&settings, true);
    }
    if let Some(path) = std::env::var_os("TALIS_SETTINGS_BACKDROP_CAPTURE") {
        other.present();
        wait(&other, true);
        wait(&settings, false);
        let paintable = gtk::WidgetPaintable::new(Some(&settings));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, settings.width() as f64, settings.height() as f64);
        let node = snapshot.to_node().unwrap();
        settings
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(path)
            .unwrap();
    }
    other.destroy();
    settings.destroy();
}

#[test]
#[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
fn controls_css_parses_without_errors() {
    gtk::init().unwrap();
    let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let collected = errors.clone();
    let provider = gtk::CssProvider::new();
    provider
        .connect_parsing_error(move |_, _, error| collected.borrow_mut().push(error.to_string()));
    provider.load_from_data(include_str!("../../controls.css"));
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
}
