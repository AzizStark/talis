//! Small runtime-loaded X11 boundary. Only Talis's own window is modified.
//! Xlib format-32 properties use native unsigned-long arrays, including on LP64.
use super::geometry::Rect;
use anyhow::{Context, Result, ensure};
use gtk::{gdk, glib::object::ObjectType, prelude::*};
use libloading::Library;
use std::{
    cell::Cell,
    ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void},
    rc::Rc,
};

type Display = *mut c_void;

/// GTK's cached EWMH window-manager name, for diagnostics and WM quirks.
/// Keep the native API inside this adapter and never invoke it on Wayland.
pub(crate) fn window_manager_name() -> Option<String> {
    let display = gdk::Display::default()?;
    if display.type_().name() != "GdkX11Display" {
        return None;
    }
    unsafe {
        let library = Library::new("libgtk-4.so.1").ok()?;
        let screen: libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> *mut c_void> =
            library.get(b"gdk_x11_display_get_screen\0").ok()?;
        let name: libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> *const c_char> = library
            .get(b"gdk_x11_screen_get_window_manager_name\0")
            .ok()?;
        let screen = screen(display.as_ptr().cast());
        if screen.is_null() {
            return None;
        }
        let name = name(screen);
        if name.is_null() {
            return None;
        }
        let value = CStr::from_ptr(name).to_string_lossy().trim().to_owned();
        (!value.is_empty() && !value.eq_ignore_ascii_case("unknown")).then_some(value)
    }
}
type Window = c_ulong;
type Atom = c_ulong;
type InternAtom = unsafe extern "C" fn(Display, *const c_char, c_int) -> Atom;
type ChangeProperty =
    unsafe extern "C" fn(Display, Window, Atom, Atom, c_int, c_int, *const u8, c_int) -> c_int;
type MoveResize = unsafe extern "C" fn(Display, Window, c_int, c_int, c_uint, c_uint) -> c_int;
// Synthetic managed ConfigureRequest: client XIDs can have different WM frame
// parents, so direct XConfigureWindow with a client sibling can cause BadMatch.
#[repr(C)]
struct ConfigureRequest {
    event_type: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: Display,
    parent: Window,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: Window,
    detail: c_int,
    value_mask: c_ulong,
}
type Lower = unsafe extern "C" fn(Display, Window) -> c_int;
type Flush = unsafe extern "C" fn(Display) -> c_int;
type ShapeRectangles =
    unsafe extern "C" fn(Display, Window, c_int, c_int, c_int, *const c_void, c_int, c_int, c_int);
type SetHints = unsafe extern "C" fn(Display, Window, *mut WmHints) -> c_int;
type GetHints = unsafe extern "C" fn(Display, Window) -> *mut WmHints;
type GetProtocols = unsafe extern "C" fn(Display, Window, *mut *mut Atom, *mut c_int) -> c_int;
type SetProtocols = unsafe extern "C" fn(Display, Window, *mut Atom, c_int) -> c_int;
type SetTransientFor = unsafe extern "C" fn(Display, Window, Window) -> c_int;
type Free = unsafe extern "C" fn(*mut c_void) -> c_int;
type GetDisplay = unsafe extern "C" fn(*mut c_void) -> Display;
type GetXid = unsafe extern "C" fn(*mut c_void) -> Window;
type RootWindow = unsafe extern "C" fn(Display) -> Window;
type Translate = unsafe extern "C" fn(
    Display,
    Window,
    Window,
    c_int,
    c_int,
    *mut c_int,
    *mut c_int,
    *mut Window,
) -> c_int;
type SendEvent = unsafe extern "C" fn(Display, Window, c_int, c_long, *mut XEvent) -> c_int;
type GetProperty = unsafe extern "C" fn(
    Display,
    Window,
    Atom,
    c_long,
    c_long,
    c_int,
    Atom,
    *mut Atom,
    *mut c_int,
    *mut c_ulong,
    *mut c_ulong,
    *mut *mut u8,
) -> c_int;

#[repr(C)]
#[derive(Clone, Copy)]
struct ClientMessage {
    event_type: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: Display,
    window: Window,
    message_type: Atom,
    format: c_int,
    data: [c_long; 5],
}
#[repr(C)]
union XEvent {
    client: ClientMessage,
    configure: std::mem::ManuallyDrop<ConfigureRequest>,
    pad: [c_long; 24],
}

