use super::*;

#[test]
#[ignore = "Requires private GTK display/clipboard and TALIS_DEBUG_INFO_TEST=1"]
fn preview_copies_only_on_request_and_releases_on_close() {
    assert_eq!(std::env::var("TALIS_DEBUG_INFO_TEST").as_deref(), Ok("1"));
    gtk::init().unwrap();
    let display = gtk::gdk::Display::default().unwrap();
    style::install(&display);
    let parent = gtk::Window::new();
    parent.set_default_size(300, 100);
    parent.present();
    let clipboard = display.clipboard();
    clipboard.set_text("unchanged clipboard");
    let mut report = report::Report::new();
    report.field("Talis Version", env!("CARGO_PKG_VERSION"));
    report.field("OS", "Linux (private UI test)");
    report.field("GPU renderer", "Test renderer");
    report.settings(&crate::config::Config::default());
    report.playback(&crate::playback::PlaybackFeed::default(), Instant::now());
    let text = report.finish();
    let window = preview(&parent, text.clone());
    let weak = window.downgrade();
    window.present();
    let root = window.clone().upcast::<gtk::Widget>();
    assert!(!window.has_css_class("talis-controls"));
    assert!(window.child().unwrap().has_css_class("talis-controls"));
    assert_eq!(window.transient_for().as_ref(), Some(&parent));
    let view = style::find_widget(&root, "debug-info-report")
        .unwrap()
        .downcast::<gtk::TextView>()
        .unwrap();
    assert!(!view.is_editable());
    let buffer = view.buffer();
    assert_eq!(
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .as_str(),
        text
    );
    let context = glib::MainContext::default();
    let read = || {
        context
            .block_on(clipboard.read_text_future())
            .unwrap()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        read(),
        "unchanged clipboard",
        "opening must not replace clipboard"
    );
    let copy = style::find_widget(&root, "debug-info-copy")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    copy.emit_clicked();
    assert_eq!(read(), text);
    assert_eq!(copy.label().as_deref(), Some("Copied"));
    if let Ok(path) = std::env::var("TALIS_DEBUG_INFO_CAPTURE") {
        let until = Instant::now() + std::time::Duration::from_millis(200);
        while Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        window
            .renderer()
            .unwrap()
            .render_texture(snapshot.to_node().unwrap(), None)
            .save_to_png(path)
            .unwrap();
    }
    let close = style::find_widget(&root, "debug-info-close")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    close.emit_clicked();
    drop((close, copy, view, buffer, root, window));
    while context.pending() {
        context.iteration(false);
    }
    assert!(
        weak.upgrade().is_none(),
        "support window must not retain itself"
    );
    assert_eq!(read(), text);
    let window = preview(&parent, "Next snapshot".into());
    let weak = window.downgrade();
    window.present();
    parent.destroy();
    drop(parent);
    drop(window);
    while context.pending() {
        context.iteration(false);
    }
    assert!(
        weak.upgrade().is_none(),
        "closing Settings must release its support window"
    );
}
