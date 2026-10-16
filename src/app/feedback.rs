//! Retain existing error sources, but keep their visibility out of page layout.
use gtk::prelude::*;

pub(super) fn wrap(content: &gtk::Box) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(content));
    let notices = gtk::Box::new(gtk::Orientation::Vertical, 6);
    notices.set_widget_name("settings-notifications");
    notices.set_halign(gtk::Align::End);
    notices.set_valign(gtk::Align::End);
    notices.set_margin_end(16);
    notices.set_margin_bottom(16);
    let mut sources = Vec::new();
    collect(&content.clone().upcast(), &mut sources);
    for label in sources {
        if let Some(parent) = label.parent().and_downcast::<gtk::Box>() {
            parent.remove(&label);
            label.remove_css_class("card-footer");
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(46);
            label.set_lines(3);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("settings-notification");
            // Size each toast independently instead of filling the widest row.
            row.set_halign(gtk::Align::End);
            row.set_visible(label.get_visible());
            row.append(&label);
            let close = gtk::Button::from_icon_name("window-close-symbolic");
            close.set_widget_name("dismiss-settings-notification");
            close.add_css_class("flat");
            close.set_valign(gtk::Align::Center);
            close.set_tooltip_text(Some("Dismiss notification"));
            let weak_label = label.downgrade();
            close.connect_clicked(move |_| {
                if let Some(label) = weak_label.upgrade() {
                    label.set_visible(false);
                }
            });
            row.append(&close);
            let weak_row = row.downgrade();
            label.connect_visible_notify(move |label| {
                if let Some(row) = weak_row.upgrade() {
                    row.set_visible(label.get_visible());
                }
            });
            notices.append(&row);
        }
    }
    overlay.add_overlay(&notices);
    overlay.set_measure_overlay(&notices, false);
    overlay
}

fn collect(widget: &gtk::Widget, sources: &mut Vec<gtk::Label>) {
    if widget.has_css_class("settings-feedback-source") {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            sources.push(label.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect(&current, sources);
        child = current.next_sibling();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a private GTK display"]
    fn notifications_size_to_their_own_message() {
        gtk::init().unwrap();
        super::super::style::install(&gtk::gdk::Display::default().unwrap());
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("talis-controls");
        let short = super::super::graphics_notice::source();
        let long = gtk::Label::new(Some(&"Long error details ".repeat(30)));
        long.add_css_class("settings-feedback-source");
        long.set_visible(false);
        page.append(&short);
        page.append(&long);
        let overlay = wrap(&page);
        overlay.add_css_class("talis-controls");
        let window = gtk::Window::new();
        window.set_default_size(800, 300);
        window.set_child(Some(&overlay));
        short.set_visible(true);
        long.set_visible(true);
        window.present();
        let context = gtk::glib::MainContext::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while std::time::Instant::now() < until {
            context.iteration(false);
        }
        let short_row = short.parent().unwrap();
        let long_row = long.parent().unwrap();
        eprintln!(
            "short row={} label={} text={:?}; long row={} label={} text={:?}",
            short_row.width(),
            short.width(),
            short.layout().pixel_size(),
            long_row.width(),
            long.width(),
            long.layout().pixel_size()
        );
        if let Some(path) = std::env::var_os("TALIS_TOAST_TEST_CAPTURE") {
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            let texture = window
                .renderer()
                .unwrap()
                .render_texture(snapshot.to_node().unwrap(), None);
            texture.save_to_png(std::path::PathBuf::from(path)).unwrap();
        }
        assert!(short_row.width() < long_row.width());
        assert!(short.width() <= short.layout().pixel_size().0 + 16);
        window.destroy();
    }

    #[test]
    #[ignore = "Requires a private GTK display"]
    fn error_notifications_do_not_change_page_measurement_and_release_sources() {
        gtk::init().unwrap();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&gtk::Label::new(Some("Scene settings")));
        let status = gtk::Label::new(None);
        status.add_css_class("settings-feedback-source");
        status.set_visible(false);
        page.append(&status);
        let overlay = wrap(&page);
        assert_ne!(status.parent(), Some(page.clone().upcast()));
        let before = overlay.measure(gtk::Orientation::Vertical, 450);
        status.set_text("Could not load scene. The current scene is retained.");
        status.set_visible(true);
        assert!(
            status.get_visible() && status.parent().unwrap().get_visible(),
            "hidden notification parents must become visible"
        );
        assert_eq!(overlay.measure(gtk::Orientation::Vertical, 450), before);
        status.set_text(&"long error details ".repeat(50));
        assert_eq!(overlay.measure(gtk::Orientation::Vertical, 450), before);
        let close = status
            .parent()
            .unwrap()
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        close.emit_clicked();
        assert!(!status.is_visible());
        assert!(!status.parent().unwrap().is_visible());
        assert_eq!(overlay.measure(gtk::Orientation::Vertical, 450), before);
        drop(close);
        let weak = status.downgrade();
        drop(status);
        drop(overlay);
        assert!(weak.upgrade().is_none());
    }
}
