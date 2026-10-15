//! Resolve installation paths and migrate only recognized bundled assets.
use super::SceneSettings;
use anyhow::Result;
use std::path::{Path, PathBuf};

const BUNDLED_FILES: &[&str] = &[
    "beauty.png",
    "arm-poses/atlas.json",
    "arm-flow/atlas.json",
    "sleeve/atlas.json",
    "record-lighting/atlas.json",
    "vinyl-detail/atlas.json",
    "buttons/atlas.json",
];

impl SceneSettings {
    /// Rebase only recognized bundled paths; custom paths remain authoritative.
    pub fn resolve_resources(&mut self) -> Result<()> {
        if let Some(root) = crate::resources::data_directory()? {
            self.migrate_bundled_paths(&root);
            self.resolve_resources_at(&root);
        }
        Ok(())
    }

    pub(super) fn resolve_resources_at(&mut self, root: &Path) {
        self.for_each_path(|path| {
            let Ok(relative) = path.strip_prefix("assets/scenes") else {
                return;
            };
            let mut parts = relative.iter();
            let tier = parts.next().and_then(|v| v.to_str());
            let variant = parts.next().and_then(|v| v.to_str());
            let file = parts.as_path();
            if matches!(tier, Some("1080p" | "1440p" | "4k"))
                && matches!(variant, Some("day" | "night"))
                && BUNDLED_FILES.iter().any(|name| file == Path::new(name))
            {
                *path = root.join(&*path);
            }
        });
    }

    /// Migrate only missing, exact legacy bundled paths. Never redirect a custom
    /// package, an existing legacy file, or a path in another checkout.
    pub fn migrate_bundled_paths(&mut self, root: &Path) {
        let old = root.join("assets/scenes");
        self.for_each_path(|path| {
            for file in BUNDLED_FILES {
                for prefix in ["", "day", "night"] {
                    let relative = Path::new("assets/scenes").join(prefix).join(file);
                    let absolute = old.join(prefix).join(file);
                    let variant = if prefix == "night" { "night" } else { "day" };
                    let new = old.join("1440p").join(variant).join(file);
                    if (*path == relative || *path == absolute)
                        && !path.exists()
                        && !root.join(&*path).exists()
                        && new.is_file()
                    {
                        *path = new;
                        return;
                    }
                }
            }
        });
    }

    fn for_each_path(&mut self, mut apply: impl FnMut(&mut PathBuf)) {
        for path in [
            &mut self.background,
            &mut self.arm_poses,
            &mut self.arm_flow,
        ] {
            apply(path);
        }
        for path in [
            &mut self.sleeve,
            &mut self.record_lighting,
            &mut self.vinyl_detail,
            &mut self.buttons,
        ]
        .into_iter()
        .flatten()
        {
            apply(path);
        }
        if let Some(night) = &mut self.night {
            for path in [
                &mut night.background,
                &mut night.arm_poses,
                &mut night.arm_flow,
            ] {
                apply(path);
            }
            for path in [
                &mut night.sleeve,
                &mut night.record_lighting,
                &mut night.vinyl_detail,
                &mut night.buttons,
            ]
            .into_iter()
            .flatten()
            {
                apply(path);
            }
        }
    }
}
