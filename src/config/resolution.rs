//! Select only matched bundled tiers; never rewrite custom scene paths.
use super::{ScenePackage, SceneResolution, SceneSettings};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

const PACKAGE_FILES: &[&str] = &[
    "beauty.png",
    "beauty.json",
    "arm-guide.json",
    "arm-poses/atlas.json",
    "arm-flow/atlas.json",
    "sleeve/atlas.json",
    "stack-sleeve/atlas.json",
    "empty-stand/atlas.json",
    "record-lighting/atlas.json",
    "vinyl-detail/atlas.json",
    "buttons/atlas.json",
    "knob/atlas.json",
];

fn absolute(mut package: ScenePackage, root: &Path) -> ScenePackage {
    package.make_absolute(root);
    package
}

impl SceneSettings {
    pub fn bundled_resolution_available(&self) -> bool {
        self.bundled_root().is_some()
    }

    fn bundled_root(&self) -> Option<PathBuf> {
        let roots = [
            std::env::current_dir().ok(),
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        ];
        roots
            .into_iter()
            .flatten()
            .find(|root| self.is_bundled_at(root))
    }

    fn is_bundled_at(&self, root: &Path) -> bool {
        ["1440p", "1080p", "4k"]
            .into_iter()
            .any(|tier| self.matches_tier(root, tier))
    }

    /// Automatic falls back to 1440p if its optional tier is absent.
    /// Explicit optional tiers fail without changing the current paths.
    /// Dimensions include monitor scale. Called at launch or an explicit edit,
    /// never from animation callbacks or ordinary window resizing.
    pub fn resolve_bundled_resolution(&mut self, width: i32, height: i32) -> Result<bool> {
        let Some(root) = self.bundled_root() else {
            return Ok(false); // Custom scene paths remain authoritative.
        };
        self.resolve_at(&root, width, height)
    }

    fn matches_tier(&self, root: &Path, tier: &str) -> bool {
        let day = ScenePackage {
            background: self.background.clone(),
            arm_poses: self.arm_poses.clone(),
            arm_flow: self.arm_flow.clone(),
            sleeve: self.sleeve.clone(),
            record_lighting: self.record_lighting.clone(),
            vinyl_detail: self.vinyl_detail.clone(),
            buttons: self.buttons.clone(),
        };
        let base = root.join("assets/scenes").join(tier);
        absolute(day, root) == ScenePackage::at(&base.join("day"))
            && self.night.clone().map(|n| absolute(n, root))
                == Some(ScenePackage::at(&base.join("night")))
    }

    fn resolve_at(&mut self, root: &Path, width: i32, height: i32) -> Result<bool> {
        if !self.is_bundled_at(root) {
            return Ok(false);
        }
        let base = root.join("assets/scenes");
        let requested = match self.resolution {
            SceneResolution::FullHd => Some(("1080p", [1920, 1080])),
            SceneResolution::FourK => Some(("4k", [3840, 2160])),
            SceneResolution::Automatic if width > 2560 && height > 1440 => {
                Some(("4k", [3840, 2160]))
            }
            SceneResolution::Automatic
                if width > 0 && height > 0 && width <= 1920 && height <= 1080 =>
            {
                Some(("1080p", [1920, 1080]))
            }
            _ => None,
        };
        let name = if let Some((name, dimensions)) = requested {
            if Self::complete_tier(&base.join(name), dimensions) {
                name
            } else {
                ensure!(
                    self.resolution == SceneResolution::Automatic,
                    "The complete {name} day/night scene pair is unavailable"
                );
                "1440p"
            }
        } else {
            "1440p"
        };
        let tier = base.join(name);
        if self.matches_tier(root, name) {
            return Ok(false);
        }
        let package = ScenePackage::at(&tier.join("day"));
        self.background = package.background;
        self.arm_poses = package.arm_poses;
        self.arm_flow = package.arm_flow;
        self.sleeve = package.sleeve;
        self.record_lighting = package.record_lighting;
        self.vinyl_detail = package.vinyl_detail;
        self.buttons = package.buttons;
        self.night = Some(ScenePackage::at(&tier.join("night")));
        Ok(true)
    }

    fn complete_tier(tier: &Path, dimensions: [i32; 2]) -> bool {
        let receipt = tier.join("resolution.json");
        let verified_resolution = std::fs::metadata(&receipt)
            .ok()
            .filter(|m| m.is_file() && m.len() <= 65_536)
            .and_then(|_| std::fs::read(&receipt).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|v| v.get("resolution") == Some(&serde_json::json!(dimensions)));
        verified_resolution
            && ["day", "night"]
                .into_iter()
                .all(|v| PACKAGE_FILES.iter().all(|p| tier.join(v).join(p).is_file()))
    }

    #[cfg(test)]
    fn select_4k_at(&mut self, root: &Path) -> bool {
        self.resolve_at(root, 3840, 2160).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests;