#[repr(C)]
#[derive(Default)]
struct WmHints {
    flags: c_long,
    input: c_int,
    initial_state: c_int,
    icon_pixmap: c_ulong,
    icon_window: c_ulong,
    icon_x: c_int,
    icon_y: c_int,
    icon_mask: c_ulong,
    window_group: c_ulong,
}

struct X11 {
    // Keep both dispatch libraries and the borrowed native display alive.
    _gtk: Library,
    _xlib: Library,
    _shape: Library,
    display: gdk::Display,
    native: Display,
    get_xid: GetXid,
    intern: InternAtom,
    property: ChangeProperty,
    move_resize: MoveResize,
    lower: Lower,
    flush: Flush,
    shape_rectangles: ShapeRectangles,
    set_hints: SetHints,
    get_hints: GetHints,
    get_protocols: GetProtocols,
    set_protocols: SetProtocols,
    set_transient_for: SetTransientFor,
    free: Free,
    root: RootWindow,
    translate: Translate,
    send_event: SendEvent,
    get_property: GetProperty,
}

/// Resolve only our GTK-owned X11 surface; foreign-window queries use XCB.
pub(crate) fn window_id(window: &gtk::ApplicationWindow) -> Option<u32> {
    let display = gtk::prelude::WidgetExt::display(window);
    if display.type_().name() != "GdkX11Display" {
        return None;
    }
    thread_local! {
        // One GTK bridge lookup, not repeated loading of every placement symbol
        // for each visibility poll. All calls stay on the GTK thread.
        static BRIDGE: std::cell::OnceCell<Option<(Library, GetXid)>> = const { std::cell::OnceCell::new() };
    }
    let surface = window.surface()?;
    BRIDGE.with(|bridge| {
        let (_, get_xid) = bridge
            .get_or_init(|| unsafe {
                let library = Library::new("libgtk-4.so.1").ok()?;
                let function = *library.get(b"gdk_x11_surface_get_xid\0").ok()?;
                Some((library, function))
            })
            .as_ref()?;
        let xid = unsafe { get_xid(surface.as_ptr().cast()) };
        (xid != 0).then(|| u32::try_from(xid).ok()).flatten()
    })
}

impl X11 {
    fn load(display: gdk::Display) -> Result<Self> {
        // These APIs remain the native X11 bridge on GTK4; GTK 4.18 marks them
        // deprecated. Keep their use isolated, never use them on Wayland surfaces.
        unsafe {
            let gtk = Library::new("libgtk-4.so.1").context("Cannot load GTK X11 adapter")?;
            let xlib = Library::new("libX11.so.6").context("Desktop mode needs libX11.so.6")?;
            let shape = Library::new("libXext.so.6").context("Desktop mode needs libXext.so.6")?;
            let shape_rectangles = *shape.get(b"XShapeCombineRectangles\0")?;
            let get_display: GetDisplay = *gtk.get(b"gdk_x11_display_get_xdisplay\0")?;
            let get_xid = *gtk.get(b"gdk_x11_surface_get_xid\0")?;
            let native = get_display(display.as_ptr().cast());
            ensure!(!native.is_null(), "GTK did not provide an X11 display");
            Ok(Self {
                native,
                get_xid,
                display,
                intern: *xlib.get(b"XInternAtom\0")?,
                property: *xlib.get(b"XChangeProperty\0")?,
                move_resize: *xlib.get(b"XMoveResizeWindow\0")?,
                lower: *xlib.get(b"XLowerWindow\0")?,
                flush: *xlib.get(b"XFlush\0")?,
                set_hints: *xlib.get(b"XSetWMHints\0")?,
                get_hints: *xlib.get(b"XGetWMHints\0")?,
                get_protocols: *xlib.get(b"XGetWMProtocols\0")?,
                set_protocols: *xlib.get(b"XSetWMProtocols\0")?,
                set_transient_for: *xlib.get(b"XSetTransientForHint\0")?,
                free: *xlib.get(b"XFree\0")?,
                root: *xlib.get(b"XDefaultRootWindow\0")?,
                translate: *xlib.get(b"XTranslateCoordinates\0")?,
                send_event: *xlib.get(b"XSendEvent\0")?,
                get_property: *xlib.get(b"XGetWindowProperty\0")?,
                _gtk: gtk,
                _xlib: xlib,
                _shape: shape,
                shape_rectangles,
            })
        }
    }

