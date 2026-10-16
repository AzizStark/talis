use super::*;
use std::{cell::Cell, process::Command, rc::Rc};

fn fixture() -> (crate::autostart::Fixture, gio::Settings) {
    let fixture = crate::autostart::Fixture::new();
    std::fs::write(
        fixture.root.join("org.gnome.shell.gschema.xml"),
        r#"
<schemalist><schema id="org.gnome.shell" path="/org/gnome/shell/">
<key name="enabled-extensions" type="as"><default>[]</default></key>
<key name="disabled-extensions" type="as"><default>[]</default></key>
<key name="disable-user-extensions" type="b"><default>false</default></key>
<key name="disable-extension-version-validation" type="b"><default>false</default></key>
</schema></schemalist>"#,
    )
    .unwrap();
    assert!(
        Command::new("glib-compile-schemas")
            .arg(&fixture.root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(
        fixture.root.join("metadata.json"),
        include_str!("../../../../integrations/gnome/metadata.json"),
    )
    .unwrap();
    let source = gio::SettingsSchemaSource::from_directory(&fixture.root, None, false).unwrap();
    let schema = source.lookup("org.gnome.shell", false).unwrap();
    let backend = gio::memory_settings_backend_new();
    let settings = gio::Settings::new_full(&schema, Some(&backend), None);
    settings.set_strv(ENABLED, ["other-a", "other-b"]).unwrap();
    settings
        .set_strv(DISABLED, ["disabled-other", UUID])
        .unwrap();
    (fixture, settings)
}

fn info(state: f64, can_change: bool) -> HashMap<String, glib::Variant> {
    HashMap::from([
        ("state".into(), state.to_variant()),
        ("canChange".into(), can_change.to_variant()),
    ])
}

#[test]
fn inspection_never_changes_enablement_and_unknown_requires_sign_in() {
    assert_eq!(
        classify(&HashMap::new(), false).unwrap(),
        Status::SignIn { enabled: false }
    );
    assert_eq!(
        classify(&info(2.0, true), false).unwrap(),
        Status::EnableNow
    );
    assert_eq!(
        classify(&info(6.0, true), false).unwrap(),
        Status::EnableNow
    );
    assert_eq!(
        classify(&info(1.0, false), false).unwrap(),
        Status::Starting
    );
    assert_eq!(classify(&info(8.0, true), false).unwrap(), Status::Starting);
    assert_eq!(
        classify(&info(4.0, true), true).unwrap(),
        Status::SignIn { enabled: true }
    );
    for state in [3.0, 5.0, 7.0, 99.0, f64::NAN, f64::INFINITY] {
        assert!(classify(&info(state, true), false).is_err());
    }
    assert!(classify(&info(2.0, false), false).is_err());
    assert!(
        classify(
            &HashMap::from([("state".into(), "bad".to_variant())]),
            false
        )
        .is_err()
    );
    assert_eq!(
        classify(&HashMap::new(), true).unwrap(),
        Status::SignIn { enabled: true }
    );
}

#[test]
fn consent_changes_only_our_uuid_and_is_idempotent() {
    let (enabled, disabled) = enabled_lists(
        vec!["other-a".into(), "other-b".into()],
        vec!["disabled-other".into(), UUID.into()],
    );
    assert_eq!(enabled, ["other-a", "other-b", UUID]);
    assert_eq!(disabled, ["disabled-other"]);
    assert_eq!(
        enabled_lists(enabled.clone(), disabled.clone()),
        (enabled, disabled)
    );
}

#[test]
#[ignore = "Requires a compiled private GNOME schema; run with isolated setup checks"]
fn global_extension_switch_is_never_changed() {
    let (_fixture, settings) = fixture();
    settings
        .set_boolean("disable-user-extensions", true)
        .unwrap();
    let before = settings.strv(ENABLED);
    assert!(remember_enablement(&settings).is_err());
    assert_eq!(settings.strv(ENABLED), before);
    assert!(settings.boolean("disable-user-extensions"));
    assert!(settings.strv(DISABLED).iter().any(|id| id == UUID));
}

#[test]
#[ignore = "Requires a compiled private GNOME schema; run with isolated setup checks"]
fn read_only_settings_refuse_enablement() {
    let (fixture, _) = fixture();
    let source = gio::SettingsSchemaSource::from_directory(&fixture.root, None, false).unwrap();
    let schema = source.lookup("org.gnome.shell", false).unwrap();
    let backend = gio::null_settings_backend_new();
    let settings = gio::Settings::new_full(&schema, Some(&backend), None);
    assert!(!settings.is_writable(ENABLED));
    assert!(remember_enablement(&settings).is_err());
    assert!(settings.strv(ENABLED).is_empty());
}

#[test]
#[ignore = "Requires a private dbus-run-session and TALIS_GNOME_SETUP_TEST=1; never run on the real session bus"]
fn private_shell_consent_enable_logout_and_failure_paths() {
    assert_eq!(std::env::var("TALIS_GNOME_SETUP_TEST").as_deref(), Ok("1"));
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .starts_with("unix:path=/tmp/")
    );
    let context = glib::MainContext::new();
    context.with_thread_default(|| context.block_on(async {
        let (fixture, settings) = fixture();
        let server = gio::DBusConnection::for_address_future(
            &std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION, None,
        ).await.unwrap();
        let name = server.call_future(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "RequestName", Some(&("org.gnome.SessionManager", 4_u32).to_variant()),
            None, gio::DBusCallFlags::NONE, 1000).await.unwrap();
        assert_eq!(name.get::<(u32,)>(), Some((1,)), "a real session manager must never be called");
        let state = Rc::new(Cell::new(None::<f64>));
        let enabled = Rc::new(Cell::new(true));
        let version = Rc::new(std::cell::RefCell::new(String::from("50.1")));
        let enables = Rc::new(Cell::new(0));
        let logouts = Rc::new(Cell::new(0));
        let opens = Rc::new(Cell::new(0));
        let xml = gio::DBusNodeInfo::for_xml(r#"<node>
<interface name="org.freedesktop.DBus.Properties"><method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method></interface>
<interface name="org.gnome.Shell.Extensions"><method name="GetExtensionInfo"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method><method name="EnableExtension"><arg type="s" direction="in"/><arg type="b" direction="out"/></method></interface>
<interface name="org.gnome.SessionManager"><method name="Logout"><arg type="u" direction="in"/></method></interface>
<interface name="io.github.azizstark.Talis"><method name="OpenSettings"/></interface>
</node>"#).unwrap();
        let mut registrations = Vec::new();
        for (name, path) in [
            ("org.freedesktop.DBus.Properties", PATH),
            (INTERFACE, PATH),
            ("org.gnome.SessionManager", "/org/gnome/SessionManager"),
            ("io.github.azizstark.Talis", "/io/github/azizstark/Talis"),
        ] {
            let interface = xml.lookup_interface(name).unwrap();
            let (state, enabled, version, enables, logouts, opens) = (state.clone(), enabled.clone(), version.clone(), enables.clone(), logouts.clone(), opens.clone());
            registrations.push(server.register_object(path, &interface).method_call(move |_, _, _, _, method, args, reply| {
                match method {
                    "GetAll" => reply.return_value(Some(&(HashMap::from([
                        ("ShellVersion".to_string(), version.borrow().to_variant()),
                        ("UserExtensionsEnabled".to_string(), enabled.get().to_variant()),
                    ]),).to_variant())),
                    "GetExtensionInfo" => { assert_eq!(args.get::<(String,)>().unwrap().0, UUID); let mut value = state.get().map(|s| info(s, true)).unwrap_or_default(); if state.get().is_some() { value.insert("error".into(), "TypeError: API unavailable".to_variant()); } reply.return_value(Some(&(value,).to_variant())); }
                    "EnableExtension" => { assert_eq!(args.get::<(String,)>().unwrap().0, UUID); enables.set(enables.get()+1); state.set(Some(1.0)); reply.return_value(Some(&(true,).to_variant())); }
                    "Logout" => { assert_eq!(args.get::<(u32,)>(), Some((0,))); logouts.set(logouts.get()+1); reply.return_value(Some(&().to_variant())); }
                    "OpenSettings" => { opens.set(opens.get()+1); reply.return_value(Some(&().to_variant())); }
                    _ => panic!("unexpected method {method}"),
                }
            }).build().unwrap());
        }
        let client = Client { connection: gio::bus_get_future(gio::BusType::Session).await.unwrap(), owner: server.unique_name().unwrap().to_string(), settings: settings.clone(), extension: fixture.root.clone(), user_extension: fixture.root.join("user/gnome-shell/extensions").join(UUID) };
        assert_eq!(client.status().await.unwrap(), Status::SignIn { enabled: false });
        assert_eq!((enables.get(), logouts.get()), (0, 0));
        assert!(!settings.strv(ENABLED).iter().any(|id| id == UUID));
        assert!(client.request_logout().await.is_err());
        assert_eq!(client.enable().await.unwrap(), Status::SignIn { enabled: true });
        assert_eq!((enables.get(), logouts.get()), (0, 0), "saving consent must not sign out or call EnableExtension for an undiscovered extension");
        assert_eq!(settings.strv(ENABLED).as_slice(), ["other-a", "other-b", UUID]);
        client.request_logout().await.unwrap();
        assert_eq!(logouts.get(), 1);
        state.set(Some(2.0));
        assert_eq!(client.enable().await.unwrap(), Status::Starting);
        assert_eq!(enables.get(), 1);
        assert!(client.open_settings().await);
        assert_eq!(opens.get(), 1);
        enabled.set(false);
        assert!(client.enable().await.is_err());
        assert!(client.request_logout().await.is_err());
        enabled.set(true);
        *version.borrow_mut() = "51.0".into();
        state.set(Some(4.0));
        let original = std::fs::read(fixture.root.join("metadata.json")).unwrap();
        assert_eq!(client.status().await.unwrap(), Status::Prepare);
        assert!(!client.user_extension.exists(), "inspection must not write metadata");
        assert_eq!(client.enable().await.unwrap(), Status::SignIn { enabled: true });
        assert_eq!((enables.get(), logouts.get()), (1, 1));
        assert_eq!(std::fs::read(fixture.root.join("metadata.json")).unwrap(), original);
        assert!(metadata::registered(&client.user_extension, "51.0").unwrap());
        assert_eq!(client.status().await.unwrap(), Status::SignIn { enabled: true });
        client.request_logout().await.unwrap();
        assert_eq!(logouts.get(), 2);
        assert!(!settings.boolean("disable-extension-version-validation"));
        state.set(Some(2.0));
        assert_eq!(client.enable().await.unwrap(), Status::Starting);
        assert_eq!(enables.get(), 2, "GNOME 51 must actually be attempted");
        *version.borrow_mut() = "99.alpha".into();
        state.set(Some(2.0));
        assert_eq!(client.status().await.unwrap(), Status::EnableNow);
        assert_eq!(client.enable().await.unwrap(), Status::Starting);
        assert_eq!(enables.get(), 3, "loadable copies need no version-registration detour");
        state.set(Some(3.0));
        assert!(client.status().await.unwrap_err().to_string().contains("TypeError: API unavailable"));
        assert!(client.enable().await.is_err());
        assert_eq!((enables.get(), logouts.get()), (3, 2));
        for registration in registrations { server.unregister_object(registration).unwrap(); }
        assert!(client.status().await.is_err());
    })).unwrap();
}
