//! Optional authored cover-OFF beauty, tied to the exact covered scene.
use crate::asset_validation::{AssetFile, decode_image, valid_hash};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub width: u32,
    pub height: u32,
    pub base_image_sha256: String,
    pub source_sha256: String,
    pub cover_visible: bool,
    pub image: AssetFile,
}
pub struct StandAssets {
    pub image: image::RgbaImage,
}
impl StandAssets {
    #[cfg(test)]
    pub(crate) fn load(path: &Path, beauty: &Path, dimensions: (u32, u32)) -> Result<Self> {
        Self::load_budgeted(path, beauty, dimensions, &mut Default::default())
    }
    pub(crate) fn load_budgeted(
        path: &Path,
        beauty: &Path,
        dimensions: (u32, u32),
        budget: &mut crate::asset_validation::SceneBudget,
    ) -> Result<Self> {
        let manifest: Manifest = crate::asset_validation::read_manifest(path)?;
        ensure!(
            manifest.format_version == 1
                && !manifest.cover_visible
                && (manifest.width, manifest.height) == dimensions
                && dimensions.0 > 0
                && dimensions.1 > 0,
            "Invalid empty-stand state/version/dimensions"
        );
        ensure!(
            valid_hash(&manifest.source_sha256)
                && valid_hash(&manifest.base_image_sha256)
                && crate::asset_validation::hash_file(beauty)? == manifest.base_image_sha256,
            "Empty stand belongs to another beauty"
        );
        budget.reserve(
            crate::asset_validation::image_bytes(dimensions, 4, "empty stand")?,
            "empty stand",
        )?;
        let bytes = crate::asset_validation::read_hashed_bounded(
            path.parent().unwrap(),
            &manifest.image,
            64 * 1024 * 1024,
        )?;
        Ok(Self {
            image: decode_image(&bytes, dimensions, "empty stand")?.into_rgba8(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_validation::sha256;
    use std::fs;
    #[test]
    fn stand_requires_matched_beauty_off_state_and_intact_local_image() {
        let fixture = crate::autostart::Fixture::new();
        let beauty = fixture.root.join("beauty.png");
        let empty = fixture.root.join("empty.png");
        image::RgbaImage::from_pixel(16, 9, image::Rgba([10, 20, 30, 255]))
            .save(&beauty)
            .unwrap();
        image::RgbaImage::from_pixel(16, 9, image::Rgba([40, 50, 60, 255]))
            .save(&empty)
            .unwrap();
        let manifest = serde_json::json!({
            "format_version":1,"width":16,"height":9,"cover_visible":false,
            "source_sha256":"1".repeat(64),
            "base_image_sha256":sha256(&fs::read(&beauty).unwrap()),
            "image":{"file":"empty.png","sha256":sha256(&fs::read(&empty).unwrap())}
        });
        let path = fixture.root.join("atlas.json");
        let load = |m: &serde_json::Value| {
            fs::write(&path, serde_json::to_vec(m).unwrap()).unwrap();
            StandAssets::load(&path, &beauty, (16, 9))
        };
        assert_eq!(
            load(&manifest).unwrap().image.get_pixel(0, 0).0,
            [40, 50, 60, 255]
        );
        for (key, value) in [
            ("format_version", serde_json::json!(2)),
            ("width", serde_json::json!(15)),
            ("cover_visible", serde_json::json!(true)),
            ("source_sha256", serde_json::json!("bad")),
            ("base_image_sha256", serde_json::json!("0".repeat(64))),
        ] {
            let mut bad = manifest.clone();
            bad[key] = value;
            assert!(load(&bad).is_err(), "{key}");
        }
        let mut bad = manifest.clone();
        bad["image"]["file"] = serde_json::json!("../empty.png");
        assert!(load(&bad).is_err());
        fs::write(&empty, b"corrupt").unwrap();
        assert!(load(&manifest).is_err());
    }
}
