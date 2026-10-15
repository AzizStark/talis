use anyhow::{Context, Result, ensure};
mod paths;
#[cfg(feature = "assets")]
mod resolution;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    #[default]
    Silver,
    Gold,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderSettings {
    /// Opt-in MATE placement workaround; covers Caja's desktop icons.
    pub experimental_mate_overlay: bool,
    pub fps: u32,
    /// Desktop-only, conservative visibility gate; unknown visibility stays active.
    pub pause_when_hidden: bool,
    pub rotate_artwork: bool,
    /// Hold the displayed arm pose when false; record/artwork rotation is independent.
    pub animate_arm: bool,
    pub finish: Finish,
    pub arm_interpolation: bool,
    /// Approximate visual catch-up response; not a change to song timing.
    pub arm_response_seconds: f64,
    pub arm_range_start: f32,
    pub arm_range_end: f32,
}
impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            experimental_mate_overlay: false,
            fps: 60,
            pause_when_hidden: true,
            rotate_artwork: true,
            animate_arm: true,
            finish: Finish::Silver,
            arm_interpolation: false,
            arm_response_seconds: 0.6,
            arm_range_start: 0.0,
            arm_range_end: 1.0,
        }
    }
}

/// Cached photorealistic scene playback modes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewMode {
    #[default]
    Beauty,
    Arm,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SceneVariant {
    #[default]
    Day,
    Night,
}
impl SceneVariant {
    pub fn id(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Night => "night",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Night => "Night",
        }
    }
}

/// Scene asset detail, not the monitor's display mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum SceneResolution {
    #[default]
    #[serde(rename = "automatic")]
    Automatic,
    #[serde(rename = "1080p")]
    FullHd,
    #[serde(rename = "1440p")]
    Qhd,
    #[serde(rename = "4k")]
    FourK,
}
impl SceneResolution {
    pub fn id(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::FullHd => "1080p",
            Self::Qhd => "1440p",
            Self::FourK => "4k",
        }
    }
}

