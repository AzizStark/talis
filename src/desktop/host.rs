//! Desktop boundary; Wayland layer-shell is explicitly experimental and opt-in.
use super::{Session, visibility, x11};
use anyhow::{Context, Result, ensure};
use gtk::{gdk, prelude::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    X11,
    LayerShell,
}

/// Supported mechanisms, not a guarantee of fresh observations or WM acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub selective_input: bool,
    pub separate_animation_surface: bool,
    pub visibility_probe: bool,
    pub desktop_startup: bool,
}
impl BackendKind {
    pub fn capabilities(self) -> Capabilities {
        match self {
            Self::X11 => Capabilities {
                selective_input: true,
                separate_animation_surface: true,
                visibility_probe: true,
                desktop_startup: true,
            },
            Self::LayerShell => Capabilities {
                selective_input: true,
                separate_animation_surface: false,
                visibility_probe: false,
                desktop_startup: false,
            },
        }
    }
}

fn select_backend(display_type: &str, session: Session, input_shapes: bool) -> Result<BackendKind> {
    ensure!(
        session != Session::Wayland && display_type == "GdkX11Display",
        "Desktop mode currently supports native X11 sessions only. Wayland needs a compositor-specific adapter; use preview mode without --desktop for now"
    );
    ensure!(
        input_shapes,
        "This X11 display cannot provide click-through desktop surfaces"
    );
    Ok(BackendKind::X11)
}

pub(super) fn check_display(display: &gdk::Display) -> Result<BackendKind> {
    select_backend(
        display.type_().name(),
        Session::detect(),
        display.supports_input_shapes(),
    )
}

#[derive(Clone)]
enum HostBackend {
    X11(x11::DesktopPlacement),
    #[cfg(feature = "wayland-desktop")]
    LayerShell(super::wayland::DesktopPlacement),
}

