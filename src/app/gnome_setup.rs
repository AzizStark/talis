//! Native first-launch assistant, separate from rendering and saved scene settings.
mod backend;
use backend::{Client, Status};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) fn run() -> anyhow::Result<()> {
    anyhow::ensure!(
        crate::desktop::Session::detect() == crate::desktop::Session::Wayland
            && std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .any(|name| name.eq_ignore_ascii_case("GNOME")),
        "GNOME setup is only available in a GNOME Wayland session"
    );
    gtk::init()?;
    let app = gtk::Application::builder()
        .application_id(super::style::APP_ICON_NAME)
        .build();
    app.connect_startup(|app| {
        if let Some(connection) = app.dbus_connection() {
            connection.set_exit_on_close(false);
        }
        if let Some(display) = gtk::gdk::Display::default() {
            super::style::install_app_icon(&display);
        }
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let ui = Ui::new(app);
        ui.window.present();
        Ui::perform(&ui, Action::Check);
    });
    app.run_with_args::<&str>(&[]);
    Ok(())
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
enum Action {
    Check,
    Enable,
    Logout,
}

struct Ui {
    window: gtk::ApplicationWindow,
    title: gtk::Label,
    message: gtk::Label,
    primary: gtk::Button,
    spinner: gtk::Spinner,
    action: Cell<Action>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}
impl Ui {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Talis · Setup")
            .default_width(440)
            .resizable(false)
            .build();
        window.set_widget_name("gnome-setup");
        super::style::bind_window_icon(&window);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
        body.set_margin_top(28);
        body.set_margin_bottom(28);
        body.set_margin_start(28);
        body.set_margin_end(28);
        let title = gtk::Label::new(Some("Setting up Talis"));
        title.add_css_class("title-2");
        title.set_xalign(0.0);
        let message = gtk::Label::new(Some("Checking the GNOME desktop companion…"));
        message.set_wrap(true);
        message.set_max_width_chars(48);
        message.set_xalign(0.0);
        body.append(&title);
        body.append(&message);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.set_halign(gtk::Align::End);
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        spinner.set_halign(gtk::Align::Start);
        let later = gtk::Button::with_label("Later");
        later.set_widget_name("gnome-setup-later");
        let primary = gtk::Button::with_label("Try Again");
        primary.set_widget_name("gnome-setup-primary");
        primary.add_css_class("suggested-action");
        buttons.append(&spinner);
        buttons.append(&later);
        buttons.append(&primary);
        body.append(&buttons);
        window.set_child(Some(&body));
        // Enter/Escape and window-close must never enable an extension or log out.
        window.set_default_widget(Some(&later));
        let ui = Rc::new(Self {
            window,
            title,
            message,
            primary,
            spinner,
            action: Cell::new(Action::Check),
            task: RefCell::new(None),
        });
        let weak = Rc::downgrade(&ui);
        ui.primary.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let action = ui.action.get();
                Self::perform(&ui, action);
            }
        });
        let weak_window = ui.window.downgrade();
        later.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                window.close();
            }
        });
        // Keep the controller alive for precisely the window's lifetime.
        let keep = Rc::new(RefCell::new(Some(ui.clone())));
        let closing = keep.clone();
        ui.window.connect_close_request(move |_| {
            Self::release(&closing);
            glib::Propagation::Proceed
        });
        ui.window.connect_destroy(move |_| {
            Self::release(&keep);
        });
        ui
    }

    fn release(keep: &RefCell<Option<Rc<Self>>>) {
        // Release the keeper borrow before dropping widgets can emit destroy.
        let ui = keep.borrow_mut().take();
        if let Some(ui) = ui {
            if let Some(task) = ui.task.borrow_mut().take() {
                task.abort();
            }
        }
    }

    fn display(&self, status: Status) {
        let (title, message, button, action) = match status {
            Status::EnableNow => (
                "Enable Talis on your desktop",
                "Enable the GNOME companion to show Talis behind your windows. Talis will also start when you sign in.",
                "Enable Talis",
                Action::Enable,
            ),
            Status::Prepare | Status::SignIn { enabled: false } => (
                "Finish setting up Talis",
                "Enable the included GNOME companion to show Talis on your desktop. GNOME needs one logout/login to finish setup. Talis will then start automatically when you sign in.",
                "Set Up Talis",
                Action::Enable,
            ),
            Status::SignIn { enabled: true } => (
                "Ready for your next sign-in",
                "Talis is set to start automatically after you log out and back in. Save your work first. You can sign out later or open GNOME’s logout confirmation now.",
                "Log Out…",
                Action::Logout,
            ),
            Status::Starting => (
                "Starting Talis",
                "The GNOME companion is enabled. Talis may take a moment to load. Try again to open Settings.",
                "Try Again",
                Action::Check,
            ),
        };
        self.title.set_text(title);
        self.message.set_text(message);
        self.primary.set_label(button);
        self.action.set(action);
    }

    fn perform(ui: &Rc<Self>, action: Action) {
        if !ui.primary.is_sensitive() {
            return;
        }
        ui.primary.set_sensitive(false);
        ui.spinner.set_visible(true);
        ui.spinner.start();
        let weak = Rc::downgrade(ui);
        let task = glib::spawn_future_local(async move {
            let result = async {
                let client = Client::connect().await?;
                let status = match action {
                    Action::Check => client.status().await?,
                    Action::Enable => client.enable().await?,
                    Action::Logout => {
                        client.request_logout().await?;
                        Status::SignIn { enabled: true }
                    }
                };
                let opened = status == Status::Starting && client.open_settings().await;
                Ok::<_, anyhow::Error>((status, opened))
            }
            .await;
            let Some(ui) = weak.upgrade().filter(|ui| ui.window.is_visible()) else {
                return;
            };
            ui.spinner.stop();
            ui.spinner.set_visible(false);
            ui.primary.set_sensitive(true);
            match result {
                Ok((_, true)) => {
                    ui.window.close();
                }
                Ok((status, false)) => ui.display(status),
                Err(error) => {
                    ui.title.set_text("Talis setup needs attention");
                    ui.message.set_text(&format!("{error:#}"));
                    ui.primary.set_label("Try Again");
                    ui.action.set(Action::Check);
                }
            }
        });
        *ui.task.borrow_mut() = Some(task);
    }
}
