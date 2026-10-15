//! Hash-bound, scene-linear printed-paper response and its exported view transform.
use crate::{
    artwork_geometry::ArtworkSettings,
    asset_validation::{AssetFile, local_filename, valid_hash},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Deserialize, Serialize)]
pub struct SleeveManifest {
    pub format_version: u32,
    #[serde(default)]
    pub response_kind: Option<String>,
    pub base_image_sha256: String,
    pub width: u32,
    pub height: u32,
    pub crop: [u32; 4],
    pub corners: [[f64; 2]; 4],
    pub illumination: AssetFile,
    pub residual: AssetFile,
    pub view_lut: AssetFile,
    pub lut_edge: u32,
    pub lut_log_min: f32,
    pub lut_log_max: f32,
}
impl SleeveManifest {
    pub fn validate(&self, dimensions: (u32, u32)) -> Result<[f32; 9]> {
        ensure!(
            self.format_version == 1 && valid_hash(&self.base_image_sha256),
            "Invalid sleeve package version/hash"
        );
        ensure!(
            (self.width, self.height) == dimensions,
            "Sleeve belongs to a different scene resolution"
        );
        let [x, y, w, h] = self.crop;
        ensure!(
            w > 0
                && h > 0
                && w <= 4096
                && h <= 4096
                && u64::from(w) * u64::from(h) <= 8_388_608
                && x.checked_add(w).is_some_and(|right| right <= self.width)
                && y.checked_add(h).is_some_and(|bottom| bottom <= self.height),
            "Invalid sleeve crop"
        );
        ensure!(
            (16..=128).contains(&self.lut_edge)
                && self.lut_log_min.is_finite()
                && self.lut_log_max.is_finite()
                && (-24.0..=0.0).contains(&self.lut_log_min)
                && (0.0..=24.0).contains(&self.lut_log_max)
                && self.lut_log_max > self.lut_log_min,
            "Invalid sleeve colour LUT domain"
        );
        for file in [&self.illumination, &self.residual, &self.view_lut] {
            ensure!(
                local_filename(&file.file) && valid_hash(&file.sha256),
                "Invalid sleeve filename/hash"
            );
        }
        let fit = ArtworkSettings {
            corners: self.corners,
            size_percent: 100.0,
            rotation_rpm: 0.0,
            ..Default::default()
        };
        fit.transform()
    }
}

