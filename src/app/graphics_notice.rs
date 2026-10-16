//! One dismissible software-rendering notice per Settings window lifetime.
use super::{lifecycle::SharedRenderer, style};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub(super) fn source() -> gtk::Label {
    let label = style::label(
        "Software rendering detected. Performance may be reduced.",
        "quiet-status",
    );
    label.set_widget_name("software-rendering-notice");
    label.add_css_class("settings-feedback-source");
    label.set_visible(false);
    label
}

pub(super) fn attach(
    controls: &gtk::ApplicationWindow,
    area: &gtk::GLArea,
    renderer: &SharedRenderer,
) {
    let Some(label) = style::find_widget(&controls.clone().upcast(), "software-rendering-notice")
        .and_downcast::<gtk::Label>()
    else {
        return;
    };
    let label = label.downgrade();
    let renderer = Rc::downgrade(renderer);
    let notified = Rc::new(Cell::new(false));
    let refresh = move || {
        let (Some(label), Some(renderer)) = (label.upgrade(), renderer.upgrade()) else {
            return;
        };
        // Drop the renderer borrow before GTK notification handlers can run.
        let software = renderer
            .borrow()
            .as_ref()
            .map(|renderer| renderer.info().software_rendering());
        update(&label, &notified, software);
    };
    let realized = refresh.clone();
    // Attached after the lifecycle's realize handler initializes the renderer.
    // No timer or per-frame GL query; a hidden Settings window retains the notice.
    area.connect_realize(move |_| realized());
    refresh();
}

fn update(label: &gtk::Label, notified: &Cell<bool>, software: Option<bool>) {
    if software == Some(true) {
        if !notified.replace(true) {
            label.set_visible(true);
        }
    } else {
        label.set_visible(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires private GTK display, LIBGL_ALWAYS_SOFTWARE=1 and TALIS_SOFTWARE_RENDERING_TEST=1"]
    fn real_software_context_updates_notice_after_initialization() {
        assert_eq!(
            std::env::var("TALIS_SOFTWARE_RENDERING_TEST").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref(), Ok("1"));
        gtk::init().unwrap();
        let app = gtk::Application::builder()
            .application_id("io.github.azizstark.Talis.SoftwareRenderingTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let controls = gtk::ApplicationWindow::builder().application(&app).build();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let label = source();
        page.append(&label);
        controls.set_child(Some(&super::super::feedback::wrap(&page)));
        let scene = gtk::Window::new();
        scene.set_default_size(320, 180);
        let area = gtk::GLArea::new();
        area.set_required_version(3, 3);
        area.set_use_es(false);
        scene.set_child(Some(&area));
        let mut config = crate::config::Config::default();
        config.scene.mode = crate::config::PreviewMode::Beauty;
        let asset =
            std::cell::RefCell::new(Some(crate::preview::PreviewAsset::load(&config).unwrap()));
        let renderer: SharedRenderer = Rc::new(std::cell::RefCell::new(None));
        let initializing = renderer.clone();
        area.connect_realize(move |area| {
            area.make_current();
            assert!(area.error().is_none(), "{:?}", area.error());
            *initializing.borrow_mut() = Some(unsafe {
                crate::rendering::Renderer::new(asset.borrow_mut().take().unwrap()).unwrap()
            });
        });
        attach(&controls, &area, &renderer);
        assert!(!label.get_visible(), "unknown renderer must not warn");
        scene.present();
        let context = gtk::glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        assert!(
            renderer
                .borrow()
                .as_ref()
                .unwrap()
                .info()
                .software_rendering()
        );
        assert!(label.get_visible(), "initialization must show the notice");
        assert!(
            !controls.get_visible(),
            "warning must not force Settings open"
        );
        area.make_current();
        unsafe {
            renderer.borrow_mut().take().unwrap().destroy();
        }
        controls.destroy();
        scene.destroy();
    }

    #[test]
    #[ignore = "Requires a private GTK display"]
    fn notice_is_informational_dismissible_and_does_not_shift_layout() {
        gtk::init().unwrap();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&gtk::Label::new(Some("Settings content")));
        let label = source();
        page.append(&label);
        let overlay = super::super::feedback::wrap(&page);
        let initial = overlay.measure(gtk::Orientation::Vertical, 450);
        let notified = Cell::new(false);
        for software in [None, Some(false)] {
            update(&label, &notified, software);
            assert!(!label.get_visible());
            assert!(!notified.get());
        }
        update(&label, &notified, Some(true));
        assert!(label.get_visible() && label.parent().unwrap().get_visible());
        assert_eq!(overlay.measure(gtk::Orientation::Vertical, 450), initial);
        let close = label
            .parent()
            .unwrap()
            .last_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        close.emit_clicked();
        assert!(!label.get_visible() && !label.parent().unwrap().get_visible());
        for software in [Some(true), None, Some(false), Some(true)] {
            update(&label, &notified, software);
            assert!(
                !label.get_visible(),
                "dismissal survives context recreation"
            );
        }
        let next_launch = Cell::new(false);
        update(&label, &next_launch, Some(true));
        assert!(label.get_visible());
        update(&label, &next_launch, Some(false));
        assert!(
            !label.get_visible(),
            "do not retain a warning after changing renderer"
        );
        drop(close);
        let weak = label.downgrade();
        drop(label);
        drop(overlay);
        assert!(weak.upgrade().is_none());
    }
}
