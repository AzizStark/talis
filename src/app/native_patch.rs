//! Opt-in animated surface, placed by the desktop host. Coordinates remain full-scene.
use super::presenter::RegionalArea;
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::Cell;

pub(super) struct NativePatch {
    pub window: gtk::ApplicationWindow,
    scene: glib::WeakRef<gtk::ApplicationWindow>,
    placement: crate::desktop::SurfacePlacement,
    positioned: Cell<Option<([i32; 4], i32)>>,
    scene_origin: Cell<Option<[i32; 2]>>,
}

impl NativePatch {
    pub(super) fn relocate(&self, owner: &RegionalArea, origin: [i32; 2]) {
        let imp = owner.imp();
        let Some((rect, scale)) = self.positioned.get() else {
            return;
        };
        let Some(scene) = self.scene.upgrade() else {
            return;
        };
        if imp.failed.get()
            || self.scene_origin.get() == Some(origin)
            || !scene.is_mapped()
            || !self.window.is_mapped()
            || scale != scene.scale_factor()
            || scale != self.window.scale_factor()
        {
            return;
        }
        match self
            .placement
            .position_if_changed(&self.window, &scene, rect)
        {
            Ok(moved) => {
                self.scene_origin.set(Some(origin));
                if moved {
                    super::visibility::invalidate_layout(owner.upcast_ref());
                    tracing::debug!(?origin, "Native animated patch relocated");
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Native patch relocation failed; using full-frame fallback: {error:#}"
                );
                imp.failed.set(true);
                imp.region.set(None);
                self.window.set_visible(false);
                super::presenter::queue_layout_render(owner.upcast_ref());
            }
        }
    }
    pub(super) fn prepare(&self, owner: &RegionalArea) -> bool {
        let imp = owner.imp();
        if imp.failed.get() {
            return false;
        }
        let scale = owner.scale_factor();
        let (w, h) = (owner.width() * scale, owner.height() * scale);
        let region = imp.renderer.borrow().upgrade().and_then(|r| {
            r.borrow()
                .as_ref()
                .and_then(|r| r.regional_rect(w, h, scale))
        });
        let result = (|| -> anyhow::Result<bool> {
            let region = region.ok_or_else(|| anyhow::anyhow!("No safe native patch bounds"))?;
            let rect = crate::desktop::geometry::logical_patch(region, h, scale)
                .ok_or_else(|| anyhow::anyhow!("Unaligned native patch bounds"))?;
            let scene = self
                .scene
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("Scene closed"))?;
            let animated = imp.animated.get().unwrap();
            if animated.error().is_some()
                || (self.window.is_mapped() && self.window.scale_factor() != scale)
            {
                anyhow::bail!("Native patch context/scale incompatible with scene");
            }
            imp.region.set(Some(region));
            if self.positioned.get() != Some((rect, scale)) {
                self.window.set_size_request(rect[2], rect[3]);
                self.window.set_default_size(rect[2], rect[3]);
                self.placement.position(&self.window, &scene, rect)?;
                self.positioned.set(Some((rect, scale)));
                self.scene_origin.set(None);
                tracing::info!(?region, ?rect, "Native animated patch positioned");
            }
            if !self.window.is_visible() {
                self.window.set_visible(true);
            }
            // Allocation is asynchronous. Never render into a wrongly sized FBO.
            if animated.width() * scale != region[2] || animated.height() * scale != region[3] {
                animated.queue_render();
                return Ok(false);
            }
            Ok(true)
        })();
        match result {
            Ok(ready) => ready,
            Err(error) => {
                tracing::warn!("Native patch unavailable; using full-frame fallback: {error:#}");
                imp.failed.set(true);
                imp.region.set(None);
                self.window.set_visible(false);
                super::presenter::queue_layout_render(owner.upcast_ref());
                false
            }
        }
    }
}

pub(super) fn install(
    area: &gtk::GLArea,
    scene: &gtk::ApplicationWindow,
    host: &crate::desktop::DesktopHost,
) -> anyhow::Result<()> {
    let owner = area
        .downcast_ref::<RegionalArea>()
        .ok_or_else(|| anyhow::anyhow!("Native patch requires regional scene owner"))?;
    let app = scene
        .application()
        .ok_or_else(|| anyhow::anyhow!("Scene needs application"))?;
    let window = gtk::ApplicationWindow::builder()
        .application(&app)
        .title("Talis · Animated patch")
        .default_width(1)
        .default_height(1)
        .build();
    let placement = match host.configure_animation(&window) {
        Ok(value) => value,
        Err(error) => {
            window.destroy();
            return Err(error);
        }
    };
    let animated = owner.imp().animated.get().unwrap();
    animated.unparent();
    window.set_child(Some(animated));
    let weak = owner.downgrade();
    let scene_weak = scene.downgrade();
    let map_placement = placement.clone();
    window.connect_map(move |patch| {
        let (weak, scene_weak, placement, patch) = (
            weak.clone(),
            scene_weak.clone(),
            map_placement.clone(),
            patch.downgrade(),
        );
        glib::idle_add_local_once(move || {
            if let (Some(owner), Some(scene), Some(patch)) =
                (weak.upgrade(), scene_weak.upgrade(), patch.upgrade())
            {
                if !scene.is_mapped() || !patch.is_mapped() {
                    return;
                }
                if let Err(error) = placement.stack_above_scene(&patch, &scene) {
                    tracing::warn!("Cannot stack native patch: {error:#}");
                    owner.imp().failed.set(true);
                    patch.set_visible(false);
                }
                if let Some(native) = owner.imp().native.borrow().as_ref() {
                    native.positioned.set(None);
                }
                super::presenter::queue_render(owner.upcast_ref());
            }
        });
    });
    let weak = window.downgrade();
    scene.connect_unrealize(move |_| {
        if let Some(patch) = weak.upgrade() {
            patch.destroy();
        }
    });
    let weak = owner.downgrade();
    window.connect_close_request(move |patch| {
        patch.set_visible(false);
        if let Some(owner) = weak.upgrade() {
            owner.imp().failed.set(true);
            owner.imp().region.set(None);
            super::presenter::queue_layout_render(owner.upcast_ref());
        }
        glib::Propagation::Stop
    });
    *owner.imp().native.borrow_mut() = Some(NativePatch {
        window,
        scene: scene.downgrade(),
        placement,
        positioned: Cell::new(None),
        scene_origin: Cell::new(None),
    });
    Ok(())
}
