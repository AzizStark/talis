//! Experimental native layer-shell placement; no foreign-window mutation.
use anyhow::{Context, Result, ensure};
use gtk::{gdk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

fn scene_namespace(desktops: &str) -> &'static str {
    // KWin maps the exact "desktop" scope to its desktop window type. A
    // generic bottom-layer scope is a normal window and Show Desktop hides it.
    // Keep other compositors' existing namespace/rule matching unchanged.
    if super::host::plasma_session(desktops) {
        "desktop"
    } else {
        "talis-desktop-experimental"
    }
}

#[derive(Clone)]
pub(super) struct DesktopPlacement {
    monitor: gdk::Monitor,
    index: u32,
}
impl DesktopPlacement {
    pub fn new(index: u32) -> Result<Self> {
        let display = gdk::Display::default().context("No GTK display")?;
        ensure!(
            display.type_().name() == "GdkWaylandDisplay",
            "Experimental Wayland desktop requires native Wayland, not X11/XWayland"
        );
        ensure!(
            gtk4_layer_shell::is_supported(),
            "This compositor does not expose layer-shell; no desktop window was created"
        );
        let monitor = display
            .monitors()
            .item(index)
            .and_then(|o| o.downcast::<gdk::Monitor>().ok())
            .context("Requested Wayland output does not exist")?;
        Ok(Self { monitor, index })
    }
    pub fn description(&self) -> String {
        format!(
            "Experimental Wayland bottom layer · monitor {} · selective input · icons/widgets may be covered",
            self.index
        )
    }
    pub fn configure(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        ensure!(
            !window.is_realized(),
            "Layer-shell role must be assigned before realization"
        );
        window.set_decorated(false);
        window.set_focusable(false);
        window.set_default_size(
            self.monitor.geometry().width(),
            self.monitor.geometry().height(),
        );
        window.init_layer_shell();
        let namespace = scene_namespace(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
        window.set_namespace(Some(namespace));
        tracing::info!(namespace, "Configuring experimental Wayland scene role");
        window.set_layer(Layer::Bottom);
        window.set_monitor(Some(&self.monitor));
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            window.set_anchor(edge, true);
        }
        window.connect_map(|window| {
            if let Some(surface) = window.surface() {
                surface.set_input_region(&gtk::cairo::Region::create());
            }
        });
        self.monitor.connect_invalidate({
            let weak = window.downgrade();
            move |_| {
                if let Some(window) = weak.upgrade() {
                    tracing::warn!(
                        "Selected Wayland output removed; closing experimental desktop session"
                    );
                    if let Some(app) = window.application() {
                        app.quit();
                    }
                }
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::scene_namespace;

    #[test]
    fn plasma_scene_uses_the_exact_kwin_desktop_scope() {
        for desktop in ["KDE", "kde", "KDE:Plasma", "other:KDE", "KDE:other"] {
            assert_eq!(scene_namespace(desktop), "desktop", "{desktop}");
        }
    }

    #[test]
    fn other_desktops_keep_the_existing_namespace() {
        for desktop in [
            "",
            "sway",
            "Hyprland",
            "GNOME",
            "X-Cinnamon",
            "MATE",
            "KDEish",
            "notKDE",
            "Plasma",
        ] {
            assert_eq!(
                scene_namespace(desktop),
                "talis-desktop-experimental",
                "{desktop}"
            );
        }
    }
}