/// One matched lighting package. Only the selected package is loaded.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScenePackage {
    pub background: PathBuf,
    pub arm_poses: PathBuf,
    pub arm_flow: PathBuf,
    pub sleeve: Option<PathBuf>,
    #[serde(default)]
    pub record_lighting: Option<PathBuf>,
    #[serde(default)]
    pub vinyl_detail: Option<PathBuf>,
    #[serde(default)]
    pub buttons: Option<PathBuf>,
}
impl ScenePackage {
    pub fn at(root: &Path) -> Self {
        Self {
            background: root.join("beauty.png"),
            arm_poses: root.join("arm-poses/atlas.json"),
            arm_flow: root.join("arm-flow/atlas.json"),
            sleeve: Some(root.join("sleeve/atlas.json")),
            record_lighting: Some(root.join("record-lighting/atlas.json")),
            vinyl_detail: Some(root.join("vinyl-detail/atlas.json")),
            buttons: Some(root.join("buttons/atlas.json")),
        }
    }
    pub fn make_absolute(&mut self, directory: &Path) {
        for path in [
            &mut self.background,
            &mut self.arm_poses,
            &mut self.arm_flow,
        ] {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
        if let Some(path) = &mut self.sleeve {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
        if let Some(path) = &mut self.record_lighting {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
        if let Some(path) = &mut self.vinyl_detail {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
        if let Some(path) = &mut self.buttons {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SceneSettings {
    pub mode: PreviewMode,
    pub variant: SceneVariant,
    pub resolution: SceneResolution,
    pub background: PathBuf,
    pub arm_poses: PathBuf,
    pub arm_flow: PathBuf,
    /// Optional baked printed-paper response; never applied to a different beauty.
    pub sleeve: Option<PathBuf>,
    /// Optional stationary lighting for the rotating centre artwork.
    pub record_lighting: Option<PathBuf>,
    pub vinyl_detail: Option<PathBuf>,
    pub buttons: Option<PathBuf>,
    pub night: Option<ScenePackage>,
}
impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            mode: PreviewMode::Beauty,
            variant: SceneVariant::Day,
            resolution: SceneResolution::Automatic,
            background: "assets/scenes/1440p/day/beauty.png".into(),
            arm_poses: "assets/scenes/1440p/day/arm-poses/atlas.json".into(),
            arm_flow: "assets/scenes/1440p/day/arm-flow/atlas.json".into(),
            sleeve: Some("assets/scenes/1440p/day/sleeve/atlas.json".into()),
            record_lighting: Some("assets/scenes/1440p/day/record-lighting/atlas.json".into()),
            vinyl_detail: Some("assets/scenes/1440p/day/vinyl-detail/atlas.json".into()),
            buttons: Some("assets/scenes/1440p/day/buttons/atlas.json".into()),
            night: Some(ScenePackage::at(Path::new("assets/scenes/1440p/night"))),
        }
    }
}
impl SceneSettings {
    pub fn selected_package(&self) -> Result<ScenePackage> {
        match self.variant {
            SceneVariant::Day => Ok(ScenePackage {
                background: self.background.clone(),
                arm_poses: self.arm_poses.clone(),
                arm_flow: self.arm_flow.clone(),
                sleeve: self.sleeve.clone(),
                record_lighting: self.record_lighting.clone(),
                vinyl_detail: self.vinyl_detail.clone(),
                buttons: self.buttons.clone(),
            }),
            SceneVariant::Night => self
                .night
                .clone()
                .context("No night scene package configured"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub minimize_to_tray: bool,
    pub render: RenderSettings,
    pub scene: SceneSettings,
    pub artwork: crate::artwork_geometry::ArtworkSettings,
    pub transport: crate::transport_geometry::TransportSettings,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            minimize_to_tray: true,
            render: RenderSettings::default(),
            scene: SceneSettings::default(),
            artwork: crate::artwork_geometry::ArtworkSettings::default(),
            transport: crate::transport_geometry::TransportSettings::default(),
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::default().overlay(path)
    }
    pub fn overlay(&self, path: &Path) -> Result<Self> {
        let source =
            fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
        let mut overrides: toml::Value =
            toml::from_str(&source).context("Invalid Talis configuration")?;
        if let Some(artwork) = overrides
            .get_mut("artwork")
            .and_then(toml::Value::as_table_mut)
        {
            if let Some(legacy_size) = artwork.remove("diameter_percent") {
                ensure!(
                    !artwork.contains_key("size_percent"),
                    "Duplicate artwork size setting"
                );
                artwork.insert("size_percent".into(), legacy_size);
            }
        }
        let mut values = toml::Value::try_from(self)?;
        merge_config(&mut values, overrides);
        let mut config: Self = values.try_into().context("Invalid Talis configuration")?;
        config.scene.resolve_resources()?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        self.scene.selected_package()?;
        ensure!(
            self.render.arm_range_start.is_finite()
                && self.render.arm_range_end.is_finite()
                && (0.0..=1.0).contains(&self.render.arm_range_start)
                && (self.render.arm_range_start..=1.0).contains(&self.render.arm_range_end),
            "Arm range must satisfy 0 <= start <= end <= 1"
        );
        self.artwork.transform()?;
        self.transport.validate()?;
        ensure!(
            self.render.arm_response_seconds.is_finite()
                && (0.2..=2.0).contains(&self.render.arm_response_seconds),
            "render.arm_response_seconds must be a finite number between 0.2 and 2.0"
        );
        ensure!(
            !self.render.arm_interpolation || self.scene.mode == PreviewMode::Arm,
            "Arm interpolation requires the arm preview mode"
        );
        ensure!(
            (1..=120).contains(&self.render.fps),
            "render.fps must be between 1 and 120"
        );
        Ok(())
    }
}
fn merge_config(base: &mut toml::Value, overrides: toml::Value) {
    if let (Some(base), Some(overrides)) = (base.as_table_mut(), overrides.as_table()) {
        for (key, value) in overrides {
            if let Some(existing) = base.get_mut(key) {
                merge_config(existing, value.clone());
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overrides;
    }
}
#[cfg(test)]
mod tests;