/// GTK-thread-owned desktop placement and platform factories. Renderer resources
/// remain owned by the existing GL lifecycle; the host never owns their textures.
#[derive(Clone)]
pub struct DesktopHost {
    backend: HostBackend,
}
impl DesktopHost {
    /// Restrict this opt-in workaround to MATE, never a Cinnamon session.
    pub(crate) fn mate_overlay_available() -> bool {
        mate_session(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
    }
    pub(crate) fn set_initial_mate_overlay(&self, enabled: bool) {
        match &self.backend {
            HostBackend::X11(placement) => {
                placement.set_overlay(enabled && Self::mate_overlay_available())
            }
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => {}
        }
    }
    pub(crate) fn set_mate_overlay(
        &self,
        scene: &gtk::ApplicationWindow,
        patch: Option<&gtk::ApplicationWindow>,
        enabled: bool,
    ) -> Result<()> {
        ensure!(
            !enabled || Self::mate_overlay_available(),
            "Experimental overlay is available only in MATE"
        );
        match &self.backend {
            HostBackend::X11(placement) => {
                let previous = placement.overlay();
                let result = placement.apply_visual_role(scene, enabled).and_then(|_| {
                    if let Some(patch) = patch {
                        placement.apply_visual_role(patch, enabled)?;
                    }
                    Ok(())
                });
                if result.is_err() {
                    let _ = placement.apply_visual_role(scene, previous);
                    if let Some(patch) = patch {
                        let _ = placement.apply_visual_role(patch, previous);
                    }
                } else {
                    placement.set_overlay(enabled);
                }
                result
            }
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => {
                ensure!(!enabled, "MATE overlay is X11-only");
                Ok(())
            }
        }
    }
    pub fn new(monitor: u32) -> Result<Self> {
        Ok(Self {
            backend: HostBackend::X11(x11::DesktopPlacement::new(monitor)?),
        })
    }
    pub fn new_for_session(
        monitor: u32,
        experimental_wayland: bool,
        layer: crate::config::WaylandLayer,
    ) -> Result<Self> {
        if !experimental_wayland {
            return Self::new(monitor);
        }
        #[cfg(feature = "wayland-desktop")]
        {
            Ok(Self {
                backend: HostBackend::LayerShell(super::wayland::DesktopPlacement::new(
                    monitor, layer,
                )?),
            })
        }
        #[cfg(not(feature = "wayland-desktop"))]
        {
            let _ = layer;
            anyhow::bail!(
                "Experimental Wayland desktop requires a build with --features wayland-desktop"
            )
        }
    }
    pub fn kind(&self) -> BackendKind {
        match &self.backend {
            HostBackend::X11(_) => BackendKind::X11,
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => BackendKind::LayerShell,
        }
    }
    pub fn capabilities(&self) -> Capabilities {
        let mut capabilities = self.kind().capabilities();
        if self.kind() == BackendKind::LayerShell {
            capabilities.desktop_startup = Self::wayland_startup_session_supported();
        }
        capabilities
    }
    pub(crate) fn wayland_startup_session_supported() -> bool {
        Self::plasma_session()
    }
    /// Covered-window observations currently come only from the KWin adapter.
    /// Other layer-shell sessions may still throttle frames or report a lock.
    pub(crate) fn wayland_coverage_session_supported() -> bool {
        cfg!(feature = "wayland-desktop") && Self::plasma_session()
    }
    pub(crate) fn plasma_session() -> bool {
        plasma_session(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
    }
    pub fn description(&self) -> String {
        match &self.backend {
            HostBackend::X11(placement) => placement.description(),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(placement) => placement.description(),
        }
    }
    pub fn configure_scene(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        match &self.backend {
            HostBackend::X11(placement) => placement.configure(window),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(placement) => placement.configure(window),
        }
    }
    /// Startup and explicit desktop launches use the same actual-display check.
    pub(crate) fn check_startup() -> Result<()> {
        let display = gdk::Display::default().context("No GTK display is available")?;
        ensure!(
            check_display(&display)?.capabilities().desktop_startup,
            "Desktop startup is unavailable for this backend"
        );
        Ok(())
    }
    pub(crate) fn input_widget(&self) -> gtk::DrawingArea {
        match &self.backend {
            HostBackend::X11(_) => x11::ButtonInputPlacement::widget(),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => unreachable!("Wayland uses the scene input surface"),
        }
    }
    pub(crate) fn configure_input(
        &self,
        window: &gtk::ApplicationWindow,
    ) -> Result<SurfacePlacement> {
        ensure!(
            self.capabilities().selective_input,
            "Desktop input regions are unavailable"
        );
        match &self.backend {
            HostBackend::X11(_) => Ok(SurfacePlacement::from_x11(
                x11::ButtonInputPlacement::configure(window)?,
            )),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => anyhow::bail!("Wayland input uses the scene surface"),
        }
    }
    pub(crate) fn configure_animation(
        &self,
        window: &gtk::ApplicationWindow,
    ) -> Result<SurfacePlacement> {
        ensure!(
            self.capabilities().separate_animation_surface,
            "Separate animated surfaces are unavailable"
        );
        match &self.backend {
            HostBackend::X11(placement) => Ok(SurfacePlacement::from_x11(
                x11::ButtonInputPlacement::configure_visual_patch_with_overlay(
                    window,
                    placement.overlay(),
                )?,
            )),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => {
                anyhow::bail!("Separate Wayland animation surfaces are not implemented")
            }
        }
    }
    pub(crate) fn visibility_probe(&self) -> std::io::Result<visibility::Probe> {
        match &self.backend {
            HostBackend::X11(_) => visibility::Probe::start_x11(),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Wayland has no exact coverage probe",
            )),
        }
    }
    pub(crate) fn visibility_targets(
        &self,
        scene: &gtk::ApplicationWindow,
        patch: Option<&gtk::ApplicationWindow>,
        layout: visibility::Layout,
    ) -> Option<visibility::Targets> {
        match &self.backend {
            HostBackend::X11(_) => {
                let scene = x11::window_id(scene)?;
                let patch = match patch {
                    Some(window) => Some(x11::window_id(window)?),
                    None => None,
                };
                Some(visibility::Targets::x11(scene, patch, layout))
            }
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn configure_desktop_fixture(
        &self,
        window: &gtk::ApplicationWindow,
    ) -> Result<SurfacePlacement> {
        match &self.backend {
            HostBackend::X11(_) => Ok(SurfacePlacement::from_x11(
                x11::ButtonInputPlacement::configure_desktop_fixture(window)?,
            )),
            #[cfg(feature = "wayland-desktop")]
            HostBackend::LayerShell(_) => anyhow::bail!("X11 fixture unavailable on Wayland"),
        }
    }
}

pub(super) fn plasma_session(desktop: &str) -> bool {
    desktop
        .split(':')
        .any(|entry| entry.eq_ignore_ascii_case("KDE"))
}

fn mate_session(desktop: &str) -> bool {
    let tokens: Vec<_> = desktop.split(':').collect();
    tokens
        .iter()
        .any(|token| token.eq_ignore_ascii_case("MATE"))
        && !tokens.iter().any(|token| {
            token.eq_ignore_ascii_case("X-Cinnamon") || token.eq_ignore_ascii_case("Cinnamon")
        })
}

#[cfg(test)]
mod mate_tests {
    #[test]
    fn workaround_is_mate_only() {
        for name in ["MATE", "mate", "MATE:Other"] {
            assert!(super::mate_session(name));
        }
        for name in ["", "XFCE", "Cinnamon", "X-Cinnamon:MATE", "not-MATE"] {
            assert!(!super::mate_session(name));
        }
    }
}

/// Native placement details are hidden from transport and presentation. GTK
/// windows remain owned by their existing application/weak callback lifetimes.
#[derive(Clone)]
pub(crate) struct SurfacePlacement {
    backend: SurfaceBackend,
}
#[derive(Clone)]
enum SurfaceBackend {
    X11(x11::ButtonInputPlacement),
}
impl SurfacePlacement {
    fn from_x11(placement: x11::ButtonInputPlacement) -> Self {
        Self {
            backend: SurfaceBackend::X11(placement),
        }
    }
    pub(crate) fn can_receive(&self, window: &gtk::ApplicationWindow) -> Result<bool> {
        match &self.backend {
            SurfaceBackend::X11(placement) => placement.can_receive(window),
        }
    }
    pub(crate) fn position(
        &self,
        window: &gtk::ApplicationWindow,
        scene: &gtk::ApplicationWindow,
        rect: [i32; 4],
    ) -> Result<()> {
        self.position_if_changed(window, scene, rect).map(|_| ())
    }
    pub(crate) fn position_if_changed(
        &self,
        window: &gtk::ApplicationWindow,
        scene: &gtk::ApplicationWindow,
        rect: [i32; 4],
    ) -> Result<bool> {
        match &self.backend {
            SurfaceBackend::X11(placement) => placement.position_if_changed(window, scene, rect),
        }
    }
    pub(crate) fn stack_above_scene(
        &self,
        window: &gtk::ApplicationWindow,
        scene: &gtk::ApplicationWindow,
    ) -> Result<()> {
        match &self.backend {
            SurfaceBackend::X11(placement) => placement.stack_above_scene(window, scene),
        }
    }
    pub(crate) fn set_input_region(
        &self,
        window: &gtk::ApplicationWindow,
        region: &gtk::cairo::Region,
    ) -> Result<()> {
        let surface = window.surface().context("Missing desktop input surface")?;
        surface.set_input_region(region);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn nontransparent_pixels(&self, window: &gtk::ApplicationWindow) -> Result<usize> {
        match &self.backend {
            SurfaceBackend::X11(placement) => placement.nontransparent_pixels(window),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_requires_native_display_and_click_through() {
        assert_eq!(
            select_backend("GdkX11Display", Session::X11, true).unwrap(),
            BackendKind::X11
        );
        assert_eq!(
            select_backend("GdkX11Display", Session::Unknown, true).unwrap(),
            BackendKind::X11
        );
        assert!(select_backend("GdkWaylandDisplay", Session::Wayland, true).is_err());
        assert!(select_backend("GdkWaylandDisplay", Session::X11, true).is_err());
        assert!(select_backend("GdkX11Display", Session::Wayland, true).is_err());
        assert!(select_backend("GdkX11Display", Session::X11, false).is_err());
        assert!(select_backend("GdkBroadwayDisplay", Session::Unknown, true).is_err());
    }
    #[test]
    fn experimental_wayland_declares_only_implemented_capabilities() {
        let c = BackendKind::LayerShell.capabilities();
        assert!(c.selective_input);
        assert!(!c.separate_animation_surface && !c.visibility_probe && !c.desktop_startup);
    }
}
