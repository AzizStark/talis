//! Explicit startup toggle; initial rendering only reads the autostart state.
mod prompt;
use super::{Options, state::AppState, style};
use crate::autostart::{Launch, Manager};
use anyhow::{Context, Result};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) fn controls(options: &Options, state: Rc<RefCell<AppState>>) -> gtk::Box {
    if options.shell_surface {
        // The enabled companion launches this owned client at GNOME login.
        // Never create a second, standalone X11 autostart entry for it.
        return extension_managed();
    }
    if options.experimental_wayland
        && !crate::desktop::DesktopHost::wayland_startup_session_supported()
    {
        // Missing backend support is expected, not an initialization failure.
        // Do not inspect/write autostart state or offer a prompt on this path.
        return unavailable();
    }
    let wayland_library = if options.experimental_wayland {
        wayland_startup_library()
    } else {
        Ok(None)
    };
    if let Err(error) = &wayland_library {
        return build(
            Err(anyhow::anyhow!("{error:#}")),
            || anyhow::bail!("Desktop startup is unavailable"),
            false,
        );
    }
    let wayland_library = wayland_library.unwrap();
    let manager = Manager::for_user();
    let offer_prompt = prompt::eligible(options)
        && (options.experimental_wayland || crate::desktop::DesktopHost::check_startup().is_ok());
    let options = options.clone();
    build(
        manager,
        move || {
            if !options.experimental_wayland {
                crate::desktop::DesktopHost::check_startup()?;
            }
            let state = state.borrow();
            let feed = state.playback.borrow();
            Ok(Launch {
                executable: crate::autostart::executable()?,
                directory: launch_directory(&state.config.scene)?,
                config: state.config.clone(),
                monitor: options.monitor,
                following: feed.enabled,
                player: feed.preferred.clone(),
                wayland_library: wayland_library.clone(),
                graphics_environment: if options.experimental_wayland {
                    vec![]
                } else {
                    ["GDK_DEBUG", "GDK_DISABLE"]
                        .into_iter()
                        .filter_map(|name| {
                            std::env::var(name)
                                .ok()
                                .filter(|v| !v.is_empty())
                                .map(|v| (name.into(), v))
                        })
                        .collect()
                },
            })
        },
        offer_prompt,
    )
}

fn launch_directory(scene: &crate::config::SceneSettings) -> Result<std::path::PathBuf> {
    // Bundled scenes have a durable installed root. Custom relative paths still
    // mean the caller's directory, never the application's data directory.
    if scene.bundled_resolution_available() {
        if let Some(directory) = crate::resources::data_directory()? {
            return Ok(directory);
        }
    }
    std::env::current_dir().context("Cannot locate scene directory")
}

fn wayland_startup_library() -> Result<Option<std::path::PathBuf>> {
    anyhow::ensure!(
        crate::desktop::DesktopHost::wayland_startup_session_supported(),
        "Login startup is not implemented for this Wayland desktop"
    );
    let preload = std::env::var("LD_PRELOAD").context(
        "Launch through scripts/run.sh in a Wayland session to configure the layer-shell library",
    )?;
    resolve_wayland_preload(
        &preload,
        &std::env::var("LD_LIBRARY_PATH").unwrap_or_default(),
    )
    .map(Some)
}

fn resolve_wayland_preload(preload: &str, search: &str) -> Result<std::path::PathBuf> {
    let library = preload
        .split(|c: char| c.is_whitespace() || c == ':')
        .find(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libgtk4-layer-shell.so"))
        })
        .context("No explicit gtk4-layer-shell preload is configured")?;
    let library = std::path::Path::new(library);
    let library = if library.is_absolute() {
        library.to_path_buf()
    } else {
        anyhow::ensure!(
            library.components().count() == 1,
            "Layer-shell preload must be absolute or a library name"
        );
        std::env::split_paths(search)
            .filter(|path| path.is_absolute())
            .map(|directory| directory.join(library))
            .find(|path| path.is_file())
            .context("Cannot find layer-shell preload in the explicit library search path")?
    };
    let library = library
        .canonicalize()
        .context("Cannot resolve layer-shell preload")?;
    anyhow::ensure!(library.is_file(), "Layer-shell preload is not a file");
    Ok(library)
}

