use super::*;

#[test]
#[ignore = "Requires a private GTK display and session bus; tests no real extension or logout"]
fn later_is_default_and_closing_releases_each_setup_state() {
    gtk::init().unwrap();
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.SetupTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    for (index, status) in [
        Status::Prepare,
        Status::EnableNow,
        Status::SignIn { enabled: false },
        Status::SignIn { enabled: true },
        Status::Starting,
    ]
    .into_iter()
    .enumerate()
    {
        let ui = Ui::new(&app);
        ui.display(status);
        let cancel =
            super::super::style::find_widget(&ui.window.clone().upcast(), "gnome-setup-later")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
        assert_eq!(
            ui.window.default_widget().as_ref(),
            Some(cancel.upcast_ref())
        );
        assert!(
            ui.task.borrow().is_none(),
            "showing a consent dialog alone must not invoke any action"
        );
        struct Pending(std::rc::Rc<std::cell::Cell<bool>>);
        impl Drop for Pending {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let cancelled = Rc::new(Cell::new(false));
        let pending = Pending(cancelled.clone());
        *ui.task.borrow_mut() = Some(glib::spawn_future_local(async move {
            let _pending = pending;
            std::future::pending::<()>().await;
        }));
        let weak = Rc::downgrade(&ui);
        ui.window.present();
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
        if let Ok(directory) = std::env::var("TALIS_GNOME_SETUP_CAPTURE_DIRECTORY") {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                context.iteration(false);
                let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(
                    &snapshot,
                    ui.window.width() as f64,
                    ui.window.height() as f64,
                );
                if let Some(node) = snapshot.to_node() {
                    ui.window
                        .renderer()
                        .unwrap()
                        .render_texture(&node, None)
                        .save_to_png(
                            std::path::Path::new(&directory).join(format!("setup-{index}.png")),
                        )
                        .unwrap();
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "setup did not draw");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        cancel.emit_clicked();
        drop(ui);
        while context.pending() {
            context.iteration(false);
        }
        assert!(
            weak.upgrade().is_none(),
            "setup controller leaked after Later"
        );
        assert!(
            cancelled.get(),
            "pending setup work must be cancelled on close"
        );
    }
}