pub struct SleeveAssets {
    pub manifest: SleeveManifest,
    pub transform: [f32; 9],
    /// Native-endian binary RGBA half floats for GL_HALF_FLOAT upload.
    pub illumination: Vec<u8>,
    /// Robust bright-paper RGB, derived once from this scene's calibration.
    pub light_reference: [f32; 3],
    pub residual: Vec<u8>,
    /// RGBA8, red-fastest 3D LUT; already display encoded by Blender.
    pub view_lut: Vec<u8>,
}
pub(crate) fn bounded_file(folder: &Path, file: &AssetFile, bytes: usize) -> Result<Vec<u8>> {
    ensure!(
        local_filename(&file.file),
        "Sleeve files must stay in their package"
    );
    let metadata = fs::metadata(folder.join(&file.file))?;
    ensure!(
        metadata.is_file() && metadata.len() == bytes as u64,
        "Incorrect sleeve buffer size: {}",
        file.file
    );
    let data = crate::asset_validation::read_hashed_bounded(folder, file, bytes as u64)?;
    ensure!(data.len() == bytes, "Sleeve buffer changed while reading");
    Ok(data)
}
fn validate_half(data: &mut [u8]) -> Result<()> {
    ensure!(data.len() % 8 == 0, "Incomplete sleeve RGBA16F pixel");
    for (i, sample) in data.chunks_exact_mut(2).enumerate() {
        let bits = u16::from_le_bytes([sample[0], sample[1]]);
        ensure!(
            bits & 0x7c00 != 0x7c00 && bits & 0x8000 == 0,
            "Nonfinite/negative sleeve lighting"
        );
        ensure!(i % 4 != 3 || bits <= 0x3c00, "Sleeve coverage exceeds one");
        sample.copy_from_slice(&bits.to_ne_bytes());
    }
    Ok(())
}
/// Decode validated nonnegative, finite native-endian half floats without adding
/// a runtime dependency. Subnormal values still matter for dim night packages.
fn positive_half(sample: &[u8]) -> f32 {
    let bits = u16::from_ne_bytes([sample[0], sample[1]]);
    let exponent = (bits >> 10) & 31;
    let mantissa = u32::from(bits & 1023);
    if exponent == 0 {
        mantissa as f32 * 2.0_f32.powi(-24)
    } else {
        f32::from_bits(((u32::from(exponent) + 112) << 23) | (mantissa << 13))
    }
}
fn light_reference(illumination: &[u8]) -> [f32; 3] {
    // Fully covered paper only: AA edges, plants, stand and background must not
    // influence the reference. P90 rejects isolated coat/bump highlights and
    // automatically follows authored day/night lighting (before AgX exposure).
    let mut light: Vec<(f32, [f32; 3])> = illumination
        .chunks_exact(8)
        .filter(|pixel| positive_half(&pixel[6..8]) > 0.999)
        .map(|pixel| {
            let alpha = positive_half(&pixel[6..8]);
            let rgb = [0, 2, 4].map(|i| positive_half(&pixel[i..i + 2]) / alpha);
            let luminance = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            (luminance, rgb)
        })
        .filter(|(luminance, _)| *luminance > 0.0)
        .collect();
    if light.is_empty() {
        return [0.0; 3];
    }
    let index = (light.len() - 1) * 9 / 10;
    let reference = light
        .select_nth_unstable_by(index, |a, b| a.0.total_cmp(&b.0))
        .1
        .0;
    // Average chromaticity near P90; a single noisy texel must not determine
    // the tint. Normalize back to P90 luminance so day/night exposure is retained.
    let mut chroma = [0.0_f64; 3];
    let mut count = 0;
    for (luminance, rgb) in &light {
        if (reference * 0.95..=reference * 1.05).contains(luminance) {
            for c in 0..3 {
                chroma[c] += f64::from(rgb[c] / luminance);
            }
            count += 1;
        }
    }
    chroma.map(|c| (c * (f64::from(reference) / f64::from(count))) as f32)
}
impl SleeveAssets {
    #[cfg(test)]
    pub(crate) fn load(path: &Path, beauty: &Path, dimensions: (u32, u32)) -> Result<Self> {
        Self::load_response(path, beauty, dimensions, "sleeve")
    }
    #[cfg(test)]
    pub(crate) fn load_record(path: &Path, beauty: &Path, dimensions: (u32, u32)) -> Result<Self> {
        Self::load_response(path, beauty, dimensions, "record_lighting")
    }
    #[cfg(test)]
    fn load_response(
        path: &Path,
        beauty: &Path,
        dimensions: (u32, u32),
        kind: &str,
    ) -> Result<Self> {
        Self::load_budgeted(path, beauty, dimensions, kind, &mut Default::default())
    }
    pub(crate) fn load_budgeted(
        path: &Path,
        beauty: &Path,
        dimensions: (u32, u32),
        kind: &str,
        budget: &mut crate::asset_validation::SceneBudget,
    ) -> Result<Self> {
        use crate::asset_validation::{buffer_bytes, hash_file, image_bytes, read_manifest};
        let manifest: SleeveManifest = read_manifest(path)?;
        ensure!(
            if kind != "sleeve" {
                manifest.response_kind.as_deref() == Some(kind)
            } else {
                manifest.response_kind.is_none()
                    || manifest.response_kind.as_deref() == Some("sleeve")
            },
            "Baked paper response is for a different artwork surface"
        );
        let transform = manifest.validate(dimensions)?;
        ensure!(
            hash_file(beauty)? == manifest.base_image_sha256,
            "Sleeve lighting does not match this beauty; regenerate it"
        );
        let folder = path.parent().context("Missing sleeve package folder")?;
        let bytes =
            image_bytes((manifest.crop[2], manifest.crop[3]), 8, "paper response")? as usize;
        let lut_bytes = buffer_bytes(
            &[
                u64::from(manifest.lut_edge),
                u64::from(manifest.lut_edge),
                u64::from(manifest.lut_edge),
                4,
            ],
            "paper colour LUT",
        )?;
        budget.reserve(bytes as u64 * 2 + lut_bytes, kind)?;
        let mut illumination = bounded_file(folder, &manifest.illumination, bytes)?;
        let mut residual = bounded_file(folder, &manifest.residual, bytes)?;
        validate_half(&mut illumination)?;
        validate_half(&mut residual)?;
        let light_reference = if kind == "sleeve" {
            light_reference(&illumination)
        } else {
            [0.0; 3] // Record/dust do not use artistic sleeve shadow lift.
        };
        let view_lut = bounded_file(folder, &manifest.view_lut, lut_bytes as usize)?;
        tracing::info!(crop=?manifest.crop, lut_edge=manifest.lut_edge,
            texture_mib=(bytes*2+view_lut.len()) as f64 / 1048576.0,
            "Loaded baked paper illumination/highlight response; no runtime ray tracing");
        Ok(Self {
            manifest,
            transform,
            illumination,
            light_reference,
            residual,
            view_lut,
        })
    }
}

#[cfg(test)]
mod tests;
