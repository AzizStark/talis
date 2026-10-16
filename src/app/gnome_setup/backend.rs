//! Bounded GNOME IPC and consent-only changes to this companion's enablement.
mod metadata;
use anyhow::{Context, Result, bail, ensure};
use gtk::{gio, glib, prelude::*};
use std::{collections::HashMap, path::PathBuf};

pub(super) const UUID: &str = "talis@azizstark";
const INTERFACE: &str = "org.gnome.Shell.Extensions";
const PATH: &str = "/org/gnome/Shell";
const ENABLED: &str = "enabled-extensions";
const DISABLED: &str = "disabled-extensions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Prepare,
    EnableNow,
    SignIn { enabled: bool },
    Starting,
}

pub(super) struct Client {
    connection: gio::DBusConnection,
    owner: String,
    settings: gio::Settings,
    extension: PathBuf,
    user_extension: PathBuf,
}
impl Client {
    pub(super) async fn connect() -> Result<Self> {
        let connection = crate::session_bus::connect().await?;
        let owner = connection
            .call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                Some(&("org.gnome.Shell",).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
            )
            .await?
            .get::<(String,)>()
            .context("GNOME Shell is unavailable")?
            .0;
        let schema = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup("org.gnome.shell", true))
            .context("GNOME extension settings are unavailable")?;
        for key in [ENABLED, DISABLED, "disable-user-extensions"] {
            ensure!(
                schema.has_key(key),
                "GNOME extension settings are incomplete"
            );
        }
        let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
        let mut roots = vec![glib::user_data_dir()];
        if let Some(root) = crate::resources::data_directory()? {
            if let Some(share) = root.parent() {
                roots.push(share.to_path_buf());
            }
        }
        roots.extend(glib::system_data_dirs());
        let extension = roots.into_iter()
            .map(|root| root.join("gnome-shell/extensions").join(UUID))
            .find(|path| path.join("metadata.json").is_file())
            .context("The GNOME companion is not installed. For a portable bundle, run setup-gnome.sh first, then reopen Talis.")?;
        Ok(Self {
            connection,
            owner,
            settings,
            extension,
            user_extension: glib::user_data_dir()
                .join("gnome-shell/extensions")
                .join(UUID),
        })
    }

    async fn call(
        &self,
        interface: &str,
        method: &str,
        args: &glib::Variant,
    ) -> Result<glib::Variant> {
        Ok(self
            .connection
            .call_future(
                Some(&self.owner),
                PATH,
                interface,
                method,
                Some(args),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
            )
            .await?)
    }

    async fn shell_version(&self) -> Result<String> {
        let (properties,) = self
            .call(
                "org.freedesktop.DBus.Properties",
                "GetAll",
                &(INTERFACE,).to_variant(),
            )
            .await?
            .get::<(HashMap<String, glib::Variant>,)>()
            .context("Invalid GNOME properties")?;
        ensure!(
            properties
                .get("UserExtensionsEnabled")
                .and_then(|v| v.get::<bool>())
                == Some(true)
                && !self.settings.boolean("disable-user-extensions"),
            "Extensions are turned off in GNOME. Turn them on in the Extensions app, then try again."
        );
        properties
            .get("ShellVersion")
            .and_then(|v| v.get::<String>())
            .context("GNOME Shell version is unavailable")
    }

    fn companion_directory(&self) -> &std::path::Path {
        if self.user_extension.exists() || self.user_extension.is_symlink() {
            &self.user_extension
        } else {
            &self.extension
        }
    }

    pub(super) async fn status(&self) -> Result<Status> {
        let version = self.shell_version().await?;
        let current = metadata::registered(self.companion_directory(), &version)?;
        let (info,) = self
            .call(INTERFACE, "GetExtensionInfo", &(UUID,).to_variant())
            .await?
            .get::<(HashMap<String, glib::Variant>,)>()
            .context("Invalid companion status")?;
        let scheduled = self.settings.strv(ENABLED).iter().any(|id| id == UUID)
            && !self.settings.strv(DISABLED).iter().any(|id| id == UUID);
        let status = classify(&info, scheduled)?;
        // Recognized, loadable companions can be attempted as-is. Only unknown
        // or cached-outdated copies need local metadata preparation.
        if !current
            && (info.is_empty() || info.get("state").and_then(|v| v.get::<f64>()) == Some(4.0))
        {
            Ok(Status::Prepare)
        } else {
            Ok(status)
        }
    }

    /// Called only by the setup button. Recheck the current Shell before writing.
    pub(super) async fn enable(&self) -> Result<Status> {
        match self.status().await? {
            Status::Prepare => {
                let version = self.shell_version().await?;
                metadata::prepare(self.companion_directory(), &self.user_extension, &version)?;
                remember_enablement(&self.settings)?;
                Ok(Status::SignIn { enabled: true })
            }
            Status::EnableNow => {
                let accepted = self
                    .call(INTERFACE, "EnableExtension", &(UUID,).to_variant())
                    .await?
                    .get::<(bool,)>()
                    .context("Invalid enable response")?
                    .0;
                ensure!(
                    accepted,
                    "GNOME could not enable Talis. Try again after signing in."
                );
                self.status().await
            }
            Status::SignIn { .. } => {
                remember_enablement(&self.settings)?;
                Ok(Status::SignIn { enabled: true })
            }
            Status::Starting => Ok(Status::Starting),
        }
    }

    pub(super) async fn open_settings(&self) -> bool {
        self.connection
            .call_future(
                Some(&self.owner),
                "/io/github/azizstark/Talis",
                "io.github.azizstark.Talis",
                "OpenSettings",
                None,
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                1500,
            )
            .await
            .is_ok()
    }

    /// Normal confirmation and inhibitors remain controlled by GNOME. Never force.
    pub(super) async fn request_logout(&self) -> Result<()> {
        ensure!(
            self.status().await? == (Status::SignIn { enabled: true }),
            "Recheck Talis setup before signing out"
        );
        self.connection.call_future(
            Some("org.gnome.SessionManager"), "/org/gnome/SessionManager",
            "org.gnome.SessionManager", "Logout", Some(&(0_u32,).to_variant()), None,
            gio::DBusCallFlags::NO_AUTO_START, 3000,
        ).await.context("Could not open GNOME's logout confirmation. You can sign out from the system menu instead.")?;
        Ok(())
    }
}

