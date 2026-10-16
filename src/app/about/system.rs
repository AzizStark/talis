//! One-shot, read-only session information. No subprocesses or polling.
use gtk::prelude::*;

pub(super) fn desktop_name(value: &str) -> String {
    let known = value
        .split(':')
        .find_map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "cinnamon" | "x-cinnamon" => Some("Cinnamon"),
            "gnome" | "gnome-classic" => Some("GNOME"),
            "kde" | "plasma" => Some("KDE Plasma"),
            "mate" => Some("MATE"),
            "xfce" | "xfce4" => Some("Xfce"),
            "lxqt" => Some("LXQt"),
            "lxde" => Some("LXDE"),
            "budgie" | "budgie:gnome" => Some("Budgie"),
            "pantheon" => Some("Pantheon"),
            "sway" => Some("Sway"),
            "hyprland" => Some("Hyprland"),
            _ => None,
        });
    known.map(str::to_owned).unwrap_or_else(|| {
        let value = value.trim();
        if value.is_empty() {
            "Not reported".into()
        } else {
            value.chars().take(100).collect()
        }
    })
}

fn wayland_manager(desktop: &str) -> &'static str {
    // Wayland has no universal WM-name query. Mark these as session-derived,
    // unlike X11's actual window-manager report, rather than pretending to probe.
    match desktop {
        "GNOME" => "Mutter (session)",
        "KDE Plasma" => "KWin (session)",
        "Cinnamon" => "Muffin (session)",
        "Sway" => "Sway (session)",
        "Hyprland" => "Hyprland (session)",
        _ => "Not reported",
    }
}

pub(in crate::app) fn details() -> [(&'static str, &'static str, String); 4] {
    let os = gtk::glib::os_info("PRETTY_NAME")
        .or_else(|| gtk::glib::os_info("NAME"))
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Linux".into());
    let session = [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .into_iter()
    .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))
    .unwrap_or_default();
    let desktop = desktop_name(&session);
    let wayland =
        gtk::gdk::Display::default().is_some_and(|d| d.type_().name() == "GdkWaylandDisplay");
    let manager = crate::desktop::window_manager_name().unwrap_or_else(|| {
        if wayland {
            wayland_manager(&desktop).into()
        } else {
            "Not reported".into()
        }
    });
    [
        (
            "Talis Version",
            "about-version",
            env!("CARGO_PKG_VERSION").into(),
        ),
        ("OS", "about-os", os),
        ("Desktop environment", "about-desktop", desktop),
        ("Window manager", "about-wm", manager),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_names_are_readable_and_unknown_values_are_not_guessed() {
        for (value, expected) in [
            ("X-Cinnamon", "Cinnamon"),
            ("ubuntu:GNOME", "GNOME"),
            ("KDE", "KDE Plasma"),
            ("xfce", "Xfce"),
            ("MATE", "MATE"),
            ("", "Not reported"),
            ("custom", "custom"),
        ] {
            assert_eq!(desktop_name(value), expected);
        }
        assert_eq!(wayland_manager("custom"), "Not reported");
        assert_eq!(wayland_manager("KDE Plasma"), "KWin (session)");
    }
}
