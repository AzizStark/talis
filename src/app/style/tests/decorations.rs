use gtk::prelude::*;

#[test]
#[ignore = "Requires a private GTK display; verifies native title-bar hover pixels"]
fn content_styles_leave_native_close_button_hover_unchanged() {
    gtk::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    let display = gtk::gdk::Display::default().unwrap();
    let theme = gtk::CssProvider::new();
    // A light desktop theme must retain its own hover background and icon color.
    theme.load_from_data(
        "* { transition: none; }
         headerbar button:hover { background: #c6d4e2; color: #1c2630; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &theme,
        gtk::STYLE_PROVIDER_PRIORITY_THEME,
    );
    let window = gtk::Window::new();
    window.set_default_size(400, 180);
    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    let controls = gtk::WindowControls::new(gtk::PackType::End);
    controls.set_decoration_layout(Some(":minimize,maximize,close"));
    header.pack_end(&controls);
    window.set_titlebar(Some(&header));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&gtk::Label::new(Some("Settings content")));
    window.set_child(Some(&body));
    window.present();

    fn close_button(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is::<gtk::Button>() && widget.has_css_class("close") {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(button) = close_button(&current) {
                return Some(button);
            }
            child = current.next_sibling();
        }
        None
    }
    let close = close_button(controls.upcast_ref()).expect("native close button");
    close.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    let pixels = || {
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let paintable = gtk::WidgetPaintable::new(Some(&close));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, close.width() as f64, close.height() as f64);
        let texture = window
            .renderer()
            .unwrap()
            .render_texture(snapshot.to_node().unwrap(), None);
        let stride = texture.width() as usize * 4;
        let mut pixels = vec![0; stride * texture.height() as usize];
        texture.download(&mut pixels, stride);
        (texture.width(), texture.height(), pixels)
    };
    let native = pixels();
    super::super::install(&display);
    // Reproduce the original window-wide selector leak before checking the fix.
    window.add_css_class("talis-controls");
    assert_ne!(
        pixels(),
        native,
        "window-scoped hover must reproduce the bug"
    );
    window.remove_css_class("talis-controls");
    body.add_css_class("talis-controls");
    assert!(
        pixels() == native,
        "native hover pixels must stay unchanged"
    );
    window.destroy();
}
