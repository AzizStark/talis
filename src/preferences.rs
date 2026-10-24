//! Versioned user preferences, independent of GTK, renderer state and scene paths.
mod storage;
mod worker;
use crate::config::{Config, SceneResolution, SceneVariant, WaylandLayer};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub use storage::Store;
pub use worker::Worker;

pub type Patch = toml::map::Map<String, toml::Value>;
fn version() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Document {
    #[serde(default = "version")]
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneVariant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_resolution: Option<SceneResolution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_when_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimize_to_tray: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_size_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_rpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_brightness: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleeve_shadow_lift: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arm_range_start: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arm_range_end: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_mate_overlay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wayland_layer: Option<WaylandLayer>,
    /// "automatic" explicitly clears a formerly pinned player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_player: Option<String>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: 1,
            scene: None,
            scene_resolution: None,
            fps: None,
            pause_when_hidden: None,
            minimize_to_tray: None,
            label_size_percent: None,
            rotation_rpm: None,
            surface_opacity: None,
            surface_brightness: None,
            sleeve_shadow_lift: None,
            arm_range_start: None,
            arm_range_end: None,
            experimental_mate_overlay: None,
            wayland_layer: None,
            preferred_player: None,
        }
    }
}

impl Document {
    pub fn empty() -> Self {
        Self {
            schema_version: 1,
            ..Self::default()
        }
    }
    pub fn apply(&self, config: &mut Config) {
        macro_rules! apply {
            ($field:ident, $target:expr) => {
                if let Some(value) = self.$field {
                    $target = value;
                }
            };
        }
        apply!(scene, config.scene.variant);
        apply!(scene_resolution, config.scene.resolution);
        apply!(fps, config.render.fps);
        apply!(pause_when_hidden, config.render.pause_when_hidden);
        apply!(minimize_to_tray, config.minimize_to_tray);
        apply!(label_size_percent, config.artwork.size_percent);
        apply!(rotation_rpm, config.artwork.rotation_rpm);
        apply!(surface_opacity, config.artwork.vinyl_opacity);
        apply!(surface_brightness, config.artwork.vinyl_brightness);
        apply!(sleeve_shadow_lift, config.artwork.sleeve_shadow_lift);
        apply!(arm_range_start, config.render.arm_range_start);
        apply!(arm_range_end, config.render.arm_range_end);
        apply!(
            experimental_mate_overlay,
            config.render.experimental_mate_overlay
        );
        apply!(wayland_layer, config.render.wayland_layer);
    }
    pub fn player(&self) -> Result<Option<String>> {
        match self.preferred_player.as_deref() {
            None | Some("automatic") => Ok(None),
            Some(player) => normalize_player(player).map(Some),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "Unsupported preferences schema version {}",
            self.schema_version
        );
        let mut config = Config::default();
        self.apply(&mut config);
        config.validate()?;
        self.player()?;
        Ok(())
    }
    pub fn from_config(config: &Config, player: Option<&str>) -> Self {
        Self {
            schema_version: 1,
            scene: Some(config.scene.variant),
            scene_resolution: Some(config.scene.resolution),
            fps: Some(config.render.fps),
            pause_when_hidden: Some(config.render.pause_when_hidden),
            minimize_to_tray: Some(config.minimize_to_tray),
            label_size_percent: Some(config.artwork.size_percent),
            rotation_rpm: Some(config.artwork.rotation_rpm),
            surface_opacity: Some(config.artwork.vinyl_opacity),
            surface_brightness: Some(config.artwork.vinyl_brightness),
            sleeve_shadow_lift: Some(config.artwork.sleeve_shadow_lift),
            arm_range_start: Some(config.render.arm_range_start),
            arm_range_end: Some(config.render.arm_range_end),
            experimental_mate_overlay: Some(config.render.experimental_mate_overlay),
            wayland_layer: Some(config.render.wayland_layer),
            preferred_player: Some(player.unwrap_or("automatic").into()),
        }
    }
    pub fn values(&self) -> Patch {
        toml::Value::try_from(self)
            .expect("typed preferences serialize")
            .as_table()
            .unwrap()
            .clone()
    }
}

pub fn normalize_player(value: &str) -> Result<String> {
    let name = if value.starts_with("org.mpris.MediaPlayer2.") {
        value.to_owned()
    } else {
        format!("org.mpris.MediaPlayer2.{value}")
    };
    ensure!(
        name.len() <= 255
            && !name.split('.').any(|part| part.is_empty()
                || part.as_bytes()[0].is_ascii_digit()
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')),
        "Invalid MPRIS player name"
    );
    Ok(name)
}

pub fn config_directory() -> Result<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".config"))
        })
        .context("Cannot locate the user's config directory")
}

pub struct Loaded {
    pub config: Config,
    pub document: Document,
    pub store: Option<Store>,
    pub issue: Option<String>,
}
/// Startup snapshots retain package/launch requirements, not frozen UI values.
pub fn startup_base(mut config: Config) -> Config {
    let mut defaults = Document::from_config(&Config::default(), None);
    defaults.scene = None;
    defaults.apply(&mut config);
    config
}
#[derive(Clone)]
pub struct Session {
    pub store: Option<Store>,
    pub issue: Option<String>,
}

pub fn load_user(base: Config, explicit: Option<&Path>, enabled: bool) -> Result<Loaded> {
    if !enabled {
        return load(base, explicit, None);
    }
    match Store::for_user() {
        Ok(store) => load(base, explicit, Some(store)),
        Err(error) => {
            let mut loaded = load(base, explicit, None)?;
            loaded.issue = Some(format!("Preferences unavailable: {error:#}"));
            Ok(loaded)
        }
    }
}

/// Explicit files overlay only their supplied fields. No loading operation writes.
pub fn load(base: Config, explicit: Option<&Path>, store: Option<Store>) -> Result<Loaded> {
    let (document, issue) = match store.as_ref().map(Store::read).transpose() {
        Ok(document) => (document.unwrap_or_else(Document::empty), None),
        Err(error) => (
            Document::empty(),
            Some(format!(
                "Preferences unavailable; file preserved: {error:#}"
            )),
        ),
    };
    let mut config = base.clone();
    document.apply(&mut config);
    if let Some(path) = explicit {
        config = config.overlay(path)?;
    }
    if let Err(error) = config.validate() {
        // A saved scene may be unavailable in a restricted custom/owned package.
        // Keep the saved preference intact rather than replacing it with fallback.
        if explicit.is_some() {
            return Err(error);
        }
        config.scene.variant = base.scene.variant;
        config.validate()?;
        return Ok(Loaded {
            config,
            document: document.clone(),
            store,
            issue: Some(format!(
                "Saved settings unavailable in this scene package: {error:#}"
            )),
        });
    }
    Ok(Loaded {
        config,
        document,
        store,
        issue,
    })
}

#[cfg(test)]
mod tests;
