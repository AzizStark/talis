//! Layer preference resolution is independent of GTK and never changes settings.
use crate::config::WaylandLayer;
use anyhow::{Result, bail};
use std::ffi::OsStr;

pub(crate) fn environment_override() -> Result<Option<WaylandLayer>> {
    parse_override(std::env::var_os("TALIS_WAYLAND_LAYER").as_deref())
}

fn parse_override(value: Option<&OsStr>) -> Result<Option<WaylandLayer>> {
    match value {
        None => Ok(None),
        Some(value) => match value.to_str().and_then(WaylandLayer::from_id) {
            Some(layer @ (WaylandLayer::Background | WaylandLayer::Bottom)) => Ok(Some(layer)),
            _ => bail!("TALIS_WAYLAND_LAYER must be background or bottom"),
        },
    }
}

pub(crate) fn resolve(
    desktops: &str,
    preference: WaylandLayer,
    environment: Option<WaylandLayer>,
) -> WaylandLayer {
    match environment.unwrap_or(preference) {
        WaylandLayer::Automatic
            if desktops
                .split(':')
                .any(|desktop| desktop.eq_ignore_ascii_case("Hyprland")) =>
        {
            WaylandLayer::Background
        }
        WaylandLayer::Automatic => WaylandLayer::Bottom,
        explicit => explicit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn automatic_layer_keeps_existing_desktop_defaults() {
        for desktop in ["Hyprland", "hyprland", "wlroots:Hyprland"] {
            assert_eq!(
                resolve(desktop, WaylandLayer::Automatic, None),
                WaylandLayer::Background
            );
        }
        for desktop in ["", "KDE", "sway", "GNOME", "notHyprland"] {
            assert_eq!(
                resolve(desktop, WaylandLayer::Automatic, None),
                WaylandLayer::Bottom
            );
        }
    }

    #[test]
    fn environment_precedes_saved_choice_and_saved_choice_precedes_automatic() {
        for desktop in ["Hyprland", "KDE", "sway"] {
            for preference in [
                WaylandLayer::Automatic,
                WaylandLayer::Background,
                WaylandLayer::Bottom,
            ] {
                for environment in [WaylandLayer::Background, WaylandLayer::Bottom] {
                    assert_eq!(resolve(desktop, preference, Some(environment)), environment);
                }
            }
            for preference in [WaylandLayer::Background, WaylandLayer::Bottom] {
                assert_eq!(resolve(desktop, preference, None), preference);
            }
        }
    }

    #[test]
    fn environment_accepts_only_the_existing_explicit_overrides() {
        assert_eq!(parse_override(None).unwrap(), None);
        for layer in [WaylandLayer::Background, WaylandLayer::Bottom] {
            assert_eq!(
                parse_override(Some(OsStr::new(layer.id()))).unwrap(),
                Some(layer)
            );
        }
        for value in ["", "automatic", "auto", "top", "overlay", "invalid"] {
            assert!(parse_override(Some(OsStr::new(value))).is_err());
        }
        assert!(parse_override(Some(OsStr::from_bytes(b"\xff"))).is_err());
    }
}