fn row(description: &str) -> (gtk::Box, gtk::Switch, gtk::Label) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let toggle = gtk::Switch::new();
    toggle.set_widget_name("run-on-startup");
    toggle.set_tooltip_text(Some("Start Talis automatically when you sign in."));
    body.append(&style::row("Run on startup", description, &toggle));
    let status = style::label("", "quiet-status");
    status.add_css_class("card-footer");
    status.set_visible(false);
    status.set_widget_name("startup-status");
    status.add_css_class("settings-feedback-source");
    body.append(&status);
    (body, toggle, status)
}

fn unavailable() -> gtk::Box {
    let description = "Not available in this session";
    let (body, toggle, _) = row(description);
    toggle.set_sensitive(false);
    toggle.set_tooltip_text(Some(description));
    body
}

fn extension_managed() -> gtk::Box {
    let description = "Managed by the GNOME extension";
    let (body, toggle, _) = row(description);
    toggle.set_active(true);
    toggle.set_sensitive(false);
    toggle.set_tooltip_text(Some(description));
    body
}

fn build(
    manager: Result<Manager>,
    launch: impl Fn() -> Result<Launch> + 'static,
    offer_prompt: bool,
) -> gtk::Box {
    let (body, toggle, status) = row("Start Talis when you sign in");
    let manager =
        match manager.and_then(|manager| manager.enabled().map(|enabled| (manager, enabled))) {
            Ok((manager, enabled)) => {
                toggle.set_active(enabled);
                manager
            }
            Err(error) => {
                toggle.set_sensitive(false);
                status.set_text(&format!("Startup unavailable: {error:#}"));
                status.set_visible(true);
                return body;
            }
        };
    if offer_prompt {
        prompt::attach(&body, &toggle, manager.clone());
    }
    let previous = Cell::new(toggle.is_active());
    let syncing = Cell::new(false);
    let status = status.downgrade();
    toggle.connect_active_notify(move |toggle| {
        if syncing.get() {
            return;
        }
        let requested = toggle.is_active();
        // GObject may deliver the rollback's notify after this callback returns.
        // The restored value is already the persisted value, so don't act twice.
        if requested == previous.get() {
            return;
        }
        let result = if requested {
            launch().and_then(|launch| manager.enable(&launch))
        } else {
            manager.disable()
        };
        match result {
            Ok(()) => {
                previous.set(requested);
                if let Some(status) = status.upgrade() {
                    status.set_visible(false);
                }
            }
            Err(error) => {
                syncing.set(true);
                toggle.set_active(previous.get());
                syncing.set(false);
                if let Some(status) = status.upgrade() {
                    status.set_text(&format!("Could not change startup: {error:#}"));
                    status.set_visible(true);
                }
            }
        }
    });
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    #[test]
    fn layer_preload_resolves_names_in_directories_with_spaces_without_cwd_search() {
        let root =
            std::env::temp_dir().join(format!("talis-preload-{} with spaces", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let library = root.join("libgtk4-layer-shell.so.0");
        std::fs::write(&library, b"fixture").unwrap();
        assert_eq!(
            resolve_wayland_preload("libgtk4-layer-shell.so.0", root.to_str().unwrap()).unwrap(),
            library.canonicalize().unwrap()
        );
        // An absolute preload containing spaces is not a single loader token.
        assert!(resolve_wayland_preload(library.to_str().unwrap(), "").is_err());
        assert!(resolve_wayland_preload("libgtk4-layer-shell.so.0", "").is_err());
        assert!(
            resolve_wayland_preload("relative/libgtk4-layer-shell.so.0", root.to_str().unwrap())
                .is_err()
        );
        assert!(resolve_wayland_preload("/missing/libgtk4-layer-shell.so.0", "").is_err());
        std::fs::remove_file(library).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn launch_directory_is_durable_for_bundled_assets_but_preserves_custom_paths() {
        let bundled = crate::config::SceneSettings::default();
        assert_eq!(
            launch_directory(&bundled).unwrap(),
            crate::resources::data_directory().unwrap().unwrap()
        );
        let custom = crate::config::SceneSettings {
            background: "custom/beauty.png".into(),
            ..bundled
        };
        assert_eq!(
            launch_directory(&custom).unwrap(),
            std::env::current_dir().unwrap()
        );
    }
    #[test]
    #[ignore = "Requires an isolated GTK display; writes only to a temporary config directory"]
    fn startup_toggle_creates_removes_and_rolls_back_without_account_changes() {
        gtk::init().unwrap();
        crate::desktop::DesktopHost::check_startup().unwrap();
        let fixture = crate::autostart::Fixture::new();
        let manager = fixture.manager();
        let saved_launch = fixture.launch();
        let body = build(
            Ok(manager.clone()),
            move || {
                Ok(Launch {
                    executable: saved_launch.executable.clone(),
                    directory: saved_launch.directory.clone(),
                    config: saved_launch.config.clone(),
                    monitor: 0,
                    following: false,
                    player: None,
                    graphics_environment: vec![],
                    wayland_library: None,
                })
            },
            false,
        );
        let toggle = style::find_widget(&body.clone().upcast(), "run-on-startup")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        assert!(!toggle.is_active());
        assert!(!fixture.root.join("autostart").exists());
        toggle.set_active(true);
        assert!(manager.enabled().unwrap());
        let status = style::find_widget(&body.clone().upcast(), "startup-status").unwrap();
        assert!(
            !status.is_visible(),
            "success feedback must not expand the UI"
        );
        toggle.set_active(false);
        assert!(!manager.enabled().unwrap());
        assert!(
            !status.is_visible(),
            "disabled state is already shown by the switch"
        );
        let weak = toggle.downgrade();
        drop(toggle);
        drop(body);
        assert!(weak.upgrade().is_none());
        let body = build(Ok(manager), || bail!("Test launch failure"), false);
        let toggle = style::find_widget(&body.clone().upcast(), "run-on-startup")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        toggle.set_active(true);
        assert!(!toggle.is_active(), "failed write must restore the switch");
        let status = style::find_widget(&body.upcast(), "startup-status")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(status.is_visible() && status.text().contains("Test launch failure"));
    }

    #[test]
    #[ignore = "Requires an isolated GTK display; does not read or write account startup settings"]
    fn unsupported_startup_is_quiet_but_initialization_errors_still_report() {
        gtk::init().unwrap();
        let managed = extension_managed().upcast::<gtk::Widget>();
        let toggle = style::find_widget(&managed, "run-on-startup")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        assert!(toggle.is_active() && !toggle.is_sensitive());
        assert_eq!(
            toggle.tooltip_text().as_deref(),
            Some("Managed by the GNOME extension")
        );
        assert!(
            !style::find_widget(&managed, "startup-status")
                .unwrap()
                .is_visible()
        );
        let body = unavailable().upcast::<gtk::Widget>();
        let toggle = style::find_widget(&body, "run-on-startup")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        assert!(!toggle.is_sensitive() && !toggle.is_active());
        assert_eq!(
            toggle.tooltip_text().as_deref(),
            Some("Not available in this session")
        );
        let status = style::find_widget(&body, "startup-status")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(!status.is_visible() && status.text().is_empty());
        let failure = build(
            Err(anyhow::anyhow!("Test ownership failure")),
            || bail!("Must not launch"),
            false,
        )
        .upcast::<gtk::Widget>();
        let status = style::find_widget(&failure, "startup-status")
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert!(status.is_visible() && status.text().contains("Test ownership failure"));
    }
}
