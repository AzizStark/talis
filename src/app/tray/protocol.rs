//! StatusNotifierItem and a static DBusMenu on the existing GLib context.
use super::TrayUi;
use gtk::{gio, glib, prelude::*};
use std::{collections::HashMap, rc::Rc};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_PATH: &str = "/StatusNotifierItem";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU_PATH: &str = "/TalisMenu";
type Properties = HashMap<String, glib::Variant>;
type Layout = (i32, Properties, Vec<glib::Variant>);
// KDE tries a symbolic name's non-symbolic base in installed themes before
// consulting a flat IconThemePath. Never share that base with the launcher.
const SYMBOLIC_ICON_NAME: &str = "io.github.azizstark.TalisTray-symbolic";
const SYMBOLIC_ICON: &str =
    include_str!("../../../assets/ui/branding/io.github.azizstark.TalisTray-symbolic.svg");
const SYMBOLIC_THEME_INDEX: &str = "[Icon Theme]\nName=Talis tray\n\
Comment=Private Talis status icon\nDirectories=scalable/status\n\n\
[scalable/status]\nSize=22\nMinSize=16\nMaxSize=256\nType=Scalable\nContext=Status\n";
// XApp resolves our private symbolic name to a file path. Cinnamon then treats
// that path as a themed symbolic name instead of loading the file. Publishing
// only pixmaps lets XApp supply the regular PNG path its applet can display.
pub(super) fn prefer_pixmaps(desktops: &str) -> bool {
    desktops.split(':').any(|desktop| {
        desktop.eq_ignore_ascii_case("Cinnamon") || desktop.eq_ignore_ascii_case("X-Cinnamon")
    }) && !desktops
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("KDE"))
}
const XML: &str = r#"<node>
<interface name="org.kde.StatusNotifierItem">
 <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
 <property name="Category" type="s" access="read"/>
 <property name="Id" type="s" access="read"/>
 <property name="Title" type="s" access="read"/>
 <property name="Status" type="s" access="read"/>
 <property name="WindowId" type="u" access="read"/>
 <property name="IconName" type="s" access="read"/>
 <property name="IconThemePath" type="s" access="read"/>
 <property name="IconPixmap" type="a(iiay)" access="read"/>
 <property name="OverlayIconName" type="s" access="read"/>
 <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
 <property name="AttentionIconName" type="s" access="read"/>
 <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
 <property name="AttentionMovieName" type="s" access="read"/>
 <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
 <property name="ItemIsMenu" type="b" access="read"/>
 <property name="Menu" type="o" access="read"/>
 <signal name="NewStatus"><arg type="s"/></signal>
</interface>
<interface name="com.canonical.dbusmenu">
 <property name="Version" type="u" access="read"/>
 <property name="TextDirection" type="s" access="read"/>
 <property name="Status" type="s" access="read"/>
 <property name="IconThemePath" type="as" access="read"/>
 <method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
 <method name="GetGroupProperties"><arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/></method>
 <method name="GetProperty"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
 <method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
 <method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
 <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
 <method name="AboutToShowGroup"><arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/></method>
 <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
 <signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})"/><arg type="a(ias)"/></signal>
</interface></node>"#;

