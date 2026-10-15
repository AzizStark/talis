//! Cropped, fully lit Blender poses for fixed-camera arm animation.
use crate::asset_validation::{decode_image, local_filename, sha256, valid_hash};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct PoseFrame {
    pub file: String,
    pub progress: f32,
    pub angle_degrees: f32,
}
#[derive(Deserialize)]
pub struct PoseManifest {
    pub format_version: u32,
    pub width: u32,
    pub height: u32,
    pub crop: [u32; 4],
    pub frames: Vec<PoseFrame>,
    pub base_image_sha256: String,
}
impl PoseManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.format_version == 1, "Unsupported arm-pose format");
        ensure!(
            valid_hash(&self.base_image_sha256),
            "Invalid base-image SHA-256"
        );
        ensure!(
            self.width > 0 && self.height > 0,
            "Empty base image dimensions"
        );
        let [x, y, w, h] = self.crop;
        ensure!(
            w > 0
                && h > 0
                && x.checked_add(w).is_some_and(|right| right <= self.width)
                && y.checked_add(h).is_some_and(|bottom| bottom <= self.height),
            "Pose crop is outside the base image"
        );
        ensure!(
            (2..=128).contains(&self.frames.len()),
            "Use 2..128 arm poses"
        );
        for (index, frame) in self.frames.iter().enumerate() {
            let expected = index as f32 / (self.frames.len() - 1) as f32;
            ensure!(
                frame.progress.is_finite()
                    && (frame.progress - expected).abs() < 0.00001
                    && frame.angle_degrees.is_finite(),
                "Poses must uniformly cover normalized progress 0..1"
            );
            ensure!(
                local_filename(&frame.file),
                "Pose filenames must be local to the asset folder"
            );
        }
        Ok(())
    }
}
pub struct ArmPoseAssets {
    pub base: image::RgbaImage,
    pub poses: Vec<image::RgbaImage>,
    pub manifest: PoseManifest,
    pub flow: Option<crate::arm_flow::ArmFlowAssets>,
    /// Hash-bound display-only region; pose/flow texture coordinates stay intact.
    pub update_crop: Option<[u32; 4]>,
}
fn display_region(
    bytes: &[u8],
    pose_hash: &str,
    flow_hash: &str,
    bounds: [u32; 4],
) -> Result<[u32; 4]> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Region {
        format_version: u32,
        pose_manifest_sha256: String,
        flow_manifest_sha256: String,
        crop: [u32; 4],
    }
    ensure!(bytes.len() <= 4096, "Arm update region exceeds size limit");
    let r: Region = serde_json::from_slice(bytes)?;
    ensure!(
        r.format_version == 1
            && r.pose_manifest_sha256 == pose_hash
            && r.flow_manifest_sha256 == flow_hash,
        "Arm update region is stale; full pose region retained"
    );
    let [x, y, w, h] = r.crop;
    let [bx, by, bw, bh] = bounds;
    ensure!(
        w > 0
            && h > 0
            && x >= bx
            && y >= by
            && x.checked_add(w)
                .zip(bx.checked_add(bw))
                .is_some_and(|(right, edge)| right <= edge)
            && y.checked_add(h)
                .zip(by.checked_add(bh))
                .is_some_and(|(bottom, edge)| bottom <= edge),
        "Arm update region is outside pose crop"
    );
    Ok(r.crop)
}
impl ArmPoseAssets {
    #[cfg(test)]
    pub(crate) fn load(base: &Path, manifest: &Path) -> Result<Self> {
        Self::load_budgeted(base, manifest, &mut Default::default())
    }
    pub(crate) fn load_budgeted(
        base: &Path,
        manifest: &Path,
        budget: &mut crate::asset_validation::SceneBudget,
    ) -> Result<Self> {
        use crate::asset_validation::{
            MAX_ASSET_BYTES, buffer_bytes, image_bytes, read_bounded, read_manifest,
        };
        let manifest_data: PoseManifest = read_manifest(manifest)?;
        manifest_data.validate()?;
        budget.reserve(
            image_bytes((manifest_data.width, manifest_data.height), 4, "arm beauty")?,
            "arm beauty",
        )?;
        let pose_bytes = image_bytes(
            (manifest_data.crop[2], manifest_data.crop[3]),
            4,
            "arm poses",
        )?;
        budget.reserve(
            buffer_bytes(
                &[pose_bytes, manifest_data.frames.len() as u64],
                "arm pose array",
            )?,
            "arm pose array",
        )?;
        let encoded = read_bounded(base, MAX_ASSET_BYTES)?;
        let hash = sha256(&encoded);
        ensure!(
            hash.eq_ignore_ascii_case(&manifest_data.base_image_sha256),
            "Arm poses do not match the beauty image; replace them with a matched scene package"
        );
        let base = decode_image(
            &encoded,
            (manifest_data.width, manifest_data.height),
            "beauty / arm-pose base",
        )?
        .into_rgba8();
        drop(encoded);
        let folder = manifest
            .parent()
            .context("Arm manifest needs an asset folder")?;
        let mut poses = Vec::with_capacity(manifest_data.frames.len());
        for frame in &manifest_data.frames {
            let path = folder.join(&frame.file);
            let bytes = read_bounded(&path, MAX_ASSET_BYTES)
                .with_context(|| format!("Cannot load {}", path.display()))?;
            let image = decode_image(
                &bytes,
                (manifest_data.crop[2], manifest_data.crop[3]),
                &frame.file,
            )?
            .into_rgba8();
            poses.push(image);
        }
        tracing::info!(poses=poses.len(),crop=?manifest_data.crop,"Loaded fixed-camera arm poses with Blender-rendered shadows/reflections");
        Ok(Self {
            base,
            poses,
            manifest: manifest_data,
            flow: None,
            update_crop: None,
        })
    }
    pub fn load_update_crop(&mut self, poses: &Path, flow: &Path) -> Result<()> {
        let path = poses.with_file_name("update-region.json");
        if !path.exists() || self.flow.is_none() {
            return Ok(());
        }
        let bytes = crate::asset_validation::read_bounded(&path, 4096)?;
        let crop = display_region(
            &bytes,
            &sha256(&crate::asset_validation::read_bounded(
                poses,
                crate::asset_validation::MAX_MANIFEST_BYTES,
            )?),
            &sha256(&crate::asset_validation::read_bounded(
                flow,
                crate::asset_validation::MAX_MANIFEST_BYTES,
            )?),
            self.manifest.crop,
        )?;
        self.update_crop = Some(crop);
        tracing::info!(crop=?crop, "Loaded display-only arm update region; original pose/flow UVs retained");
        Ok(())
    }
}

