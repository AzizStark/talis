//! Desktop host selection. Native handles stay inside platform adapters.
pub(crate) mod geometry;
#[cfg(feature = "desktop")]
mod host;
#[cfg(feature = "desktop")]
pub mod screensaver;
#[cfg(feature = "desktop")]
pub(crate) mod visibility;
#[cfg(feature = "wayland-desktop")]
mod wayland;
#[cfg(feature = "desktop")]
mod x11;
#[cfg(feature = "desktop")]
pub(crate) use host::SurfacePlacement;
#[cfg(feature = "desktop")]
pub use host::{BackendKind, Capabilities, DesktopHost};
#[cfg(all(test, feature = "desktop"))]
pub(crate) use x11::window_id as test_window_id;
#[cfg(feature = "desktop")]
pub(crate) use x11::window_manager_name;
#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    X11,
    Wayland,
    Unknown,
}
impl Session {
    pub fn detect() -> Self {
        match std::env::var("XDG_SESSION_TYPE").as_deref() {
            Ok("wayland") => Self::Wayland,
            Ok("x11") => Self::X11,
            _ if std::env::var_os("WAYLAND_DISPLAY").is_some() => Self::Wayland,
            _ if std::env::var_os("DISPLAY").is_some() => Self::X11,
            _ => Self::Unknown,
        }
    }
}