    fn xid(&self, window: &gtk::ApplicationWindow) -> Result<Window> {
        let surface = window
            .surface()
            .context("Desktop scene has no native surface")?;
        ensure!(
            surface.display() == self.display,
            "Desktop surface changed its display"
        );
        let xid = unsafe { (self.get_xid)(surface.as_ptr().cast()) };
        ensure!(xid != 0, "Desktop scene has no X11 window ID");
        Ok(xid)
    }

    fn atom(&self, name: &CStr) -> Atom {
        unsafe { (self.intern)(self.native, name.as_ptr(), 0) }
    }

    fn property(&self, xid: Window, name: &CStr, kind: Atom, values: &[c_ulong]) {
        unsafe {
            (self.property)(
                self.native,
                xid,
                self.atom(name),
                kind,
                32,
                0,
                values.as_ptr().cast(),
                values.len() as c_int,
            );
        }
    }

    fn prepare(&self, window: &gtk::ApplicationWindow, role: &CStr) -> Result<()> {
        self.prepare_with_below(window, role, true)
    }

    fn prepare_with_below(
        &self,
        window: &gtk::ApplicationWindow,
        role: &CStr,
        below: bool,
    ) -> Result<()> {
        let xid = self.xid(window)?;
        self.property(xid, c"_NET_WM_WINDOW_TYPE", 4, &[self.atom(role)]);
        let states = [
            c"_NET_WM_STATE_BELOW",
            c"_NET_WM_STATE_STICKY",
            c"_NET_WM_STATE_SKIP_TASKBAR",
            c"_NET_WM_STATE_SKIP_PAGER",
        ]
        .map(|name| self.atom(name));
        self.property(
            xid,
            c"_NET_WM_STATE",
            4,
            if below { &states } else { &states[1..] },
        );
        self.property(xid, c"_NET_WM_DESKTOP", 6, &[u32::MAX as c_ulong]);
        self.property(xid, c"_NET_WM_USER_TIME", 6, &[0]);
        // Don't request fullscreen unredirect: this scene must compose below apps.
        self.property(xid, c"_NET_WM_BYPASS_COMPOSITOR", 6, &[2]);
        self.disable_focus(window)?;
        self.click_through(window)?;
        Ok(())
    }

    fn disable_focus(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        let xid = self.xid(window)?;
        // Widget focusability does not control native X11 focus. Preserve GTK's
        // group/state hints but opt out of both direct focus and WM_TAKE_FOCUS.
        unsafe {
            let current = (self.get_hints)(self.native, xid);
            if current.is_null() {
                let mut hints = WmHints {
                    flags: 1,
                    ..Default::default()
                };
                (self.set_hints)(self.native, xid, &mut hints);
            } else {
                (*current).flags |= 1;
                (*current).input = 0;
                (self.set_hints)(self.native, xid, current);
                (self.free)(current.cast());
            }
            let mut protocols = std::ptr::null_mut();
            let mut count = 0;
            if (self.get_protocols)(self.native, xid, &mut protocols, &mut count) != 0
                && !protocols.is_null()
            {
                let take_focus = self.atom(c"WM_TAKE_FOCUS");
                let mut keep: Vec<Atom> = std::slice::from_raw_parts(protocols, count as usize)
                    .iter()
                    .copied()
                    .filter(|atom| *atom != take_focus)
                    .collect();
                (self.set_protocols)(self.native, xid, keep.as_mut_ptr(), keep.len() as c_int);
                (self.free)(protocols.cast());
            }
            (self.flush)(self.native);
        }
        Ok(())
    }

    fn request_below(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        // GTK rewrites initial state when mapping. Once managed, EWMH requires
        // a root ClientMessage, not replacing the WM-owned state property.
        let mut event = XEvent { pad: [0; 24] };
        event.client = ClientMessage {
            event_type: 33,
            serial: 0,
            send_event: 1,
            display: self.native,
            window: self.xid(window)?,
            message_type: self.atom(c"_NET_WM_STATE"),
            format: 32,
            data: [1, self.atom(c"_NET_WM_STATE_BELOW") as c_long, 0, 1, 0],
        };
        unsafe {
            ensure!(
                (self.send_event)(
                    self.native,
                    (self.root)(self.native),
                    0,
                    (1 << 20) | (1 << 19),
                    &mut event
                ) != 0,
                "Cannot request below-applications input"
            );
            (self.flush)(self.native);
        }
        Ok(())
    }

