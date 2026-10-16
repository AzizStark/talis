//! Register the current Shell for this companion only. No global version switch.
use super::UUID;
use anyhow::{Context, Result, ensure};
use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};

fn version_key(version: &str) -> Result<String> {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or_default();
    ensure!(
        !major.is_empty() && major.len() <= 10 && major.bytes().all(|b| b.is_ascii_digit()),
        "GNOME returned an invalid Shell version: {version}"
    );
    // Old Shells used major.minor keys. Trying them is allowed too; actual API
    // or import failures are reported by Shell, not guessed from their version.
    if major == "3" {
        let minor = parts.next().unwrap_or_default();
        ensure!(
            !minor.is_empty() && minor.len() <= 10 && minor.bytes().all(|b| b.is_ascii_digit()),
            "GNOME returned an invalid Shell version: {version}"
        );
        Ok(format!("{major}.{minor}"))
    } else {
        Ok(major.into())
    }
}

fn read(directory: &Path) -> Result<(serde_json::Value, Option<glib::GString>)> {
    let path = directory.join("metadata.json");
    ensure!(
        std::fs::metadata(&path)?.len() <= 32 * 1024,
        "Companion metadata is too large"
    );
    let (bytes, etag) = gio::File::for_path(path).load_contents(None::<&gio::Cancellable>)?;
    ensure!(bytes.len() <= 32 * 1024, "Companion metadata is too large");
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["uuid"].as_str() == Some(UUID),
        "Invalid Talis companion UUID"
    );
    ensure!(
        value["shell-version"]
            .as_array()
            .is_some_and(|versions| !versions.is_empty()
                && versions.iter().all(serde_json::Value::is_string)),
        "Invalid Talis companion metadata"
    );
    Ok((value, etag))
}

pub(super) fn registered(directory: &Path, version: &str) -> Result<bool> {
    let key = version_key(version)?;
    let (value, _) = read(directory)?;
    Ok(value["shell-version"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry.as_str() == Some(&key)))
}

pub(super) fn prepare(source: &Path, user_directory: &Path, version: &str) -> Result<()> {
    ensure!(
        user_directory.is_absolute() && user_directory.file_name().is_some_and(|name| name == UUID),
        "Invalid per-user companion destination"
    );
    let key = version_key(version)?;
    let (mut value, etag) = read(source)?;
    let versions = value["shell-version"].as_array_mut().unwrap();
    if !versions.iter().any(|entry| entry.as_str() == Some(&key)) {
        versions.push(key.into());
    }
    let bytes = serde_json::to_vec_pretty(&value)?;
    if source == user_directory {
        ensure!(
            !user_directory.is_symlink(),
            "The per-user Talis companion is a directory link. Install it as a local directory before updating its metadata; the linked source was not changed."
        );
        // Only metadata changes; preserve user code/config and reject concurrent
        // edits through the load etag. Replace a metadata symlink, not its target.
        gio::File::for_path(source.join("metadata.json"))
            .replace_contents(
                &bytes,
                etag.as_deref(),
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
                None::<&gio::Cancellable>,
            )
            .context("Could not register this Shell in Talis companion metadata")?;
        return Ok(());
    }
    // System package files stay read-only. Import the packaged entry by URI so
    // its relative imports (including new modules in upgrades) stay in that tree.
    let source = source.canonicalize()?;
    let parent = user_directory
        .parent()
        .context("Missing companion parent")?;
    std::fs::create_dir_all(parent)?;
    let path = parent.join(format!(".talis-setup-{}", glib::uuid_string_random()));
    std::fs::create_dir(&path)?;
    let staging = Staging(path);
    for entry in std::fs::read_dir(&source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "metadata.json" || name == "extension.js" {
            continue;
        }
        let path = Path::new(&name);
        if !matches!(path.extension().and_then(|s| s.to_str()), Some("json"))
            && !matches!(
                name.to_str(),
                Some("LICENSE" | "assets" | "talis-gnome-scene")
            )
        {
            continue;
        }
        std::os::unix::fs::symlink(source.join(&name), staging.0.join(name))?;
    }
    let entry = gio::File::for_path(source.join("extension.js")).uri();
    std::fs::write(
        staging.0.join("extension.js"),
        format!(
            "export {{default}} from {};\n",
            serde_json::to_string(entry.as_str())?
        ),
    )?;
    std::fs::write(staging.0.join("metadata.json"), bytes)?;
    gio::File::for_path(&staging.0)
        .move_(
            &gio::File::for_path(user_directory),
            gio::FileCopyFlags::NONE,
            None::<&gio::Cancellable>,
            None,
        )
        .context(
            "Could not prepare the per-user Talis companion; no existing extension was replaced",
        )?;
    Ok(())
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        // This exact, newly created staging directory contains only our metadata
        // and symlinks. Removing links never removes their package targets.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
