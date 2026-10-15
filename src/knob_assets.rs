//! Optional fixed-camera knob patches, bound to the exact scene beauty.
use crate::asset_validation::{AssetFile, decode_image, read_hashed};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;

// Preserve the previous 256x256 pixel/memory ceiling while allowing taller
// 4K border-grid padding. Each axis remains bounded independently as well.
const MAX_PATCH_AXIS: u32 = 512;
const MAX_PATCH_PIXELS: u32 = 256 * 256;

fn valid_crop([x, y, w, h]: [u32; 4], width: u32, height: u32) -> bool {
    w > 0
        && h > 0
        && w <= MAX_PATCH_AXIS
        && h <= MAX_PATCH_AXIS
        && w.checked_mul(h)
            .is_some_and(|pixels| pixels <= MAX_PATCH_PIXELS)
        && x.checked_add(w).is_some_and(|v| v <= width)
        && y.checked_add(h).is_some_and(|v| v <= height)
}

#[derive(Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub width: u32,
    pub height: u32,
    pub crop: [u32; 4],
    pub corners: [[f64; 2]; 4],
    pub marker_width: f32,
    pub marker_inner: f32,
    pub marker_outer: f32,
    pub base_image_sha256: String,
    pub clean: AssetFile,
    pub ink: AssetFile,
}