pub(super) struct Backend {
    connection: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    cleanup: Vec<Box<dyn FnOnce()>>,
    _icons: Option<IconFiles>,
}
impl Backend {
    pub fn new(connection: gio::DBusConnection, ui: &Rc<TrayUi>) -> Result<Self, glib::Error> {
        let pixel_only = prefer_pixmaps(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
        let mut backend = Self {
            connection,
            registrations: vec![],
            cleanup: vec![],
            _icons: if pixel_only {
                None
            } else {
                IconFiles::new()
                    .map_err(|error| tracing::warn!("Cannot prepare themed tray icon: {error}"))
                    .ok()
            },
        };
        let info = gio::DBusNodeInfo::for_xml(XML)?;
        let weak = Rc::downgrade(ui);
        let status = Rc::downgrade(ui);
        let pixmaps = icon_pixmaps()?.to_variant();
        let icon_name = if backend._icons.is_some() {
            SYMBOLIC_ICON_NAME
        } else {
            ""
        };
        let icon_path = backend
            ._icons
            .as_ref()
            .map(|icons| icons.directory.to_string_lossy().into_owned())
            .unwrap_or_default();
        backend.registrations.push(
            backend
                .connection
                .register_object(ITEM_PATH, &info.interfaces()[0])
                .method_call(move |_, _, _, _, method, _, invocation| {
                    if matches!(method, "Activate" | "SecondaryActivate" | "ContextMenu") {
                        if let Some(ui) = weak.upgrade() {
                            ui.show();
                        }
                    }
                    invocation.return_value(Some(&().to_variant()));
                })
                .property(move |_, _, _, _, name| match name {
                    "Category" => "ApplicationStatus".to_variant(),
                    "Id" => "talis".to_variant(),
                    "Title" => "Talis".to_variant(),
                    "Status" => status_name(status.upgrade().is_some_and(|ui| ui.enabled.get()))
                        .to_variant(),
                    "WindowId" => 0_u32.to_variant(),
                    "IconName" => icon_name.to_variant(),
                    "IconThemePath" => icon_path.to_variant(),
                    "IconPixmap" => pixmaps.clone(),
                    "OverlayIconPixmap" | "AttentionIconPixmap" => {
                        Vec::<(i32, i32, Vec<u8>)>::new().to_variant()
                    }
                    "ToolTip" => (
                        "",
                        Vec::<(i32, i32, Vec<u8>)>::new(),
                        "Talis",
                        "Open settings",
                    )
                        .to_variant(),
                    "ItemIsMenu" => false.to_variant(),
                    "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH)
                        .unwrap()
                        .to_variant(),
                    _ => "".to_variant(),
                })
                .build()?,
        );
        let weak = Rc::downgrade(ui);
        backend.registrations.push(
            backend
                .connection
                .register_object(MENU_PATH, &info.interfaces()[1])
                .method_call(move |_, _, _, _, method, parameters, invocation| {
                    let ui = weak.upgrade();
                    match menu_reply(method, &parameters, |id| {
                        if let Some(ui) = &ui {
                            match id {
                                1 => ui.show(),
                                2 => ui.quit(),
                                _ => {}
                            }
                        }
                    }) {
                        Some(reply) => invocation.return_value(Some(&reply)),
                        None => invocation.return_dbus_error(
                            "com.canonical.dbusmenu.Error.InvalidMenuItem",
                            "Unknown menu item or property",
                        ),
                    }
                })
                .property(|_, _, _, _, name| match name {
                    "Version" => 3_u32.to_variant(),
                    "TextDirection" => "ltr".to_variant(),
                    "Status" => "normal".to_variant(),
                    "IconThemePath" => Vec::<String>::new().to_variant(),
                    _ => "".to_variant(),
                })
                .build()?,
        );
        let weak = Rc::downgrade(ui);
        // gio 0.21's watch-name wrapper unwraps a nonnullable connection in
        // name_vanished, but GIO supplies NULL on bus loss. Observe owner changes
        // directly instead; the supervisor handles connection loss separately.
        let subscription = backend.connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some(WATCHER),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if signal.connection.is_closed() {
                    return;
                }
                let Some((_, _, owner)) = signal.parameters.get::<(String, String, String)>()
                else {
                    return;
                };
                if let Some(ui) = weak.upgrade().filter(|ui| !ui.stopped.get()) {
                    ui.cancel_registration();
                    ui.set_available(false);
                    if !owner.is_empty() {
                        register(signal.connection, &owner, &ui);
                    }
                }
            },
        );
        backend.cleanup.push(Box::new(move || drop(subscription)));
        let weak = Rc::downgrade(ui);
        let subscription = backend.connection.subscribe_to_signal(
            Some(WATCHER),
            Some(WATCHER),
            None,
            Some(WATCHER_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if signal.connection.is_closed() {
                    return;
                }
                if matches!(
                    signal.signal_name,
                    "StatusNotifierHostRegistered" | "StatusNotifierHostUnregistered"
                ) {
                    if let Some(ui) = weak.upgrade() {
                        register(signal.connection, signal.sender_name, &ui);
                    }
                }
            },
        );
        backend.cleanup.push(Box::new(move || drop(subscription)));
        let weak = Rc::downgrade(ui);
        let subscription = backend.connection.subscribe_to_signal(
            Some(WATCHER),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            Some(WATCHER_PATH),
            Some(WATCHER),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if signal.connection.is_closed() {
                    return;
                }
                if let Some((_, changed, invalidated)) =
                    signal.parameters.get::<(String, Properties, Vec<String>)>()
                {
                    if changed.contains_key("IsStatusNotifierHostRegistered")
                        || invalidated
                            .iter()
                            .any(|name| name == "IsStatusNotifierHostRegistered")
                    {
                        if let Some(ui) = weak.upgrade() {
                            register(signal.connection, signal.sender_name, &ui);
                        }
                    }
                }
            },
        );
        backend.cleanup.push(Box::new(move || drop(subscription)));
        // Covers a watcher already present before we installed subscriptions.
        // NO_AUTO_START avoids creating a panel service when none is running.
        register(&backend.connection, WATCHER, ui);
        Ok(backend)
    }
    pub fn is_closed(&self) -> bool {
        self.connection.is_closed()
    }
    pub fn status_changed(&self, enabled: bool) {
        if let Err(error) = self.connection.emit_signal(
            None,
            ITEM_PATH,
            ITEM,
            "NewStatus",
            Some(&(status_name(enabled),).to_variant()),
        ) {
            tracing::warn!("Cannot update tray status: {error}");
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        for cleanup in self.cleanup.drain(..) {
            cleanup();
        }
        for registration in self.registrations.drain(..) {
            let _ = self.connection.unregister_object(registration);
        }
    }
}
fn status_name(enabled: bool) -> &'static str {
    if enabled { "Active" } else { "Passive" }
}
fn register(connection: &gio::DBusConnection, owner: &str, ui: &Rc<TrayUi>) {
    if ui.stopped.get() || connection.is_closed() {
        return;
    }
    ui.cancel_registration();
    let generation = ui.generation.get();
    let weak = Rc::downgrade(ui);
    let connection = connection.clone();
    let owner = owner.to_owned();
    *ui.registration_task.borrow_mut() = Some(glib::spawn_future_local(async move {
        let registered = connection
            .call_future(
                Some(&owner),
                WATCHER_PATH,
                WATCHER,
                "RegisterStatusNotifierItem",
                Some(&(ITEM_PATH,).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                2000,
            )
            .await;
        let available = if registered.is_ok() {
            connection
                .call_future(
                    Some(&owner),
                    WATCHER_PATH,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&(WATCHER, "IsStatusNotifierHostRegistered").to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    2000,
                )
                .await
                .ok()
                .and_then(|reply| reply.get::<(glib::Variant,)>())
                .and_then(|(value,)| value.get::<bool>())
                .unwrap_or(false)
        } else {
            false
        };
        if let Some(ui) = weak.upgrade().filter(|ui| {
            !ui.stopped.get() && !connection.is_closed() && ui.generation.get() == generation
        }) {
            ui.set_available(available);
        }
    }));
}
fn properties(id: i32, names: &[String]) -> Option<Properties> {
    let mut props = Properties::new();
    match id {
        0 => {
            props.insert("children-display".into(), "submenu".to_variant());
        }
        1 | 2 => {
            props.insert(
                "label".into(),
                if id == 1 {
                    "Open settings"
                } else {
                    "Quit Talis"
                }
                .to_variant(),
            );
            props.insert("enabled".into(), true.to_variant());
            props.insert("visible".into(), true.to_variant());
        }
        _ => return None,
    }
    if !names.is_empty() {
        props.retain(|name, _| names.contains(name));
    }
    Some(props)
}
fn layout(id: i32, depth: i32, names: &[String]) -> Option<Layout> {
    let props = properties(id, names)?;
    let children = if id == 0 && depth != 0 {
        [1, 2]
            .into_iter()
            .filter_map(|child| layout(child, 0, names))
            .map(|child| child.to_variant())
            .collect()
    } else {
        vec![]
    };
    Some((id, props, children))
}
fn menu_reply(
    method: &str,
    args: &glib::Variant,
    mut clicked: impl FnMut(i32),
) -> Option<glib::Variant> {
    Some(match method {
        "GetLayout" => {
            let (id, depth, names) = args.get::<(i32, i32, Vec<String>)>()?;
            (1_u32, layout(id, depth, &names)?).to_variant()
        }
        "GetGroupProperties" => {
            let (mut ids, names) = args.get::<(Vec<i32>, Vec<String>)>()?;
            if ids.is_empty() {
                ids = vec![0, 1, 2];
            }
            (ids.into_iter()
                .filter_map(|id| properties(id, &names).map(|props| (id, props)))
                .collect::<Vec<_>>(),)
                .to_variant()
        }
        "GetProperty" => {
            let (id, name) = args.get::<(i32, String)>()?;
            (properties(id, &[])?.remove(&name)?,).to_variant()
        }
        "Event" => {
            let (id, event, _, _) = args.get::<(i32, String, glib::Variant, u32)>()?;
            properties(id, &[])?;
            if event == "clicked" {
                clicked(id);
            }
            ().to_variant()
        }
        "EventGroup" => {
            let (events,) = args.get::<(Vec<(i32, String, glib::Variant, u32)>,)>()?;
            let mut errors = vec![];
            for (id, event, _, _) in events {
                if properties(id, &[]).is_none() {
                    errors.push(id);
                } else if event == "clicked" {
                    clicked(id);
                }
            }
            (errors,).to_variant()
        }
        "AboutToShow" => {
            let (id,) = args.get::<(i32,)>()?;
            properties(id, &[])?;
            (false,).to_variant()
        }
        "AboutToShowGroup" => {
            let (ids,) = args.get::<(Vec<i32>,)>()?;
            (
                Vec::<i32>::new(),
                ids.into_iter()
                    .filter(|id| properties(*id, &[]).is_none())
                    .collect::<Vec<_>>(),
            )
                .to_variant()
        }
        _ => return None,
    })
}
// Some panel hosts (notably Plasma) recolor named SVGs using their own palette.
// Export a private icon path instead of inferring panel colors from GTK settings.
// Old hosts keep the existing ARGB fallback. Only these owned files are removed.
struct IconFiles {
    root: std::path::PathBuf,
    directory: std::path::PathBuf,
}
impl IconFiles {
    fn new() -> std::io::Result<Self> {
        let root = glib::mkdtemp_full(std::env::temp_dir().join("talis-tray-XXXXXX"), 0o700)
            .ok_or_else(std::io::Error::last_os_error)?;
        let icons = Self {
            directory: root.join("icons"),
            root,
        };
        let status = icons.directory.join("hicolor/scalable/status");
        std::fs::create_dir_all(&status)?;
        std::fs::write(
            icons.directory.join("hicolor/index.theme"),
            SYMBOLIC_THEME_INDEX,
        )?;
        std::fs::write(
            status.join(format!("{SYMBOLIC_ICON_NAME}.svg")),
            SYMBOLIC_ICON,
        )?;
        // Also retain a flat copy for hosts that do not load hicolor themes.
        std::fs::write(
            icons.directory.join(format!("{SYMBOLIC_ICON_NAME}.svg")),
            SYMBOLIC_ICON,
        )?;
        Ok(icons)
    }
}
impl Drop for IconFiles {
    fn drop(&mut self) {
        let theme = self.directory.join("hicolor");
        let scalable = theme.join("scalable");
        let status = scalable.join("status");
        let _ = std::fs::remove_file(status.join(format!("{SYMBOLIC_ICON_NAME}.svg")));
        let _ = std::fs::remove_file(theme.join("index.theme"));
        let _ = std::fs::remove_dir(status);
        let _ = std::fs::remove_dir(scalable);
        let _ = std::fs::remove_dir(theme);
        let _ = std::fs::remove_file(self.directory.join(format!("{SYMBOLIC_ICON_NAME}.svg")));
        let _ = std::fs::remove_dir(&self.directory);
        let _ = std::fs::remove_dir(&self.root);
    }
}
// Rasterize the approved app monogram once into network-order ARGB.
fn icon_pixmaps() -> Result<Vec<(i32, i32, Vec<u8>)>, glib::Error> {
    // The approved master includes its padding. Keep the host's icon slot and
    // Cinnamon's pixmap-only workaround; there is no per-frame rasterization.
    let svg = include_str!("../../../assets/ui/branding/talis-monogram-light.svg");
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|size| {
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(
                svg.as_bytes().to_vec(),
            ));
            let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
                &stream,
                size,
                size,
                true,
                None::<&gio::Cancellable>,
            )?;
            let pixels = pixbuf.read_pixel_bytes();
            let channels = pixbuf.n_channels() as usize;
            let stride = pixbuf.rowstride() as usize;
            let mut bytes = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size {
                for x in 0..size {
                    let offset = y as usize * stride + x as usize * channels;
                    let alpha = if pixbuf.has_alpha() {
                        pixels[offset + 3]
                    } else {
                        255
                    };
                    bytes.extend_from_slice(&[
                        alpha,
                        pixels[offset],
                        pixels[offset + 1],
                        pixels[offset + 2],
                    ]);
                }
            }
            Ok((size, size, bytes))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cinnamon_uses_pixels_without_changing_plasma_or_other_desktops() {
        for desktop in [
            "Cinnamon",
            "X-Cinnamon",
            "x-cinnamon:GNOME",
            "GNOME:Cinnamon",
        ] {
            assert!(prefer_pixmaps(desktop));
        }
        for desktop in [
            "KDE",
            "KDE:Cinnamon",
            "MATE",
            "XFCE",
            "GNOME",
            "",
            "NotCinnamon",
        ] {
            assert!(!prefer_pixmaps(desktop));
        }
    }
    #[test]
    fn themed_tray_files_are_private_owned_and_cleaned_up() {
        use std::os::unix::fs::PermissionsExt;
        let icons = IconFiles::new().unwrap();
        let root = icons.root.clone();
        let path = icons.directory.join(format!("{SYMBOLIC_ICON_NAME}.svg"));
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(icons.directory.file_name().unwrap(), "icons");
        let svg = std::fs::read_to_string(&path).unwrap();
        assert_eq!(svg, SYMBOLIC_ICON);
        assert_eq!(
            std::fs::read_to_string(
                icons
                    .directory
                    .join("hicolor/scalable/status")
                    .join(format!("{SYMBOLIC_ICON_NAME}.svg"))
            )
            .unwrap(),
            svg
        );
        assert_eq!(
            std::fs::read_to_string(icons.directory.join("hicolor/index.theme")).unwrap(),
            SYMBOLIC_THEME_INDEX
        );
        assert!(
            !SYMBOLIC_ICON_NAME.starts_with(&format!("{}-", crate::app::style::APP_ICON_NAME)),
            "KDE must not fall back from the tray name to the launcher"
        );
        assert!(
            !svg.contains("<rect"),
            "The tray must not gain the launcher background"
        );
        assert!(svg.contains("current-color-scheme") && svg.contains("ColorScheme-Text"));
        assert!(svg.contains("stroke=\"currentColor\""));
        drop(icons);
        assert!(!path.exists() && !root.exists());
    }
    #[test]
    fn menu_layout_and_events_are_typed_and_bounded() {
        gio::DBusNodeInfo::for_xml(XML).unwrap();
        let args = (0_i32, -1_i32, Vec::<String>::new()).to_variant();
        let reply = menu_reply("GetLayout", &args, |_| panic!("not a click")).unwrap();
        assert_eq!(reply.type_().as_str(), "(u(ia{sv}av))");
        let (_, root) = reply.get::<(u32, Layout)>().unwrap();
        assert_eq!(root.2.len(), 2);
        assert_eq!(root.2[0].get::<Layout>().unwrap().0, 1);
        assert!(layout(9, -1, &[]).is_none());
        assert!(layout(0, 0, &[]).unwrap().2.is_empty());
        assert_eq!(properties(1, &["enabled".into()]).unwrap().len(), 1);
        let mut clicks = vec![];
        let events = (vec![
            (1_i32, "clicked", 0_i32.to_variant(), 0_u32),
            (99, "clicked", 0_i32.to_variant(), 0_u32),
            (2, "hovered", 0_i32.to_variant(), 0_u32),
        ],)
            .to_variant();
        let reply = menu_reply("EventGroup", &events, |id| clicks.push(id)).unwrap();
        assert_eq!(clicks, [1]);
        assert_eq!(reply.get::<(Vec<i32>,)>().unwrap().0, [99]);
        assert!(menu_reply("Event", &().to_variant(), |_| {}).is_none());
        assert_eq!(
            icon_pixmaps().unwrap().to_variant().type_().as_str(),
            "a(iiay)"
        );
    }
    #[test]
    fn tray_svg_renders_transparent_argb_at_panel_sizes() {
        let icons = icon_pixmaps().unwrap();
        assert_eq!(icons.len(), 6);
        for (width, height, pixels) in icons {
            assert_eq!(width, height);
            assert_eq!(pixels.len(), (width * height * 4) as usize);
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[0] == 0));
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[0] > 0));
            assert!(
                pixels
                    .chunks_exact(4)
                    .any(|pixel| pixel[0] > 0 && pixel[0] < 255)
            );
            // Partially covered pixels can vary after SVG alpha round-tripping.
            for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[0] == 255) {
                for (actual, expected) in pixel[1..].iter().zip([238_u8, 231, 220]) {
                    assert!(actual.abs_diff(expected) <= 2, "SVG color rounding");
                }
            }
        }
    }
}
