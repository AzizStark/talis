//! Optional, hash-bound bidirectional motion fields for live pose-image warping.
use crate::{
    arm_assets::PoseManifest,
    asset_validation::{decode_image, local_filename, read_hashed, sha256, valid_hash},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Backward-compatible name for a hash-bound package file.
pub type FlowPair = crate::asset_validation::AssetFile;
#[derive(Deserialize, Serialize)]
pub struct EdgeLayersManifest {
    pub masks: Vec<FlowPair>,
    pub background: FlowPair,
}
#[derive(Deserialize, Serialize)]
pub struct FlowManifest {
    pub format_version: u32,
    pub width: u32,
    pub height: u32,
    pub crop_width: u32,
    pub crop_height: u32,
    pub encoding: String,
    pub pose_manifest_sha256: String,
    pub pose_images_sha256: Vec<String>,
    pub pairs: Vec<FlowPair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_layers: Option<EdgeLayersManifest>,
}
impl FlowManifest {
    pub fn validate(&self, poses: &PoseManifest) -> Result<()> {
        poses.validate()?;
        ensure!(
            self.format_version == 1 && self.encoding == "rgba16_unorm_bias32768_scale256",
            "Unsupported arm-flow encoding"
        );
        ensure!(
            self.crop_width == poses.crop[2]
                && self.crop_height == poses.crop[3]
                && self.width > 0
                && self.height > 0
                && self.width <= self.crop_width
                && self.height <= self.crop_height,
            "Arm-flow dimensions do not match the pose crop"
        );
        ensure!(
            self.pairs.len() + 1 == poses.frames.len()
                && self.pose_images_sha256.len() == poses.frames.len(),
            "Arm-flow pair count does not match the poses"
        );
        ensure!(
            valid_hash(&self.pose_manifest_sha256)
                && self.pose_images_sha256.iter().all(|hash| valid_hash(hash)),
            "Invalid arm-flow source hashes"
        );
        for pair in &self.pairs {
            ensure!(
                local_filename(&pair.file) && valid_hash(&pair.sha256),
                "Invalid arm-flow filename or hash"
            );
        }
        if let Some(edges) = &self.edge_layers {
            ensure!(
                edges.masks.len() == poses.frames.len(),
                "Incorrect foreground mask count"
            );
            for file in edges.masks.iter().chain(std::iter::once(&edges.background)) {
                ensure!(
                    local_filename(&file.file) && valid_hash(&file.sha256),
                    "Invalid edge-layer filename or hash"
                );
            }
        }
        Ok(())
    }
}
pub struct ArmFlowAssets {
    pub manifest: FlowManifest,
    pub fields: Vec<image::ImageBuffer<image::Rgba<u16>, Vec<u16>>>,
    /// Maximum forward/backward displacement per pair, in full-crop pixels.
    /// Computed while loading, not by scanning textures on the render thread.
    pub max_displacements: Vec<f32>,
    pub edges: Option<EdgeLayerAssets>,
}
pub struct EdgeLayerAssets {
    pub masks: Vec<image::GrayImage>,
    pub background: image::RgbaImage,
}
impl EdgeLayerAssets {
    pub fn apply_masks(&self, poses: &mut [image::RgbaImage]) -> Result<()> {
        ensure!(
            self.masks.len() == poses.len()
                && poses
                    .iter()
                    .zip(&self.masks)
                    .all(|(pose, mask)| pose.dimensions() == mask.dimensions()
                        && mask.dimensions() == self.background.dimensions()),
            "Foreground masks do not match pose images"
        );
        for (pose, mask) in poses.iter_mut().zip(&self.masks) {
            for (pixel, coverage) in pose.pixels_mut().zip(mask.pixels()) {
                pixel.0[3] = coverage.0[0];
            }
        }
        Ok(())
    }
}
impl ArmFlowAssets {
    #[cfg(test)]
    pub(crate) fn load(path: &Path, pose_path: &Path, poses: &PoseManifest) -> Result<Self> {
        Self::load_budgeted(path, pose_path, poses, true, &mut Default::default())
    }
    pub(crate) fn load_budgeted(
        path: &Path,
        pose_path: &Path,
        poses: &PoseManifest,
        interpolate: bool,
        budget: &mut crate::asset_validation::SceneBudget,
    ) -> Result<Self> {
        use crate::asset_validation::{
            MAX_MANIFEST_BYTES, buffer_bytes, hash_file, image_bytes, read_bounded, read_manifest,
        };
        let manifest: FlowManifest = read_manifest(path).with_context(|| {
            format!(
                "Cannot load arm interpolation data: {}. Install a complete scene package.",
                path.display()
            )
        })?;
        manifest.validate(poses)?;
        if interpolate {
            let bytes = image_bytes((manifest.width, manifest.height), 8, "arm flow")?;
            budget.reserve(
                buffer_bytes(&[bytes, manifest.pairs.len() as u64], "arm flow array")?,
                "arm flow array",
            )?;
        }
        if let Some(edges) = &manifest.edge_layers {
            let pixels = image_bytes((manifest.crop_width, manifest.crop_height), 1, "arm masks")?;
            budget.reserve(
                buffer_bytes(
                    &[pixels, edges.masks.len() as u64 + 4],
                    "arm masks/background",
                )?,
                "arm masks/background",
            )?;
        }
        ensure!(
            sha256(&read_bounded(pose_path, MAX_MANIFEST_BYTES)?)
                .eq_ignore_ascii_case(&manifest.pose_manifest_sha256),
            "Arm interpolation data is stale; install a matched scene package"
        );
        let pose_folder = pose_path.parent().context("Pose manifest needs a folder")?;
        for (frame, expected) in poses.frames.iter().zip(&manifest.pose_images_sha256) {
            ensure!(
                hash_file(&pose_folder.join(&frame.file))?.eq_ignore_ascii_case(expected),
                "Flow cache no longer matches {}",
                frame.file
            );
        }
        let folder = path.parent().context("Flow manifest needs a folder")?;
        let mut fields = Vec::with_capacity(manifest.pairs.len());
        let mut max_displacements = Vec::with_capacity(manifest.pairs.len());
        for pair in &manifest.pairs {
            if !interpolate {
                break;
            }
            let encoded = read_hashed(folder, pair)?;
            let image = decode_image(&encoded, (manifest.width, manifest.height), &pair.file)?;
            ensure!(
                image.color() == image::ColorType::Rgba16,
                "Flow fields must be RGBA16 PNGs"
            );
            let image = image.into_rgba16();
            max_displacements.push(max_displacement(&image));
            fields.push(image);
        }
        let edges = if let Some(metadata) = &manifest.edge_layers {
            let read = |file: &FlowPair| -> Result<image::DynamicImage> {
                decode_image(
                    &read_hashed(folder, file)?,
                    (manifest.crop_width, manifest.crop_height),
                    &file.file,
                )
            };
            let mut masks = Vec::with_capacity(metadata.masks.len());
            for file in &metadata.masks {
                let image = read(file)?;
                ensure!(
                    image.color() == image::ColorType::L8,
                    "Foreground masks must be grayscale 8-bit PNGs"
                );
                masks.push(image.into_luma8());
            }
            let background = read(&metadata.background)?;
            ensure!(
                background.color() == image::ColorType::Rgba8,
                "Background reference must be RGBA8"
            );
            Some(EdgeLayerAssets {
                masks,
                background: background.into_rgba8(),
            })
        } else {
            None
        };
        tracing::info!(
            pairs = fields.len(),
            width = manifest.width,
            height = manifest.height,
            edge_layers = edges.is_some(),
            "Loaded arm foreground/flow package; motion fields are used only for interpolation"
        );
        Ok(Self {
            manifest,
            fields,
            max_displacements,
            edges,
        })
    }
}
/// Shared on-disk/shader encoding: displacement in full-crop pixels, not grid pixels.
pub fn decode_component(value: u16) -> f32 {
    (value as f32 - 32768.0) / 256.0
}

fn max_displacement(field: &image::ImageBuffer<image::Rgba<u16>, Vec<u16>>) -> f32 {
    // One square root per field. Include every vector, not a percentile or only
    // the tube: shadow/edge residual motion must also influence the cadence.
    field
        .pixels()
        .flat_map(|pixel| {
            [
                [decode_component(pixel[0]), decode_component(pixel[1])],
                [decode_component(pixel[2]), decode_component(pixel[3])],
            ]
        })
        .map(|[x, y]| x * x + y * y)
        .fold(0.0_f32, f32::max)
        .sqrt()
}

#[cfg(test)]
mod tests;
