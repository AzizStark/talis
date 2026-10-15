//! Small runtime XCB boundary. Checked replies tolerate disappearing clients
//! without installing a process-global Xlib error handler in GTK.
use super::{Hidden, NativeTargets, Rect, Targets, coverage_state, covered};
use anyhow::{Context, Result, bail, ensure};
use libloading::Library;
use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void},
    ptr,
};

type Connection = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Cookie {
    sequence: u32,
}
#[repr(C)]
struct ScreenIterator {
    data: *const u8,
    rem: c_int,
    index: c_int,
}
type Connect = unsafe extern "C" fn(*const c_char, *mut c_int) -> Connection;
type Disconnect = unsafe extern "C" fn(Connection);
type ConnectionError = unsafe extern "C" fn(Connection) -> c_int;
type Setup = unsafe extern "C" fn(Connection) -> *const c_void;
type Roots = unsafe extern "C" fn(*const c_void) -> ScreenIterator;
type Next = unsafe extern "C" fn(*mut ScreenIterator);
type Intern = unsafe extern "C" fn(Connection, u8, u16, *const c_char) -> Cookie;
type Property = unsafe extern "C" fn(Connection, u8, u32, u32, u32, u32, u32) -> Cookie;
type WindowQuery = unsafe extern "C" fn(Connection, u32) -> Cookie;
type Translate = unsafe extern "C" fn(Connection, u32, u32, i16, i16) -> Cookie;
type Extension = unsafe extern "C" fn(Connection, u16, *const c_char) -> Cookie;
type Wait = unsafe extern "C" fn(Connection, u32, *mut *mut c_void) -> *mut u8;
#[repr(C)]
struct ExtensionId {
    name: *const c_char,
    global_id: c_int,
}
#[repr(C)]
struct ProtocolRequest {
    count: usize,
    ext: *mut ExtensionId,
    opcode: u8,
    is_void: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}
type Send = unsafe extern "C" fn(Connection, c_int, *mut IoVec, *const ProtocolRequest) -> u32;

