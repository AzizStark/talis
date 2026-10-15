//! Validated authored premultiplied decal plus scene-bound fixed lighting.
use crate::{
    asset_validation::AssetFile,
    sleeve_assets::{SleeveAssets, bounded_file},
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;

pub struct VinylAssets {
    pub response: SleeveAssets,
    pub detail: Vec<u8>,
    pub size: u32,
    pub baked_detail: bool,
}
#[derive(Deserialize)]
struct DetailManifest {
    detail: AssetFile,
    detail_size: u32,
    baked_detail: bool,
}
impl VinylAssets {
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
        let manifest: DetailManifest = crate::asset_validation::read_manifest(path)?;
        ensure!(
            (64..=2048).contains(&manifest.detail_size),
            "Invalid vinyl detail size"
        );
        let bytes = crate::asset_validation::image_bytes(
            (manifest.detail_size, manifest.detail_size),
            4,
            "vinyl detail",
        )?;
        budget.reserve(bytes, "vinyl detail")?;
        let response =
            SleeveAssets::load_budgeted(path, beauty, dimensions, "vinyl_detail", budget)?;
        ensure!(
            response
                .illumination
                .chunks_exact(8)
                .filter(|p| u16::from_ne_bytes([p[6], p[7]]) >= 0x3bff)
                .count()
                > 1000,
            "Vinyl calibration contains no visible surface"
        );
        let detail = bounded_file(path.parent().unwrap(), &manifest.detail, bytes as usize)?;
        ensure!(
            detail
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|&v| v <= p[3])),
            "Vinyl detail must be linear premultiplied RGBA"
        );
        Ok(Self {
            response,
            detail,
            size: manifest.detail_size,
            baked_detail: manifest.baked_detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_validation::sha256;
    use std::fs;
    #[test]
    fn surface_package_rejects_stale_corrupt_unpremultiplied_and_hidden_calibrations() {
        let fixture = crate::autostart::Fixture::new();
        let root = &fixture.root;
        let beauty = root.join("beauty.png");
        fs::write(&beauty, b"beauty fixture").unwrap();
        let half: [u8; 8] = [0, 0x38, 0, 0x38, 0, 0x38, 0, 0x3c];
        let response = half.repeat(40 * 40);
        let detail = vec![0u8; 64 * 64 * 4];
        let lut = vec![0u8; 16 * 16 * 16 * 4];
        for (name, bytes) in [
            ("illum.bin", &response),
            ("base.bin", &response),
            ("detail.bin", &detail),
            ("view.bin", &lut),
        ] {
            fs::write(root.join(name), bytes).unwrap();
        }
        let file =
            |name: &str, bytes: &[u8]| serde_json::json!({"file":name,"sha256":sha256(bytes)});
        let mut m = serde_json::json!({"format_version":1,"response_kind":"vinyl_detail",
            "base_image_sha256":sha256(b"beauty fixture"),"width":64,"height":64,"crop":[0,0,40,40],
            "corners":[[0.1,0.1],[0.9,0.1],[0.9,0.9],[0.1,0.9]],
            "illumination":file("illum.bin",&response),"residual":file("base.bin",&response),
            "view_lut":file("view.bin",&lut),"lut_edge":16,"lut_log_min":-16,"lut_log_max":6,
            "detail":file("detail.bin",&detail),"detail_size":64,"baked_detail":false});
        let manifest = root.join("atlas.json");
        let save =
            |m: &serde_json::Value| fs::write(&manifest, serde_json::to_vec(m).unwrap()).unwrap();
        let load = || VinylAssets::load(&manifest, &beauty, (64, 64));
        save(&m);
        assert!(load().is_ok());
        assert!(SleeveAssets::load_record(&manifest, &beauty, (64, 64)).is_err());
        fs::write(&beauty, b"different").unwrap();
        assert!(load().is_err());
        fs::write(&beauty, b"beauty fixture").unwrap();
        fs::write(root.join("detail.bin"), [0]).unwrap();
        assert!(load().is_err());
        let mut bad = detail.clone();
        bad[0] = 255;
        fs::write(root.join("detail.bin"), &bad).unwrap();
        m["detail"] = file("detail.bin", &bad);
        save(&m);
        assert!(load().is_err());
        fs::write(root.join("detail.bin"), &detail).unwrap();
        m["detail"] = file("detail.bin", &detail);
        m["detail_size"] = 4096.into();
        save(&m);
        assert!(load().is_err());
        m["detail_size"] = 64.into();
        m["detail"]["file"] = "../detail.bin".into();
        save(&m);
        assert!(load().is_err());
        m["detail"] = file("detail.bin", &detail);
        let hidden = vec![0u8; 40 * 40 * 8];
        fs::write(root.join("illum.bin"), &hidden).unwrap();
        m["illumination"] = file("illum.bin", &hidden);
        save(&m);
        assert!(load().is_err());
    }
}