    fn is_below(&self, window: &gtk::ApplicationWindow) -> Result<bool> {
        let xid = self.xid(window)?;
        let mut kind = 0;
        let mut format = 0;
        let mut count = 0;
        let mut remaining = 0;
        let mut data = std::ptr::null_mut();
        unsafe {
            let status = (self.get_property)(
                self.native,
                xid,
                self.atom(c"_NET_WM_STATE"),
                0,
                32,
                0,
                4,
                &mut kind,
                &mut format,
                &mut count,
                &mut remaining,
                &mut data,
            );
            let below = status == 0
                && kind == 4
                && format == 32
                && !data.is_null()
                && std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize)
                    .contains(&self.atom(c"_NET_WM_STATE_BELOW"));
            if !data.is_null() {
                (self.free)(data.cast());
            }
            Ok(below)
        }
    }

    fn click_through(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        window
            .surface()
            .context("Desktop scene has no surface")?
            .set_input_region(&gtk::cairo::Region::create());
        Ok(())
    }

    fn refresh_visual_input(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        const SHAPE_INPUT: c_int = 2;
        const SHAPE_SET: c_int = 0;
        const UNSORTED: c_int = 0;
        self.click_through(window)?;
        // Xfwm can create/resize its frame after the client's initial empty
        // shape. Reissue ShapeInput so it refreshes that frame too. GDK skips
        // equal regions, so another set_input_region(empty) alone is insufficient.
        // Modify only our client XID; never edit the WM's frame or desktop.
        unsafe {
            (self.shape_rectangles)(
                self.native,
                self.xid(window)?,
                SHAPE_INPUT,
                0,
                0,
                std::ptr::null(),
                0,
                SHAPE_SET,
                UNSORTED,
            );
            (self.flush)(self.native);
        }
        Ok(())
    }

    fn bind_visual_input(self: &Rc<Self>, window: &gtk::ApplicationWindow) -> Result<()> {
        let surface = window.surface().context("Missing visual desktop surface")?;
        let weak = window.downgrade();
        let backend = self.clone();
        let refresh: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(window) = weak.upgrade().filter(|w| w.is_mapped()) {
                if let Err(error) = backend.refresh_visual_input(&window) {
                    tracing::error!("Cannot keep desktop visual click-through: {error:#}");
                    window.set_visible(false);
                }
            }
        });
        let dimensions = Rc::new(Cell::new(None));
        surface.connect_layout({
            let dimensions = dimensions.clone();
            let refresh = refresh.clone();
            move |surface, width, height| {
                let size = (width, height, surface.scale_factor());
                if dimensions.replace(Some(size)) != Some(size) {
                    refresh();
                }
            }
        });
        surface
            .dynamic_cast::<gdk::Toplevel>()
            .map_err(|_| anyhow::anyhow!("Visual desktop surface is not a toplevel"))?
            .connect_notify_local(Some("state"), {
                let refresh = refresh.clone();
                move |_, _| refresh()
            });
        window.connect_map(move |_| {
            dimensions.set(None);
            let refresh = refresh.clone();
            gtk::glib::idle_add_local_once(move || refresh());
        });
        Ok(())
    }

    fn position(&self, window: &gtk::ApplicationWindow, monitor: &gdk::Monitor) -> Result<()> {
        let geometry = monitor.geometry();
        let (x, y, width, height) = physical_geometry(&geometry, monitor.scale_factor())?;
        let xid = self.xid(window)?;
        window.set_default_size(geometry.width(), geometry.height());
        // The beauty remains fully click-through. Button input uses a separate
        // small surface, so lowering the beauty cannot put its input below Nemo.
        unsafe {
            (self.move_resize)(self.native, xid, x, y, width, height);
            (self.lower)(self.native, xid);
            (self.flush)(self.native);
        }
        Ok(())
    }
}

