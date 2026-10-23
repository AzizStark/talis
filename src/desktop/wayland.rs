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

fn scene_layer(desktops: &str, preference: Option<&str>) -> Result<Layer> {
    match preference {
        Some("background") => Ok(Layer::Background),
        Some("bottom") => Ok(Layer::Bottom),
        Some(_) => anyhow::bail!("TALIS_WAYLAND_LAYER must be background or bottom"),
        // Hyprland bars can place their decorations in the bottom layer. Keep
        // the scene below them; preserve other desktops' existing placement.
        None if desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("Hyprland")) =>
        {
            Ok(Layer::Background)
        }
        None => Ok(Layer::Bottom),
    }
}

#[derive(Clone)]
pub(super) struct DesktopPlacement {
    monitor: gdk::Monitor,
    index: u32,
    layer: Layer,
}
impl DesktopPlacement {
    pub fn new(index: u32) -> Result<Self> {
        let preference = match std::env::var("TALIS_WAYLAND_LAYER") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => return Err(error).context("Cannot read TALIS_WAYLAND_LAYER"),
        };
        let layer = scene_layer(
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            preference.as_deref(),
        )?;
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
        Ok(Self {
            monitor,
            index,
            layer,
        })
    }
    pub fn description(&self) -> String {
        let (layer, warning) = if self.layer == Layer::Background {
            ("background", "")
        } else {
            ("bottom", " · icons/widgets may be covered")
        };
        format!(
            "Experimental Wayland {layer} layer · monitor {} · selective input{warning}",
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
        tracing::info!(namespace, layer = ?self.layer, "Configuring experimental Wayland scene role");
        window.set_layer(self.layer);
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
    use super::{Layer, scene_layer, scene_namespace};

    #[test]
    fn hyprland_scene_stays_below_bottom_layer_bar_decorations() {
        for desktop in ["Hyprland", "hyprland", "wlroots:Hyprland"] {
            assert_eq!(scene_layer(desktop, None).unwrap(), Layer::Background);
        }
        for desktop in ["", "KDE", "sway", "GNOME", "notHyprland"] {
            assert_eq!(scene_layer(desktop, None).unwrap(), Layer::Bottom);
        }
    }

    #[test]
    fn explicit_layer_can_resolve_wallpaper_or_overlay_conflicts() {
        assert_eq!(
            scene_layer("Hyprland", Some("bottom")).unwrap(),
            Layer::Bottom
        );
        assert_eq!(
            scene_layer("KDE", Some("background")).unwrap(),
            Layer::Background
        );
        for value in ["", "auto", "top", "overlay", "invalid"] {
            assert!(scene_layer("Hyprland", Some(value)).is_err());
        }
    }

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