pub(super) struct Observer {
    pub(super) scene_origin: Option<[i32; 2]>,
    connection: Connection,
    root: u32,
    atoms: HashMap<&'static str, u32>,
    disconnect: Disconnect,
    error: ConnectionError,
    intern: Intern,
    property: Property,
    geometry: WindowQuery,
    attributes: WindowQuery,
    translate: Translate,
    wait: Wait,
    shape_present: bool,
    shape: Option<WindowQuery>,
    dpms: Option<Box<ExtensionId>>,
    send: Send,
    _shape_library: Option<Library>,
    _library: Library,
}
impl Observer {
    pub(super) fn new() -> Result<Self> {
        unsafe {
            let library = Library::new("libxcb.so.1")?;
            let connect: Connect = *library.get(b"xcb_connect\0")?;
            let disconnect = *library.get(b"xcb_disconnect\0")?;
            let error: ConnectionError = *library.get(b"xcb_connection_has_error\0")?;
            let setup: Setup = *library.get(b"xcb_get_setup\0")?;
            let roots: Roots = *library.get(b"xcb_setup_roots_iterator\0")?;
            let next: Next = *library.get(b"xcb_screen_next\0")?;
            // Resolve dispatch before allocating the connection (no failure leak).
            let intern = *library.get(b"xcb_intern_atom\0")?;
            let property = *library.get(b"xcb_get_property\0")?;
            let geometry = *library.get(b"xcb_get_geometry\0")?;
            let attributes = *library.get(b"xcb_get_window_attributes\0")?;
            let translate = *library.get(b"xcb_translate_coordinates\0")?;
            let wait = *library.get(b"xcb_wait_for_reply\0")?;
            let extension: Extension = *library.get(b"xcb_query_extension\0")?;
            let send = *library.get(b"xcb_send_request\0")?;
            let mut screen = 0;
            let connection = connect(ptr::null(), &mut screen);
            ensure!(!connection.is_null(), "No XCB connection");
            let mut observer = Self {
                scene_origin: None,
                connection,
                root: 0,
                atoms: HashMap::new(),
                disconnect,
                error,
                intern,
                property,
                geometry,
                attributes,
                translate,
                wait,
                shape_present: false,
                shape: None,
                dpms: None,
                send,
                _shape_library: None,
                _library: library,
            };
            ensure!(error(connection) == 0, "XCB connection unavailable");
            let mut iter = roots(setup(connection));
            for _ in 0..screen {
                next(&mut iter);
            }
            ensure!(iter.rem > 0 && !iter.data.is_null(), "No X11 screen");
            observer.root = ptr::read_unaligned(iter.data.cast::<u32>());
            let reply = observer.reply(extension(connection, 5, c"SHAPE".as_ptr()))?;
            observer.shape_present = reply[8] != 0;
            if observer.reply(extension(connection, 4, c"DPMS".as_ptr()))?[8] != 0 {
                observer.dpms = Some(Box::new(ExtensionId {
                    name: c"DPMS".as_ptr(),
                    global_id: 0,
                }));
            }
            if observer.shape_present {
                if let Ok(lib) = Library::new("libxcb-shape.so.0") {
                    observer.shape = lib.get(b"xcb_shape_query_extents\0").ok().map(|s| *s);
                    observer._shape_library = Some(lib);
                }
            }
            Ok(observer)
        }
    }
    fn reply(&self, cookie: Cookie) -> Result<Vec<u8>> {
        unsafe {
            let mut error = ptr::null_mut();
            let reply = (self.wait)(self.connection, cookie.sequence, &mut error);
            if !error.is_null() || reply.is_null() {
                gtk::glib::ffi::g_free(error);
                gtk::glib::ffi::g_free(reply.cast());
                bail!("X11 query failed or client disappeared");
            }
            let words = ptr::read_unaligned(reply.add(4).cast::<u32>()) as usize;
            if words > 4096 {
                gtk::glib::ffi::g_free(reply.cast());
                bail!("Oversized X11 reply");
            }
            let data = std::slice::from_raw_parts(reply, 32 + words * 4).to_vec();
            gtk::glib::ffi::g_free(reply.cast());
            Ok(data)
        }
    }
    fn atom(&mut self, name: &'static str) -> Result<u32> {
        if let Some(atom) = self.atoms.get(name) {
            return Ok(*atom);
        }
        let reply = self.reply(unsafe {
            (self.intern)(self.connection, 0, name.len() as u16, name.as_ptr().cast())
        })?;
        let atom = word(&reply, 8);
        self.atoms.insert(name, atom);
        Ok(atom)
    }
    fn property(&mut self, window: u32, name: &'static str, kind: u32) -> Result<Vec<u32>> {
        let atom = self.atom(name)?;
        let reply = self
            .reply(unsafe { (self.property)(self.connection, 0, window, atom, kind, 0, 1024) })?;
        if word(&reply, 8) == 0 {
            return Ok(vec![]);
        }
        ensure!(
            reply[1] == 32 && word(&reply, 8) == kind && word(&reply, 12) == 0,
            "Unsupported or oversized X11 property"
        );
        let count = word(&reply, 16) as usize;
        ensure!(
            count <= 1024 && 32 + count * 4 <= reply.len(),
            "Invalid X11 property"
        );
        Ok((0..count).map(|i| word(&reply, 32 + i * 4)).collect())
    }
    fn rect(&self, window: u32) -> Result<(Rect, u8)> {
        let g = self.reply(unsafe { (self.geometry)(self.connection, window) })?;
        let t =
            self.reply(unsafe { (self.translate)(self.connection, window, self.root, 0, 0) })?;
        ensure!(t[1] != 0, "Client on another X11 screen");
        Ok((
            Rect {
                x: i16::from_ne_bytes(t[12..14].try_into().unwrap()) as i32,
                y: i16::from_ne_bytes(t[14..16].try_into().unwrap()) as i32,
                w: u16::from_ne_bytes(g[16..18].try_into().unwrap()) as i32,
                h: u16::from_ne_bytes(g[18..20].try_into().unwrap()) as i32,
            },
            g[1],
        ))
    }
    fn opaque(
        &mut self,
        window: u32,
        desktop: u32,
        hidden_atom: u32,
        target: Rect,
    ) -> Result<Vec<Rect>> {
        let a = self.reply(unsafe { (self.attributes)(self.connection, window) })?;
        // An InputOnly window routes events but never obscures scene pixels.
        if a[26] != 2 || u16::from_ne_bytes([a[12], a[13]]) != 1 {
            return Ok(vec![]);
        }
        let (bounds, depth) = self.rect(window)?;
        if bounds.intersection(target).is_none() {
            return Ok(vec![]);
        }
        let workspace = self.property(window, "_NET_WM_DESKTOP", 6)?;
        if !workspace
            .first()
            .is_some_and(|w| *w == desktop || *w == u32::MAX)
        {
            return Ok(vec![]);
        }
        if self
            .property(window, "_NET_WM_STATE", 4)?
            .contains(&hidden_atom)
        {
            return Ok(vec![]);
        }
        if self.property(window, "_NET_WM_PID", 6)?.first() == Some(&std::process::id()) {
            return Ok(vec![]);
        }
        let opacity = self.property(window, "_NET_WM_WINDOW_OPACITY", 6)?;
        ensure!(opacity.len() <= 1, "Ambiguous window opacity");
        if opacity.first().is_some_and(|v| *v != u32::MAX) {
            return Ok(vec![]);
        }
        if self.shape_present {
            let shape = self.shape.context("Cannot verify client shape")?;
            if self.reply(unsafe { shape(self.connection, window) })?[8] != 0 {
                return Ok(vec![]);
            }
        }
        if depth <= 24 {
            return Ok(vec![bounds]);
        }
        let opaque = self.property(window, "_NET_WM_OPAQUE_REGION", 6)?;
        ensure!(opaque.len() % 4 == 0, "Malformed opaque region");
        Ok(opaque
            .chunks_exact(4)
            .filter_map(|r| {
                // Local unsigned coordinates are bounded before arithmetic.
                if r[0] > bounds.w as u32 || r[1] > bounds.h as u32 || r[2] > 65535 || r[3] > 65535
                {
                    return None;
                }
                Rect {
                    x: bounds.x + r[0] as i32,
                    y: bounds.y + r[1] as i32,
                    w: r[2] as i32,
                    h: r[3] as i32,
                }
                .intersection(bounds)
            })
            .collect())
    }
    pub(super) fn sample(&mut self, targets: &Targets) -> Result<Option<Hidden>> {
        let NativeTargets::X11 { scene, patch } = targets.native;
        self.scene_origin = None;
        ensure!(
            unsafe { (self.error)(self.connection) } == 0,
            "X11 connection lost"
        );
        if self.screen_off()? {
            return Ok(Some(Hidden::ScreenOff));
        }
        // Show-desktop bypasses occlusion, not native alignment. Reuse this
        // translation below so the native presenter adds no duplicate query.
        let native_target = if patch.is_some() {
            let target = self.rect(scene)?.0;
            if [target.w, target.h] == targets.scene_size {
                self.scene_origin = Some([target.x, target.y]);
            }
            Some(target)
        } else {
            None
        };
        if self.property(self.root, "_NET_SHOWING_DESKTOP", 6)?.first() == Some(&1) {
            return Ok(None);
        }
        let desktop = self.property(self.root, "_NET_CURRENT_DESKTOP", 6)?;
        let Some(&desktop) = desktop.first() else {
            return Ok(None);
        };
        let stacking = self.property(self.root, "_NET_CLIENT_LIST_STACKING", 33)?;
        ensure!(stacking.len() <= 128, "Too many clients to prove coverage");
        let Some(scene_index) = stacking.iter().position(|w| *w == scene) else {
            return Ok(None);
        };
        let top = match patch {
            Some(patch) => match stacking.iter().position(|w| *w == patch) {
                Some(index) => scene_index.max(index),
                None => return Ok(None),
            },
            None => scene_index,
        };
        let hidden_atom = self.atom("_NET_WM_STATE_HIDDEN")?;
        let target = match native_target {
            Some(target) => target,
            None => self.rect(scene)?.0,
        };
        // A smaller native patch can be hidden while the scene's static border
        // remains visible. Validate its expected layout before trusting it.
        let animation = match (patch, targets.animation) {
            (Some(patch), Some(local)) => {
                if [target.w, target.h] != targets.scene_size
                    || local.x < 0
                    || local.y < 0
                    || local.w <= 0
                    || local.h <= 0
                    || local.x.checked_add(local.w).is_none_or(|v| v > target.w)
                    || local.y.checked_add(local.h).is_none_or(|v| v > target.h)
                {
                    return Ok(None);
                }
                let actual = self.rect(patch)?.0;
                if actual
                    != (Rect {
                        x: target.x + local.x,
                        y: target.y + local.y,
                        ..local
                    })
                {
                    return Ok(None);
                }
                actual
            }
            _ => target,
        };
        let mut rects = Vec::new();
        for &client in stacking[top + 1..].iter().rev() {
            // Failure for any candidate makes the whole sample uncertain. Do
            // not silently treat an unreadable compositor property as opaque.
            rects.extend(self.opaque(client, desktop, hidden_atom, target)?);
            if rects.len() > 256 {
                return Ok(None);
            }
            if covered(target, &rects) {
                break;
            }
        }
        // Avoid an obvious torn read if a raise/close happened during sampling.
        if stacking != self.property(self.root, "_NET_CLIENT_LIST_STACKING", 33)?
            || self.property(self.root, "_NET_CURRENT_DESKTOP", 6)?.first() != Some(&desktop)
            || self.property(self.root, "_NET_SHOWING_DESKTOP", 6)?.first() == Some(&1)
            || self.rect(scene)?.0 != target
            || (targets.animation.is_some()
                && patch.is_some_and(|patch| {
                    self.rect(patch).map_or(true, |(rect, _)| rect != animation)
                }))
        {
            return Ok(None);
        }
        Ok(coverage_state(
            target,
            targets.animation.map(|_| animation),
            &rects,
        ))
    }
    fn screen_off(&mut self) -> Result<bool> {
        let Some(extension) = self.dpms.as_deref_mut() else {
            return Ok(false);
        };
        // DPMS Info is opcode 7, a four-byte header-only read request. XCB fills
        // the extension opcode and length. Two leading iovecs are reserved by
        // xcb_send_request; the checked reply handles server errors locally.
        let request = ProtocolRequest {
            count: 2,
            ext: extension,
            opcode: 7,
            is_void: 0,
        };
        let mut header = [0_u8; 4];
        let mut parts = [IoVec {
            base: ptr::null_mut(),
            len: 0,
        }; 4];
        parts[2] = IoVec {
            base: header.as_mut_ptr().cast(),
            len: header.len(),
        };
        let sequence =
            unsafe { (self.send)(self.connection, 1, parts.as_mut_ptr().add(2), &request) };
        ensure!(sequence != 0, "DPMS request unavailable");
        let reply = self.reply(Cookie { sequence })?;
        Ok(dpms_off(&reply))
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        unsafe {
            (self.disconnect)(self.connection);
        }
    }
}
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn dpms_off(reply: &[u8]) -> bool {
    reply.len() >= 11
        && reply[10] != 0
        && (1..=3).contains(&u16::from_ne_bytes([reply[8], reply[9]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Read-only native X11/DPMS probe; requires an accessible X server"]
    fn native_protocol_queries_are_read_only_and_checked() {
        let mut observer = Observer::new().expect("XCB connection");
        let root = observer.root;
        assert!(observer.rect(root).unwrap().0.w > 0);
        let off = observer.screen_off().expect("Checked DPMS Info");
        eprintln!("DPMS present={}, screen off={off}", observer.dpms.is_some());
        assert!(
            observer.rect(u32::MAX).is_err(),
            "BadWindow should return an error, not kill the process"
        );
        assert!(
            observer.rect(root).is_ok(),
            "Connection must survive BadWindow"
        );
    }
    #[test]
    fn dpms_disabled_or_unknown_power_levels_do_not_suspend() {
        let mut reply = [0_u8; 32];
        for level in 0_u16..=4 {
            reply[8..10].copy_from_slice(&level.to_ne_bytes());
            reply[10] = 0;
            assert!(!dpms_off(&reply));
            reply[10] = 1;
            assert_eq!(dpms_off(&reply), (1..=3).contains(&level));
        }
        assert!(!dpms_off(&[]));
    }
}