/// A tiny transparent managed control surface: DOCK + BELOW, without struts.
/// EWMH/Muffin place it above desktop icons but below ordinary applications.
/// A DESKTOP receiver is insufficient: clicking Nemo can raise it over input.
#[derive(Clone)]
pub struct ButtonInputPlacement {
    backend: Rc<X11>,
    last_position: Rc<Cell<Option<InputPosition>>>,
    below_confirmed: Rc<Cell<bool>>,
}
type InputPosition = (Window, i32, i32, u32, u32);
fn visual_role(overlay: bool) -> &'static CStr {
    if overlay {
        c"_NET_WM_WINDOW_TYPE_DOCK"
    } else {
        c"_NET_WM_WINDOW_TYPE_DESKTOP"
    }
}

fn xfwm_desktop_stacking() -> bool {
    window_manager_name().is_some_and(|name| name.eq_ignore_ascii_case("Xfwm4"))
}
impl ButtonInputPlacement {
    /// Experimental opaque visual patch. Keep it immediately above its own
    /// desktop background. The MATE overlay intentionally covers desktop icons;
    /// the separate transport receiver remains responsible for selective input.
    pub(crate) fn configure_visual_patch_with_overlay(
        window: &gtk::ApplicationWindow,
        overlay: bool,
    ) -> Result<Self> {
        let placement = Self::configure(window)?;
        // Xfwm ignores relative stacking requests for DESKTOP windows. A
        // below-applications dock can stay above its own background instead.
        placement
            .backend
            .prepare(window, visual_role(overlay || xfwm_desktop_stacking()))?;
        placement.backend.bind_visual_input(window)?;
        Ok(placement)
    }
    pub(crate) fn stack_above_scene(
        &self,
        window: &gtk::ApplicationWindow,
        scene: &gtk::ApplicationWindow,
    ) -> Result<()> {
        let xid = self.backend.xid(window)?;
        let scene_xid = self.backend.xid(scene)?;
        if xfwm_desktop_stacking() {
            // Let the WM keep the pair ordered when the scene's post-map BELOW
            // acknowledgement moves it to another layer. Set the native hint:
            // GTK set_transient_for() would also change our role to DIALOG.
            unsafe {
                (self.backend.set_transient_for)(self.backend.native, xid, scene_xid);
            }
        }
        let mut event = XEvent { pad: [0; 24] };
        event.configure = std::mem::ManuallyDrop::new(ConfigureRequest {
            event_type: 23,
            serial: 0,
            send_event: 1,
            display: self.backend.native,
            parent: unsafe { (self.backend.root)(self.backend.native) },
            window: xid,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            border_width: 0,
            above: scene_xid,
            detail: 0,                       // Above
            value_mask: (1 << 5) | (1 << 6), // CWSibling | CWStackMode
        });
        unsafe {
            ensure!(
                (self.backend.send_event)(
                    self.backend.native,
                    (self.backend.root)(self.backend.native),
                    0,
                    (1 << 20) | (1 << 19),
                    &mut event
                ) != 0,
                "Cannot request native patch stacking"
            );
            (self.backend.flush)(self.backend.native);
        }
        Ok(())
    }
    pub fn widget() -> gtk::DrawingArea {
        let widget = gtk::DrawingArea::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_focusable(false);
        // An empty GTK snapshot may skip painting/clearing the native backing
        // buffer. A retained Cairo node guarantees a real transparent frame.
        widget.set_draw_func(|_, context, _, _| {
            context.set_operator(gtk::cairo::Operator::Source);
            context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            if let Err(error) = context.paint() {
                tracing::warn!("Cannot clear transport input surface: {error}");
            }
        });
        widget
    }
    pub fn configure(window: &gtk::ApplicationWindow) -> Result<Self> {
        Self::configure_with_role(window, c"_NET_WM_WINDOW_TYPE_DOCK", true)
    }

