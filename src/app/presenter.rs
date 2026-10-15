//! GTK-owned static background plus a pixel-aligned animated patch.
//!
//! The full-size GLArea remains the logical scene/input/context owner. Its
//! snapshot delegates to two native GLAreas, so GTK owns texture lifetimes and
//! fences; no exported texture is modified while GTK might still be using it.
use super::lifecycle::SharedRenderer;
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Weak,
};

pub(super) mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RegionalArea {
        pub background: OnceCell<gtk::GLArea>,
        pub animated: OnceCell<gtk::GLArea>,
        pub renderer: RefCell<Weak<RefCell<Option<crate::rendering::Renderer>>>>,
        pub region: Cell<Option<[i32; 4]>>,
        pub dimensions: Cell<(i32, i32)>,
        pub failed: Cell<bool>,
        pub(in crate::app) native: RefCell<Option<super::super::native_patch::NativePatch>>,
        #[cfg(test)]
        pub background_draws: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for RegionalArea {
        const NAME: &'static str = "TalisRegionalArea";
        type Type = super::RegionalArea;
        type ParentType = gtk::GLArea;
    }
    impl ObjectImpl for RegionalArea {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            for slot in [&self.background, &self.animated] {
                let child = gtk::GLArea::builder()
                    .auto_render(false)
                    .has_depth_buffer(false)
                    .can_target(false)
                    .build();
                child.set_required_version(3, 3);
                child.set_use_es(false);
                let owner = obj.downgrade();
                child.connect_create_context(move |_| owner.upgrade().and_then(|o| o.context()));
                child.set_parent(&*obj);
                slot.set(child).expect("One presentation child");
            }
            let owner = obj.downgrade();
            self.animated
                .get()
                .unwrap()
                .connect_render(move |_, context| {
                    if let Some(owner) = owner.upgrade() {
                        if !super::super::visibility::render_pending(owner.upcast_ref()) {
                            return glib::Propagation::Stop;
                        }
                        if let Some(native) = owner.imp().native.borrow().as_ref() {
                            if !native.prepare(&owner) {
                                return glib::Propagation::Stop;
                            }
                        }
                        owner.emit_by_name::<bool>("render", &[context]);
                    }
                    glib::Propagation::Stop
                });
            let owner = obj.downgrade();
            self.background.get().unwrap().connect_render(move |_, _| {
                if let Some(owner) = owner.upgrade() {
                    if !super::super::visibility::begin_background(owner.upcast_ref()) {
                        return glib::Propagation::Stop;
                    }
                    #[cfg(test)]
                    owner
                        .imp()
                        .background_draws
                        .set(owner.imp().background_draws.get() + 1);
                    if let Some(shared) = owner.imp().renderer.borrow().upgrade() {
                        if let Some(renderer) = shared.borrow().as_ref() {
                            unsafe {
                                renderer.present_background();
                            }
                        }
                    }
                }
                glib::Propagation::Stop
            });
        }
        fn dispose(&self) {
            if let Some(native) = self.native.borrow_mut().take() {
                native.window.destroy();
            }
            for child in [&self.background, &self.animated] {
                if let Some(child) = child.get() {
                    if child.parent().is_some() {
                        child.unparent();
                    }
                }
            }
        }
    }
    impl WidgetImpl for RegionalArea {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.background
                .get()
                .unwrap()
                .allocate(width, height, baseline, None);
            let scale = self.obj().scale_factor();
            let physical = (width * scale, height * scale);
            if self.dimensions.replace(physical) != physical {
                super::super::visibility::invalidate_layout(self.obj().upcast_ref());
                if !super::super::visibility::suspended(self.obj().upcast_ref()) {
                    self.background.get().unwrap().queue_render();
                }
                // Native GLArea normally emits this in its snapshot. Retain
                // global dimensions for transport hit regions and controls.
                self.obj()
                    .emit_by_name::<()>("resize", &[&physical.0, &physical.1]);
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let background = self.background.get().unwrap();
            let animated = self.animated.get().unwrap();
            if !super::super::visibility::render_pending(obj.upcast_ref()) {
                // Retain GTK's existing nodes; do not prepare or dirty a native
                // patch while hidden. A resize gets one fresh render on resume.
                obj.snapshot_child(background, snapshot);
                if self.native.borrow().is_none() {
                    obj.snapshot_child(animated, snapshot);
                }
                return;
            }
            let scale = obj.scale_factor();
            let (width, height) = (obj.width() * scale, obj.height() * scale);
            let region = self.renderer.borrow().upgrade().and_then(|r| {
                r.borrow()
                    .as_ref()
                    .and_then(|r| r.regional_rect(width, height, scale))
            });
            if self.failed.get() || obj.error().is_some() || region.is_none() {
                if let Some(native) = self.native.borrow().as_ref() {
                    native.window.set_visible(false);
                }
                self.region.set(None);
                self.parent_snapshot(snapshot);
                return;
            }
            let region = region.unwrap();
            self.region.set(Some(region));
            if let Some(native) = self.native.borrow().as_ref() {
                if native.prepare(&obj) {
                    animated.queue_render();
                }
                obj.snapshot_child(background, snapshot);
                return;
            }
            let [x, y, w, h] = region;
            let translation = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(
                (x / scale) as f32,
                ((height - y - h) / scale) as f32,
            ));
            animated.allocate(w / scale, h / scale, -1, Some(translation));
            // Render the animated patch first: this refreshes the retained cache
            // and requests a background refresh only for static asset changes.
            animated.queue_render();
            let patch = gtk::Snapshot::new();
            obj.snapshot_child(animated, &patch);
            if obj.error().is_some() || background.error().is_some() || animated.error().is_some() {
                self.failed.set(true);
                self.region.set(None);
                tracing::warn!("Regional GTK presentation unavailable; using full-frame fallback");
                self.parent_snapshot(snapshot);
                return;
            }
            obj.snapshot_child(background, snapshot);
            if let Some(node) = patch.to_node() {
                snapshot.append_node(&node);
            }
        }
    }
    impl GLAreaImpl for RegionalArea {}
}
glib::wrapper! {
    pub struct RegionalArea(ObjectSubclass<imp::RegionalArea>)
        @extends gtk::GLArea, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

pub(super) fn new() -> gtk::GLArea {
    glib::Object::builder::<RegionalArea>()
        .property("hexpand", true)
        .property("vexpand", true)
        .property("auto-render", false)
        .property("has-depth-buffer", false)
        .build()
        .upcast()
}
/// Route animation requests to the native patch without invalidating the static
/// toplevel. Initial layout and static transitions still snapshot the owner.
pub(super) fn queue_render(area: &gtk::GLArea) {
    if super::visibility::suspended(area) {
        return;
    }
    queue_raw(area);
}
/// Discrete content changes may refresh visible stationary artwork while the
/// animated patch is covered. Animation timers must use queue_render instead.
pub(super) fn queue_content_render(area: &gtk::GLArea) {
    if super::visibility::request_content(area) {
        queue_raw(area);
    }
}
pub(super) fn queue_layout_render(area: &gtk::GLArea) {
    super::visibility::invalidate_layout(area);
    if super::visibility::request_content(area) {
        area.queue_render();
    }
}
fn queue_raw(area: &gtk::GLArea) {
    if let Some(owner) = area.downcast_ref::<RegionalArea>() {
        if owner.imp().native.borrow().is_some()
            && !owner.imp().failed.get()
            && owner.imp().region.get().is_some()
        {
            owner.imp().animated.get().unwrap().queue_render();
            return;
        }
    }
    area.queue_render();
}
pub(super) fn bind(area: &gtk::GLArea, renderer: &SharedRenderer) {
    if let Some(area) = area.downcast_ref::<RegionalArea>() {
        *area.imp().renderer.borrow_mut() = std::rc::Rc::downgrade(renderer);
    }
}
pub(super) fn region(area: &gtk::GLArea) -> Option<[i32; 4]> {
    area.downcast_ref::<RegionalArea>()
        .and_then(|a| a.imp().region.get())
}
pub(super) fn refresh_background(area: &gtk::GLArea) {
    if !super::visibility::request_background(area) {
        return;
    }
    if let Some(area) = area.downcast_ref::<RegionalArea>() {
        area.imp().background.get().unwrap().queue_render();
    }
}
pub(super) fn native_window(area: &gtk::GLArea) -> Option<gtk::ApplicationWindow> {
    area.downcast_ref::<RegionalArea>().and_then(|area| {
        area.imp()
            .native
            .borrow()
            .as_ref()
            .map(|native| native.window.clone())
    })
}
/// The visibility worker already reads root coordinates. Repair native placement
/// only when those coordinates change; moving retained pixels needs no GL work.
pub(super) fn relocate_native(area: &gtk::GLArea, origin: [i32; 2]) {
    if let Some(owner) = area.downcast_ref::<RegionalArea>() {
        if let Some(native) = owner.imp().native.borrow().as_ref() {
            native.relocate(owner, origin);
        }
    }
}
/// No GL work: confirm current renderer/layout agree before publishing expected
/// physical top-left bounds to the desktop visibility provider.
pub(super) fn native_coverage_rect(area: &gtk::GLArea) -> Option<crate::desktop::visibility::Rect> {
    let owner = area.downcast_ref::<RegionalArea>()?;
    let imp = owner.imp();
    if imp.failed.get() || !imp.native.borrow().as_ref()?.window.is_mapped() {
        return None;
    }
    let scale = area.scale_factor();
    let (width, height) = (area.width() * scale, area.height() * scale);
    let region = imp
        .renderer
        .borrow()
        .upgrade()?
        .borrow()
        .as_ref()?
        .regional_rect(width, height, scale)?;
    if imp.region.get() != Some(region) {
        return None;
    }
    crate::desktop::geometry::Rect::from_gl(region, height)
}

#[cfg(test)]
mod tests;