/// Select one nearest pose, clamping progress and rounding half-steps upward.
/// Holding a whole pose keeps moving edges sharp instead of crossfading two arms.
pub fn nearest_pose_index(progress: f32, count: usize) -> usize {
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let last = count.saturating_sub(1);
    ((progress * last as f32).round() as usize).min(last)
}

/// Adjacent source poses and continuous fraction for motion-compensated warping.
pub fn interpolation_samples(progress: f32, count: usize) -> (usize, usize, f32) {
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let last = count.saturating_sub(1);
    let sample = progress * last as f32;
    let lower = (sample.floor() as usize).min(last);
    (lower, (lower + 1).min(last), sample - lower as f32)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pose_group_budget_rejects_before_opening_or_decoding_any_images() {
        let fixture = crate::autostart::Fixture::new();
        let frames: Vec<_> = (0..5)
            .map(|i| {
                serde_json::json!({
                    "file":format!("pose-{i}.png"), "progress":i as f32 / 4.0, "angle_degrees":0.0,
                })
            })
            .collect();
        let manifest = fixture.root.join("atlas.json");
        let data = serde_json::json!({"format_version":1,"width":8192,"height":4096,
        "crop":[0,0,8192,4096],"base_image_sha256":"0".repeat(64),"frames":frames});
        std::fs::write(&manifest, serde_json::to_vec(&data).unwrap()).unwrap();
        let error = match ArmPoseAssets::load(&fixture.root.join("does-not-exist.png"), &manifest) {
            Ok(_) => panic!("oversized pose array accepted"),
            Err(error) => error,
        };
        assert!(
            error
                .downcast_ref::<crate::asset_validation::ResourceLimit>()
                .is_some(),
            "{error:#}"
        );
        assert!(error.to_string().contains("arm pose array"));
    }
    #[test]
    fn display_crop_rejects_stale_corrupt_empty_outside_and_overflow_regions() {
        let mut r = serde_json::json!({"format_version":1,"pose_manifest_sha256":"a".repeat(64),
        "flow_manifest_sha256":"b".repeat(64),"crop":[1160,585,600,630]});
        let check = |v: &serde_json::Value| {
            display_region(
                &serde_json::to_vec(v).unwrap(),
                &"a".repeat(64),
                &"b".repeat(64),
                [1160, 585, 960, 630],
            )
        };
        assert_eq!(check(&r).unwrap(), [1160, 585, 600, 630]);
        assert!(display_region(b"{bad", "", "", [0, 0, 1, 1]).is_err());
        assert!(display_region(&vec![b' '; 4097], "", "", [0, 0, 1, 1]).is_err());
        r["format_version"] = 2.into();
        assert!(check(&r).is_err());
        r["format_version"] = 1.into();
        r["pose_manifest_sha256"] = "stale".into();
        assert!(check(&r).is_err());
        r["pose_manifest_sha256"] = "a".repeat(64).into();
        r["flow_manifest_sha256"] = "stale".into();
        assert!(check(&r).is_err());
        r["flow_manifest_sha256"] = "b".repeat(64).into();
        for crop in [
            [1160, 585, 0, 630],
            [1159, 585, 600, 630],
            [1160, 584, 600, 630],
            [1160, 585, 961, 630],
            [1160, 585, 600, 631],
            [u32::MAX, 585, 600, 630],
        ] {
            r["crop"] = serde_json::json!(crop);
            assert!(check(&r).is_err());
        }
    }
    #[test]
    fn interpolation_is_continuous_and_clamped() {
        assert_eq!(interpolation_samples(0.515625, 33), (16, 17, 0.5));
        assert_eq!(interpolation_samples(0.0, 33), (0, 1, 0.0));
        assert_eq!(interpolation_samples(1.0, 33), (32, 32, 0.0));
        assert_eq!(interpolation_samples(f32::NAN, 33), (0, 1, 0.0));
        assert_eq!(interpolation_samples(-1.0, 33), (0, 1, 0.0));
        assert_eq!(interpolation_samples(2.0, 33), (32, 32, 0.0));
        assert_eq!(interpolation_samples(0.5, 0), (0, 0, 0.0));
    }
    fn manifest() -> PoseManifest {
        PoseManifest {
            format_version: 1,
            width: 2560,
            height: 1440,
            crop: [1160, 540, 960, 675],
            frames: vec![
                PoseFrame {
                    file: "pose-0.png".into(),
                    progress: 0.0,
                    angle_degrees: 7.0,
                },
                PoseFrame {
                    file: "pose-1.png".into(),
                    progress: 1.0,
                    angle_degrees: -16.0,
                },
            ],
            base_image_sha256: "0".repeat(64),
        }
    }
    #[test]
    fn endpoints_and_midpoint_are_exact() {
        assert_eq!(nearest_pose_index(0.0, 33), 0);
        assert_eq!(nearest_pose_index(1.0, 33), 32);
        assert_eq!(nearest_pose_index(0.5, 33), 16);
        for count in [2, 33, 128] {
            for index in 0..count {
                assert_eq!(
                    nearest_pose_index(index as f32 / (count - 1) as f32, count),
                    index
                );
            }
        }
    }
    #[test]
    fn intermediate_positions_hold_one_pose_and_switch_at_half_steps() {
        assert_eq!(nearest_pose_index(0.515, 33), 16);
        assert_eq!(nearest_pose_index(0.515625, 33), 17);
        assert_eq!(nearest_pose_index(0.52, 33), 17);
        assert_eq!(nearest_pose_index(0.249, 3), 0);
        assert_eq!(nearest_pose_index(0.25, 3), 1);
        assert_eq!(nearest_pose_index(0.749, 3), 1);
        assert_eq!(nearest_pose_index(0.75, 3), 2);
    }
    #[test]
    fn pose_selection_clamps_invalid_progress_and_degenerate_counts() {
        assert_eq!(nearest_pose_index(-1.0, 33), 0);
        assert_eq!(nearest_pose_index(2.0, 33), 32);
        for progress in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(nearest_pose_index(progress, 33), 0);
        }
        assert_eq!(nearest_pose_index(0.5, 0), 0);
        assert_eq!(nearest_pose_index(1.0, 1), 0);
    }
    #[test]
    fn malformed_crop_and_external_frame_path_are_rejected() {
        let mut m = manifest();
        assert!(m.validate().is_ok());
        m.crop[0] = u32::MAX;
        assert!(m.validate().is_err());
        let mut m = manifest();
        m.frames[0].file = "../pose.png".into();
        assert!(m.validate().is_err());
        let mut m = manifest();
        m.frames[1].progress = 0.5;
        assert!(m.validate().is_err());
    }
}