    fn configure_with_role(
        window: &gtk::ApplicationWindow,
        role: &CStr,
        below: bool,
    ) -> Result<Self> {
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_focusable(false);
        // Keep ordinary widget layout/picking. Visibility comes from RGBA
        // pixels, not widget opacity; input is independently shaped by GDK.
        // https://docs.gtk.org/gdk4/method.Surface.set_input_region.html
        window.add_css_class("talis-transport-input");
        let css = gtk::CssProvider::new();
        css.load_from_data("window.talis-transport-input { background: transparent; box-shadow: none; border: none; padding: 0; min-width: 0; min-height: 0; }");
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(window),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::prelude::WidgetExt::realize(window);
        let backend = Rc::new(X11::load(gtk::prelude::WidgetExt::display(window))?);
        backend.prepare_with_below(window, role, below)?;
        let last_position = Rc::new(Cell::new(None));
        let below_confirmed = Rc::new(Cell::new(false));
        window.connect_map({
            let backend = backend.clone();
            let last_position = last_position.clone();
            let below_confirmed = below_confirmed.clone();
            move |window| {
                last_position.set(None);
                below_confirmed.set(false);
                let weak = window.downgrade();
                let backend = backend.clone();
                gtk::glib::idle_add_local_once(move || {
                    if let Some(window) = weak.upgrade() {
                        if window.is_mapped() {
                            if let Err(error) = backend.disable_focus(&window).and_then(|_| {
                                if below {
                                    backend.request_below(&window)
                                } else {
                                    Ok(())
                                }
                            }) {
                                tracing::warn!("Cannot configure transport receiver: {error:#}");
                                window.set_visible(false);
                            }
                        }
                    }
                });
            }
        });
        Ok(Self {
            backend,
            last_position,
            below_confirmed,
        })
    }

    /// Fail closed until the WM acknowledges BELOW after mapping. Cache the
    /// acknowledgement until remapping: hover/timing never repeatedly reads it.
    pub fn can_receive(&self, window: &gtk::ApplicationWindow) -> Result<bool> {
        if !window.is_mapped() {
            return Ok(false);
        }
        if !self.below_confirmed.get() {
            self.below_confirmed.set(self.backend.is_below(window)?);
        }
        Ok(self.below_confirmed.get())
    }

    /// Test real native pixels; an empty WidgetPaintable is not evidence that
    /// GTK cleared the X11 buffer. Never inspect a root/foreign drawable here.
    #[cfg(test)]
    pub(crate) fn nontransparent_pixels(&self, window: &gtk::ApplicationWindow) -> Result<usize> {
        let surface = window.surface().context("Missing input surface")?;
        let scale = window.scale_factor();
        let (w, h) = (surface.width() * scale, surface.height() * scale);
        ensure!(w > 0 && h > 0, "Unallocated input surface");
        unsafe {
            let get_image = self.backend._xlib.get::<unsafe extern "C" fn(
                Display,
                Window,
                c_int,
                c_int,
                c_uint,
                c_uint,
                c_ulong,
                c_int,
            ) -> *mut c_void>(b"XGetImage\0")?;
            let get_pixel =
                self.backend
                    ._xlib
                    .get::<unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_ulong>(
                        b"XGetPixel\0",
                    )?;
            let destroy = self
                .backend
                ._xlib
                .get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"XDestroyImage\0")?;
            let image = get_image(
                self.backend.native,
                self.backend.xid(window)?,
                0,
                0,
                w as u32,
                h as u32,
                0xffff_ffff,
                2,
            );
            ensure!(!image.is_null(), "Cannot read input backing pixels");
            let mut count = 0;
            for y in 0..h {
                for x in 0..w {
                    if get_pixel(image, x, y) >> 24 & 255 != 0 {
                        count += 1;
                    }
                }
            }
            destroy(image);
            Ok(count)
        }
    }

    /// Model an icon manager in isolated tests, before mapping its own window.
    #[cfg(test)]
    pub(crate) fn configure_desktop_fixture(window: &gtk::ApplicationWindow) -> Result<Self> {
        // A real icon manager uses the desktop layer, not BELOW. Xfwm applies
        // BELOW before the window type, promoting a DESKTOP fixture into the
        // receiver's layer where clicking it can falsely hide the controls.
        Self::configure_with_role(window, c"_NET_WM_WINDOW_TYPE_DESKTOP", false)
    }
    /// Rectangle is relative to the scene window, in its GTK logical pixels.
    /// Queries/modifies only Talis windows; never changes Nemo or other clients.
    /// Report native placement changes without dirtying retained GL contents.
    pub(crate) fn position_if_changed(
        &self,
        window: &gtk::ApplicationWindow,
        scene: &gtk::ApplicationWindow,
        rect: [i32; 4],
    ) -> Result<bool> {
        let scale = scene.scale_factor();
        let geometry = gdk::Rectangle::new(rect[0], rect[1], rect[2], rect[3]);
        let (dx, dy, width, height) = physical_geometry(&geometry, scale)?;
        let source = self.backend.xid(scene)?;
        let target = self.backend.xid(window)?;
        let mut x = 0;
        let mut y = 0;
        let mut child = 0;
        unsafe {
            ensure!(
                (self.backend.translate)(
                    self.backend.native,
                    source,
                    (self.backend.root)(self.backend.native),
                    0,
                    0,
                    &mut x,
                    &mut y,
                    &mut child
                ) != 0,
                "Cannot locate transport scene"
            );
            let position = (
                target,
                x.checked_add(dx).context("Input x overflow")?,
                y.checked_add(dy).context("Input y overflow")?,
                width,
                height,
            );
            if self.last_position.get() == Some(position) {
                return Ok(false);
            }
            (self.backend.move_resize)(
                self.backend.native,
                target,
                position.1,
                position.2,
                width,
                height,
            );
            (self.backend.flush)(self.backend.native);
            self.last_position.set(Some(position));
        }
        Ok(true)
    }
}

