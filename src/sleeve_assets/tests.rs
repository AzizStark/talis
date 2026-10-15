use super::*;
use crate::asset_validation::sha256;
#[test]
fn paper_light_reference_ignores_edges_zeroes_and_is_scene_relative() {
    assert_eq!(positive_half(&0x0001u16.to_ne_bytes()), 2.0_f32.powi(-24));
    assert_eq!(positive_half(&0x3c00u16.to_ne_bytes()), 1.0);
    assert_eq!(positive_half(&0x7bffu16.to_ne_bytes()), 65504.0);
    let mut pixels = Vec::new();
    for bits in std::iter::repeat_n(0x3800u16, 10).chain([0x7bff, 0]) {
        let alpha = if bits == 0x7bff { 0x3800 } else { 0x3c00 };
        pixels.extend(
            [bits, bits, bits, alpha]
                .into_iter()
                .flat_map(u16::to_ne_bytes),
        );
    }
    assert!(
        light_reference(&pixels)
            .iter()
            .all(|c| (*c - 0.5).abs() < 1e-6)
    );
    assert_eq!(light_reference(&[]), [0.0; 3]);
    assert_eq!(light_reference(&[0; 8]), [0.0; 3]);
}
#[test]
fn paper_reference_retains_warm_tint_and_ignores_bright_outliers() {
    let mut pixels = Vec::new();
    for _ in 0..100 {
        pixels.extend(
            [0x3800u16, 0x3400, 0x3000, 0x3c00]
                .into_iter()
                .flat_map(u16::to_ne_bytes),
        );
    }
    pixels.extend(
        [0x7bffu16; 3]
            .into_iter()
            .chain([0x3c00])
            .flat_map(u16::to_ne_bytes),
    );
    for (actual, expected) in light_reference(&pixels).into_iter().zip([0.5, 0.25, 0.125]) {
        assert!((actual - expected).abs() < 1e-6);
    }
}
use std::sync::atomic::{AtomicU64, Ordering};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: std::path::PathBuf,
    manifest: SleeveManifest,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "talis-sleeve-assets-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("beauty.png"), b"test beauty").unwrap();
        let half = [0x3c00u16, 0x4000, 0x4200, 0x3c00]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .cycle()
            .take(32)
            .collect::<Vec<_>>();
        fs::write(root.join("illumination.bin"), &half).unwrap();
        fs::write(root.join("residual.bin"), &half).unwrap();
        fs::write(root.join("view.bin"), vec![0; 16 * 16 * 16 * 4]).unwrap();
        let file = |name: &str| AssetFile {
            file: name.into(),
            sha256: sha256(&fs::read(root.join(name)).unwrap()),
        };
        let mut manifest = manifest();
        manifest.width = 10;
        manifest.height = 10;
        manifest.crop = [0, 0, 2, 2];
        manifest.lut_edge = 16;
        manifest.base_image_sha256 = sha256(b"test beauty");
        manifest.illumination = file("illumination.bin");
        manifest.residual = file("residual.bin");
        manifest.view_lut = file("view.bin");
        let fixture = Self { root, manifest };
        fixture.save();
        fixture
    }
    fn save(&self) {
        fs::write(
            self.root.join("atlas.json"),
            serde_json::to_vec(&self.manifest).unwrap(),
        )
        .unwrap();
    }
    fn load(&self) -> Result<SleeveAssets> {
        SleeveAssets::load(
            &self.root.join("atlas.json"),
            &self.root.join("beauty.png"),
            (10, 10),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn manifest() -> SleeveManifest {
    let file = || AssetFile {
        file: "lighting.bin".into(),
        sha256: "a".repeat(64),
    };
    SleeveManifest {
        format_version: 1,
        response_kind: None,
        base_image_sha256: "b".repeat(64),
        width: 2560,
        height: 1440,
        crop: [60, 405, 700, 675],
        corners: [
            [0.032, 0.305],
            [0.275, 0.286],
            [0.288, 0.691],
            [0.052, 0.721],
        ],
        illumination: file(),
        residual: file(),
        view_lut: file(),
        lut_edge: 96,
        lut_log_min: -16.0,
        lut_log_max: 6.0,
    }
}
#[test]
fn record_and_sleeve_responses_cannot_be_confused() {
    let mut fixture = Fixture::new();
    let path = fixture.root.join("atlas.json");
    let beauty = fixture.root.join("beauty.png");
    assert!(SleeveAssets::load_record(&path, &beauty, (10, 10)).is_err());
    fixture.manifest.response_kind = Some("record_lighting".into());
    fixture.save();
    assert!(fixture.load().is_err());
    assert!(SleeveAssets::load_record(&path, &beauty, (10, 10)).is_ok());
    fs::write(&beauty, b"stale beauty").unwrap();
    assert!(SleeveAssets::load_record(&path, &beauty, (10, 10)).is_err());
}
#[test]
fn fit_and_limits_are_validated() {
    let mut m = manifest();
    assert!(m.validate((2560, 1440)).is_ok());
    assert!(m.validate((1920, 1080)).is_err());
    m.crop[0] = u32::MAX;
    assert!(m.validate((2560, 1440)).is_err());
    m = manifest();
    m.lut_edge = 129;
    assert!(m.validate((2560, 1440)).is_err());
    m = manifest();
    m.lut_log_min = f32::NAN;
    assert!(m.validate((2560, 1440)).is_err());
    m = manifest();
    m.corners.swap(0, 2);
    assert!(m.validate((2560, 1440)).is_err());
    m = manifest();
    m.illumination.file = "../other.bin".into();
    assert!(m.validate((2560, 1440)).is_err());
}
#[test]
fn half_float_buffers_reject_nonfinite_negative_and_invalid_coverage() {
    let rgba = |bits: [u16; 4]| {
        bits.into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    assert!(validate_half(&mut rgba([0, 0x3c00, 0x4000, 0x3c00])).is_ok());
    for bits in [
        [0x7c00, 0, 0, 0],
        [0x7e00, 0, 0, 0],
        [0x8001, 0, 0, 0],
        [0, 0, 0, 0x4000],
    ] {
        assert!(validate_half(&mut rgba(bits)).is_err());
    }
    assert!(validate_half(&mut [0; 7]).is_err());
}
#[test]
fn complete_package_loads_and_rejects_stale_beauty() {
    let fixture = Fixture::new();
    let assets = fixture.load().unwrap();
    assert_eq!(assets.illumination.len(), 32);
    assert_eq!(
        u16::from_ne_bytes(assets.illumination[..2].try_into().unwrap()),
        0x3c00
    );
    fs::write(fixture.root.join("beauty.png"), b"different scene").unwrap();
    assert!(fixture.load().is_err());
}
#[test]
fn corrupt_truncated_and_nonfinite_packages_are_rejected() {
    let mut fixture = Fixture::new();
    fs::write(fixture.root.join("view.bin"), vec![1; 16 * 16 * 16 * 4]).unwrap();
    assert!(fixture.load().is_err()); // Same size, wrong hash.
    fs::write(fixture.root.join("view.bin"), [0]).unwrap();
    assert!(fixture.load().is_err());
    fs::write(fixture.root.join("view.bin"), vec![0; 16 * 16 * 16 * 4]).unwrap();
    let mut half = fs::read(fixture.root.join("illumination.bin")).unwrap();
    half[..2].copy_from_slice(&0x7e00u16.to_le_bytes());
    fs::write(fixture.root.join("illumination.bin"), &half).unwrap();
    fixture.manifest.illumination.sha256 = sha256(&half);
    fixture.save();
    assert!(fixture.load().is_err()); // Correct hash, NaN payload.
}
