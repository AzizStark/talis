//! Small, scene-bound arrays of true Blender button positions.
use crate::asset_validation::{self, AssetFile};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Deserialize)]
pub struct ButtonManifest {
    pub format_version: u32,
    pub base_image_sha256: String,
    pub width: u32,
    pub height: u32,
    pub crop: [u32; 4],
    pub steps: u32,
    pub order: [String; 3],
    pub poses: AssetFile,
}
pub struct ButtonAssets {
    pub manifest: ButtonManifest,
    pub pixels: Vec<u8>,
}
impl ButtonAssets {
    #[cfg(test)]
    pub(crate) fn load(path: &Path, beauty: &Path, dimensions: (u32, u32)) -> Result<Self> {
        Self::load_budgeted(path, beauty, dimensions, &mut Default::default())
    }
    pub(crate) fn load_budgeted(
        path: &Path,
        beauty: &Path,
        dimensions: (u32, u32),
        budget: &mut asset_validation::SceneBudget,
    ) -> Result<Self> {
        let m: ButtonManifest = asset_validation::read_manifest(path)?;
        let [x, y, w, h] = m.crop;
        ensure!(
            m.format_version == 1
                && (m.width, m.height) == dimensions
                && m.order == ["play_pause", "previous", "next"]
                && (4..=16).contains(&m.steps)
                && (1..=512).contains(&w)
                && (1..=256).contains(&h)
                && x < m.width
                && y < m.height
                && w <= m.width - x
                && h <= m.height - y,
            "Invalid button atlas geometry/order/pose count"
        );
        ensure!(
            asset_validation::valid_hash(&m.base_image_sha256)
                && asset_validation::hash_file(beauty)?.eq_ignore_ascii_case(&m.base_image_sha256),
            "Button atlas belongs to a different scene beauty"
        );
        let size = asset_validation::buffer_bytes(
            &[u64::from(w), u64::from(h), 4, 1 + 3 * u64::from(m.steps)],
            "button atlas",
        )?;
        budget.reserve(size, "button atlas")?;
        // Check size before allocating/reading a corrupt oversized package.
        let folder = path.parent().unwrap_or(Path::new("."));
        ensure!(
            asset_validation::local_filename(&m.poses.file),
            "Invalid button filename"
        );
        ensure!(
            fs::metadata(folder.join(&m.poses.file))?.len() == size,
            "Incorrect button atlas byte count"
        );
        let pixels = asset_validation::read_hashed_bounded(folder, &m.poses, size)?;
        ensure!(
            pixels.len() as u64 == size && pixels.chunks_exact(4).all(|p| p[3] == 255),
            "Button atlas must contain opaque display-encoded RGBA8"
        );
        Ok(Self {
            manifest: m,
            pixels,
        })
    }
}

pub fn layers(progress: [f32; 3], steps: u32) -> [i32; 3] {
    std::array::from_fn(|i| {
        let p = progress[i];
        let step = if p.is_finite() {
            (p.clamp(0.0, 1.0) * steps as f32).round() as i32
        } else {
            0
        };
        if step == 0 {
            0
        } else {
            i as i32 * steps as i32 + step
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layers_are_bounded_and_each_button_has_its_own_sequence() {
        assert_eq!(layers([0.0, 0.0, 0.0], 8), [0, 0, 0]);
        assert_eq!(layers([1.0, 1.0, 1.0], 8), [8, 16, 24]);
        assert_eq!(layers([0.5, 0.5, 0.5], 8), [4, 12, 20]);
        assert_eq!(layers([f32::NAN, -3.0, f32::INFINITY], 8), [0, 0, 0]);
    }
    #[test]
    fn package_rejects_wrong_beauty_bad_bounds_corruption_and_paths() {
        let f = crate::autostart::Fixture::new();
        let beauty = f.root.join("beauty.png");
        fs::write(&beauty, b"beauty").unwrap();
        let pixels = [10u8, 20, 30, 255].repeat(13);
        fs::write(f.root.join("poses.rgba"), &pixels).unwrap();
        let mut m = serde_json::json!({"format_version":1,"base_image_sha256":asset_validation::sha256(b"beauty"),
            "width":64,"height":32,"crop":[5,6,1,1],"steps":4,"order":["play_pause","previous","next"],
            "poses":{"file":"poses.rgba","sha256":asset_validation::sha256(&pixels)}});
        let manifest = f.root.join("atlas.json");
        let save =
            |m: &serde_json::Value| fs::write(&manifest, serde_json::to_vec(m).unwrap()).unwrap();
        save(&m);
        assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_ok());
        fs::write(&beauty, b"other").unwrap();
        assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_err());
        fs::write(&beauty, b"beauty").unwrap();
        for (field, value) in [("steps", 300), ("width", 0)] {
            let old = m[field].clone();
            m[field] = value.into();
            save(&m);
            assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_err());
            m[field] = old;
        }
        m["crop"][0] = u32::MAX.into();
        save(&m);
        assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_err());
        m["crop"][0] = 5.into();
        m["poses"]["file"] = "../poses.rgba".into();
        save(&m);
        assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_err());
        m["poses"]["file"] = "poses.rgba".into();
        save(&m);
        fs::write(f.root.join("poses.rgba"), vec![0u8; pixels.len()]).unwrap();
        assert!(ButtonAssets::load(&manifest, &beauty, (64, 32)).is_err());
    }
}