fn physical_geometry(rect: &gdk::Rectangle, scale: i32) -> Result<(i32, i32, u32, u32)> {
    let physical = Rect {
        x: rect.x(),
        y: rect.y(),
        w: rect.width(),
        h: rect.height(),
    }
    .scaled(scale)?;
    Ok((physical.x, physical.y, physical.w as u32, physical.h as u32))
}

/// GTK-rs 0.10.3's compute-size trampoline treats the pointed-to native struct
/// as a Rust pointer wrapper. Use the native pointer directly at this boundary.
/// GDK owns the size; the signal connection owns/frees the weak monitor reference.
fn full_monitor_size(toplevel: &gdk::Toplevel, monitor: &gdk::Monitor) {
    unsafe extern "C" fn compute(
        _: *mut gdk::ffi::GdkToplevel,
        size: *mut gdk::ffi::GdkToplevelSize,
        data: gtk::glib::ffi::gpointer,
    ) {
        unsafe {
            let monitor = &*data.cast::<gtk::glib::WeakRef<gdk::Monitor>>();
            if let Some(monitor) = monitor.upgrade() {
                let g = monitor.geometry();
                if g.width() > 0 && g.height() > 0 {
                    gdk::ffi::gdk_toplevel_size_set_size(size, g.width(), g.height());
                    gdk::ffi::gdk_toplevel_size_set_min_size(size, g.width(), g.height());
                    gdk::ffi::gdk_toplevel_size_set_shadow_width(size, 0, 0, 0, 0);
                }
            }
        }
    }
    unsafe {
        gtk::glib::signal::connect_raw(
            toplevel.as_ptr().cast(),
            c"compute-size".as_ptr(),
            Some(std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut gdk::ffi::GdkToplevel,
                    *mut gdk::ffi::GdkToplevelSize,
                    gtk::glib::ffi::gpointer,
                ),
                unsafe extern "C" fn(),
            >(compute)),
            Box::into_raw(Box::new(monitor.downgrade())),
        );
    }
}

/// An explicit X11 desktop surface, not a fullscreen normal application window.
#[derive(Clone)]
pub struct DesktopPlacement {
    backend: Rc<X11>,
    monitor: gdk::Monitor,
    index: u32,
    overlay: Rc<Cell<bool>>,
}
impl DesktopPlacement {
    /// Call on the GTK main thread after GTK initialization.
    pub fn new(index: u32) -> Result<Self> {
        let display = gdk::Display::default().context("No GTK display is available")?;
        super::host::check_display(&display)?;
        let monitors = display.monitors();
        let monitor = monitors
            .item(index)
            .and_then(|item| item.downcast::<gdk::Monitor>().ok())
            .with_context(|| {
                format!(
                    "Monitor {index} does not exist; available indices: 0..{}",
                    monitors.n_items().saturating_sub(1)
                )
            })?;
        Ok(Self {
            backend: Rc::new(X11::load(display)?),
            monitor,
            index,
            overlay: Rc::new(Cell::new(false)),
        })
    }