fn classify(info: &HashMap<String, glib::Variant>, enabled: bool) -> Result<Status> {
    if info.is_empty() {
        return Ok(Status::SignIn { enabled });
    }
    // Shell serializes JavaScript numbers as doubles, including enum values.
    let state = info
        .get("state")
        .and_then(|v| v.get::<f64>())
        .context("GNOME returned an invalid companion state")?;
    match state {
        1.0 | 8.0 => Ok(Status::Starting),
        2.0 | 6.0 => {
            ensure!(
                info.get("canChange").and_then(|v| v.get::<bool>()) == Some(true),
                "GNOME does not allow changing this extension. Check your administrator's extension policy."
            );
            Ok(Status::EnableNow)
        }
        3.0 => {
            let error = info
                .get("error")
                .and_then(|v| v.str())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("No details were provided; check the GNOME Shell journal.");
            bail!(
                "The GNOME companion failed to load: {}",
                error.chars().take(1000).collect::<String>()
            )
        }
        // Shell caches metadata until the next login. Preparation may already
        // have registered this version on disk without updating its loaded copy.
        4.0 => Ok(Status::SignIn { enabled }),
        _ => bail!("GNOME is changing the companion's state. Try again shortly."),
    }
}

fn enabled_lists(
    mut enabled: Vec<String>,
    mut disabled: Vec<String>,
) -> (Vec<String>, Vec<String>) {
    if !enabled.iter().any(|id| id == UUID) {
        enabled.push(UUID.into());
    }
    disabled.retain(|id| id != UUID);
    (enabled, disabled)
}

fn remember_enablement(settings: &gio::Settings) -> Result<()> {
    ensure!(
        !settings.boolean("disable-user-extensions"),
        "GNOME extensions are turned off"
    );
    let old_enabled = settings
        .strv(ENABLED)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let old_disabled = settings
        .strv(DISABLED)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let (enabled, disabled) = enabled_lists(old_enabled.clone(), old_disabled.clone());
    let add = enabled != old_enabled;
    let remove = disabled != old_disabled;
    ensure!(
        (!add || settings.is_writable(ENABLED)) && (!remove || settings.is_writable(DISABLED)),
        "Your administrator has locked GNOME extension settings"
    );
    settings.delay();
    let result = (|| -> Result<()> {
        if add {
            settings.set_strv(ENABLED, enabled.as_slice())?;
        }
        if remove {
            settings.set_strv(DISABLED, disabled.as_slice())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        settings.revert();
        return Err(error);
    }
    settings.apply();
    gio::Settings::sync();
    Ok(())
}

#[cfg(test)]
mod tests;