pub struct KnobAssets {
    pub manifest: Manifest,
    pub transform: [f32; 9],
    pub clean: image::RgbaImage,
    pub ink: image::RgbaImage,
}
impl KnobAssets {
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
        let m: Manifest = crate::asset_validation::read_manifest(path)?;
        let [x, y, w, h] = m.crop;
        ensure!(
            m.format_version == 1 && (m.width, m.height) == dimensions,
            "Knob dimensions/version mismatch"
        );
        ensure!(valid_crop(m.crop, m.width, m.height), "Invalid knob crop");
        ensure!(
            crate::asset_validation::hash_file(beauty)? == m.base_image_sha256,
            "Knob belongs to another scene beauty"
        );
        ensure!(
            [m.marker_width, m.marker_inner, m.marker_outer]
                .iter()
                .all(|v| v.is_finite())
                && m.marker_width > 0.0
                && m.marker_width < 0.2
                && m.marker_inner >= 0.0
                && m.marker_inner < m.marker_outer
                && m.marker_outer < 0.95,
            "Invalid marker geometry"
        );
        ensure!(
            m.corners.iter().all(|p| p[0] >= x as f64 / m.width as f64
                && p[0] <= (x + w) as f64 / m.width as f64
                && p[1] >= y as f64 / m.height as f64
                && p[1] <= (y + h) as f64 / m.height as f64),
            "Knob plane outside its crop"
        );
        let transform = crate::artwork_geometry::ArtworkSettings {
            corners: m.corners,
            size_percent: 100.0,
            ..Default::default()
        }
        .transform()?;
        let folder = path.parent().unwrap();
        budget.reserve(
            crate::asset_validation::image_bytes((w, h), 8, "knob patches")?,
            "knob patches",
        )?;
        let clean =
            decode_image(&read_hashed(folder, &m.clean)?, (w, h), "clean knob")?.into_rgba8();
        let ink = decode_image(&read_hashed(folder, &m.ink)?, (w, h), "knob ink")?.into_rgba8();
        Ok(Self {
            manifest: m,
            transform,
            clean,
            ink,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_validation::sha256;
    use std::fs;
    #[test]
    fn padded_4k_crops_preserve_the_existing_pixel_budget() {
        assert!(valid_crop([2370, 1215, 120, 270], 3840, 2160));
        assert!(valid_crop([0, 0, 256, 256], 3840, 2160));
        assert!(valid_crop([0, 0, 512, 128], 3840, 2160));
        for crop in [
            [0, 0, 257, 256],
            [0, 0, 512, 129],
            [0, 0, 513, 1],
            [0, 0, 1, 513],
            [0, 0, 0, 270],
            [3800, 1215, 120, 270],
            [2370, 2000, 120, 270],
            [u32::MAX, 0, 120, 270],
        ] {
            assert!(!valid_crop(crop, 3840, 2160), "{crop:?}");
        }
    }

    #[test]
    fn padded_knob_images_load_without_downsampling() {
        let f = crate::autostart::Fixture::new();
        let beauty = f.root.join("beauty.png");
        let patch = f.root.join("patch.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([12, 24, 36, 255]))
            .save(&beauty)
            .unwrap();
        image::RgbaImage::from_pixel(120, 270, image::Rgba([12, 24, 36, 255]))
            .save(&patch)
            .unwrap();
        let file =
            serde_json::json!({"file":"patch.png","sha256":sha256(&fs::read(&patch).unwrap())});
        let corners = [
            [2370., 1215.],
            [2490., 1215.],
            [2490., 1485.],
            [2370., 1485.],
        ]
        .map(|[x, y]| [x / 3840., y / 2160.]);
        let good = serde_json::json!({"format_version":1,"width":3840,"height":2160,
            "crop":[2370,1215,120,270],"corners":corners,
            "marker_width":0.04,"marker_inner":0.3,"marker_outer":0.6,
            "base_image_sha256":sha256(&fs::read(&beauty).unwrap()),"clean":file,"ink":file});
        let path = f.root.join("atlas.json");
        fs::write(&path, serde_json::to_vec(&good).unwrap()).unwrap();
        let loaded = KnobAssets::load(&path, &beauty, (3840, 2160)).unwrap();
        assert_eq!(loaded.clean.dimensions(), (120, 270));
        assert_eq!(loaded.ink.dimensions(), (120, 270));
    }
    #[test]
    fn package_is_hash_bound_and_rejects_invalid_geometry_and_files() {
        let f = crate::autostart::Fixture::new();
        let beauty = f.root.join("beauty.png");
        let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([12, 24, 36, 255]));
        image.save(&beauty).unwrap();
        let patch = f.root.join("patch.png");
        image::imageops::crop_imm(&image, 0, 0, 4, 4)
            .to_image()
            .save(&patch)
            .unwrap();
        let file =
            serde_json::json!({"file":"patch.png","sha256":sha256(&fs::read(&patch).unwrap())});
        let good = serde_json::json!({"format_version":1,"width":16,"height":16,"crop":[4,4,4,4],
            "corners":[[0.25,0.25],[0.5,0.25],[0.5,0.5],[0.25,0.5]],
            "marker_width":0.04,"marker_inner":0.3,"marker_outer":0.6,
            "base_image_sha256":sha256(&fs::read(&beauty).unwrap()),"clean":file,"ink":file});
        let path = f.root.join("atlas.json");
        let load = |value: &serde_json::Value| {
            fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            KnobAssets::load(&path, &beauty, (16, 16))
        };
        let loaded = load(&good).unwrap();
        let m = loaded.transform;
        for (p, expected) in
            loaded
                .manifest
                .corners
                .iter()
                .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
        {
            let x = p[0] as f32;
            let y = p[1] as f32;
            let z = m[2] * x + m[5] * y + m[8];
            assert!(((m[0] * x + m[3] * y + m[6]) / z - expected[0]).abs() < 1e-5);
            assert!(((m[1] * x + m[4] * y + m[7]) / z - expected[1]).abs() < 1e-5);
        }
        for (key, value) in [
            ("format_version", serde_json::json!(2)),
            ("crop", serde_json::json!([15, 4, 4, 4])),
            ("marker_width", serde_json::json!(-0.5)),
            ("marker_outer", serde_json::json!(1.2)),
            ("base_image_sha256", serde_json::json!("0".repeat(64))),
            ("corners", serde_json::json!(vec![[0.0, 0.0]; 4])),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(load(&bad).is_err(), "{key}");
        }
        let mut bad = good.clone();
        bad["clean"]["file"] = serde_json::json!("../patch.png");
        assert!(load(&bad).is_err());
        bad = good.clone();
        bad["ink"]["sha256"] = serde_json::json!("0".repeat(64));
        assert!(load(&bad).is_err());
        fs::write(&patch, b"corrupt").unwrap();
        assert!(load(&good).is_err());
    }
}