    pub fn description(&self) -> String {
        let g = self.monitor.geometry();
        format!(
            "X11 desktop · monitor {} · {}×{} logical pixels at {},{} · scale {} · click-through{}",
            self.index,
            g.width(),
            g.height(),
            g.x(),
            g.y(),
            self.monitor.scale_factor(),
            if self.overlay.get() {
                " · experimental MATE overlay (icons covered)"
            } else {
                ""
            }
        )
    }

    pub(crate) fn overlay(&self) -> bool {
        self.overlay.get()
    }

    pub(crate) fn set_overlay(&self, enabled: bool) {
        self.overlay.set(enabled);
    }

    pub(crate) fn apply_visual_role(
        &self,
        window: &gtk::ApplicationWindow,
        enabled: bool,
    ) -> Result<()> {
        self.backend.prepare(window, visual_role(enabled))?;
        if window.is_mapped() {
            self.backend.request_below(window)?;
        }
        Ok(())
    }

    /// Configure while unmapped, before GTK shows the scene. Never steals focus.
    pub fn configure(&self, window: &gtk::ApplicationWindow) -> Result<()> {
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_focusable(false);
        let geometry = self.monitor.geometry();
        window.set_default_size(geometry.width(), geometry.height());
        gtk::prelude::WidgetExt::realize(window);
        let toplevel = window
            .surface()
            .context("Desktop scene has no native surface")?
            .dynamic_cast::<gdk::Toplevel>()
            .map_err(|_| anyhow::anyhow!("Desktop scene is not a native toplevel"))?;
        // GTK's ordinary window handler clamps its default size to the work area
        // (monitor minus panels). This handler is connected after GTK's handler:
        // a desktop scene needs the full monitor, with panels stacked above it.
        // Override size negotiation rather than fighting ConfigureNotify with a
        // repeating XMoveResizeWindow loop or requesting fullscreen activation.
        full_monitor_size(&toplevel, &self.monitor);
        self.backend
            .prepare(window, visual_role(self.overlay.get()))?;
        self.backend.position(window, &self.monitor)?;
        self.backend.bind_visual_input(window)?;
        let placement = self.clone();
        window.connect_map(move |window| {
            if let Err(error) = placement.backend.position(window, &placement.monitor) {
                tracing::error!("Cannot position desktop scene: {error:#}");
                if let Some(app) = window.application() {
                    app.quit();
                }
            }
            // GDK rewrites WM_HINTS/protocols as it maps. Reapply after that
            // work, with a weak window so shutdown cannot keep the surface alive.
            let weak = window.downgrade();
            let backend = placement.backend.clone();
            let below = placement.overlay.get() || xfwm_desktop_stacking();
            gtk::glib::idle_add_local_once(move || {
                if let Some(window) = weak.upgrade() {
                    if window.is_mapped() {
                        // The scene and animated patch must retain the same
                        // state after GTK maps them. Otherwise Xfwm can raise
                        // its desktop between them on a click, exposing only
                        // the small patch over the original wallpaper.
                        if let Err(error) = backend.disable_focus(&window).and_then(|_| {
                            if below {
                                backend.request_below(&window)
                            } else {
                                Ok(())
                            }
                        }) {
                            tracing::error!("Cannot configure desktop scene: {error:#}");
                            if let Some(app) = window.application() {
                                app.quit();
                            }
                        }
                    }
                }
            });
        });
        for property in ["geometry", "scale-factor"] {
            let weak = window.downgrade();
            let backend = Rc::downgrade(&self.backend);
            self.monitor
                .connect_notify_local(Some(property), move |monitor, _| {
                    if let (Some(window), Some(backend)) = (weak.upgrade(), backend.upgrade()) {
                        if let Err(error) = backend.position(&window, monitor) {
                            tracing::error!("Desktop monitor changed: {error:#}");
                            if let Some(app) = window.application() {
                                app.quit();
                            }
                        }
                    }
                });
        }
        let weak = window.downgrade();
        self.monitor.connect_invalidate(move |_| {
            tracing::info!("Desktop monitor disconnected; stopping Talis");
            if let Some(window) = weak.upgrade() {
                if let Some(app) = window.application() {
                    app.quit();
                }
            }
        });
        tracing::info!(placement=%self.description(), "Configured native desktop scene; wallpaper settings unchanged");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
