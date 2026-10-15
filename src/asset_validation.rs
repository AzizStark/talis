//! Shared checks for hash-bound files inside a scene asset package.
use anyhow::{Context, Result, ensure};
use image::ImageDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Component, Path},
};

pub(crate) const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SCENE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 33_554_432;

/// Distinct from corrupt optional assets: an unsupported package must not be
/// silently accepted with its largest layers missing.
#[derive(Debug)]
pub(crate) struct ResourceLimit(String);
impl std::fmt::Display for ResourceLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scene memory limit: {}", self.0)
    }
}
impl std::error::Error for ResourceLimit {}

fn limit(condition: bool, message: impl FnOnce() -> String) -> Result<()> {
    if !condition {
        return Err(ResourceLimit(message()).into());
    }
    Ok(())
}

/// Retained CPU pixel/binary payload, not RSS or a hardware VRAM guarantee.
/// Each loader reserves its whole group before reading/decoding its buffers.
#[derive(Default)]
pub(crate) struct SceneBudget {
    bytes: u64,
}
impl SceneBudget {
    pub(crate) fn reserve(&mut self, bytes: u64, label: &str) -> Result<()> {
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| ResourceLimit(format!("byte-count overflow for {label}")))?;
        limit(total <= MAX_SCENE_BYTES, || {
            format!("{label} would require {total} retained bytes (maximum {MAX_SCENE_BYTES})")
        })?;
        self.bytes = total;
        Ok(())
    }

    pub(crate) fn optional<T>(
        &mut self,
        load: impl FnOnce(&mut Self) -> Result<T>,
        label: &str,
    ) -> Result<Option<T>> {
        let before = self.bytes;
        match load(self) {
            Ok(asset) => Ok(Some(asset)),
            Err(error) => {
                self.bytes = before;
                if error.downcast_ref::<ResourceLimit>().is_some() {
                    return Err(error);
                }
                tracing::warn!(
                    "Optional {label} unavailable; authored fallback retained: {error:#}"
                );
                Ok(None)
            }
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub(crate) fn buffer_bytes(factors: &[u64], label: &str) -> Result<u64> {
    let bytes = factors
        .iter()
        .try_fold(1_u64, |n, v| n.checked_mul(*v))
        .ok_or_else(|| ResourceLimit(format!("byte-count overflow for {label}")))?;
    limit(usize::try_from(bytes).is_ok(), || {
        format!("{label} exceeds addressable memory")
    })?;
    Ok(bytes)
}

pub(crate) fn image_bytes(dimensions: (u32, u32), channels: u64, label: &str) -> Result<u64> {
    let (w, h) = dimensions;
    limit(
        w > 0
            && h > 0
            && w <= MAX_IMAGE_EDGE
            && h <= MAX_IMAGE_EDGE
            && u64::from(w) * u64::from(h) <= MAX_IMAGE_PIXELS,
        || {
            format!(
                "unsupported {label} dimensions {w}×{h}; maximum edge {MAX_IMAGE_EDGE}, pixels {MAX_IMAGE_PIXELS}"
            )
        },
    )?;
    buffer_bytes(&[u64::from(w), u64::from(h), channels], label)
}

/// Metadata catches oversized sparse files; the capped read also handles growth
/// after inspection. Opening once avoids a pathname/metadata replacement race.
pub(crate) fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Asset is not a regular file: {}",
        path.display()
    );
    limit(metadata.len() <= maximum, || {
        format!("{} exceeds {maximum} encoded bytes", path.display())
    })?;
    read_capped(file, maximum, &path.display().to_string(), metadata.len())
}

fn read_capped(reader: impl Read, maximum: u64, label: &str, initial_size: u64) -> Result<Vec<u8>> {
    let cap = maximum.checked_add(1).context("Invalid read limit")?;
    let mut bytes = Vec::new();
    let initial_size = usize::try_from(initial_size.min(maximum))
        .map_err(|_| ResourceLimit(format!("{label} exceeds addressable memory")))?;
    bytes
        .try_reserve_exact(initial_size)
        .map_err(|_| ResourceLimit(format!("cannot reserve encoded buffer for {label}")))?;
    let mut reader = reader.take(cap);
    let mut scratch = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut scratch)?;
        if read == 0 {
            break;
        }
        limit(bytes.len() as u64 + read as u64 <= maximum, || {
            format!("{label} grew beyond {maximum} encoded bytes")
        })?;
        // Fallible reservation also avoids read_to_end doubling capacity merely
        // to detect the single byte past the cap in a growing file.
        bytes
            .try_reserve_exact(read)
            .map_err(|_| ResourceLimit(format!("cannot extend encoded buffer for {label}")))?;
        bytes.extend_from_slice(&scratch[..read]);
    }
    Ok(bytes)
}

fn image_error(error: image::ImageError, label: &str) -> anyhow::Error {
    if matches!(error, image::ImageError::Limits(_)) {
        ResourceLimit(format!("decoder limits exceeded for {label}: {error}")).into()
    } else {
        error.into()
    }
}

pub(crate) fn read_manifest<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read_bounded(path, MAX_MANIFEST_BYTES)?)
        .with_context(|| format!("Invalid asset manifest {}", path.display()))
}

pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Asset is not a regular file: {}",
        path.display()
    );
    limit(metadata.len() <= MAX_ASSET_BYTES, || {
        format!("{} exceeds {MAX_ASSET_BYTES} encoded bytes", path.display())
    })?;
    let mut reader = file.take(MAX_ASSET_BYTES + 1);
    let mut hash = Sha256::new();
    let mut scratch = [0; 64 * 1024];
    let mut count = 0_u64;
    loop {
        let read = reader.read(&mut scratch)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        limit(count <= MAX_ASSET_BYTES, || {
            format!("{} grew beyond its encoded limit", path.display())
        })?;
        hash.update(&scratch[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn decoder<'a>(bytes: &'a [u8], label: &str) -> Result<Box<dyn image::ImageDecoder + 'a>> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| image_error(error, label))
        .with_context(|| format!("Cannot inspect {label}"))?;
    image_bytes(decoder.dimensions(), 4, label)?;
    limit(decoder.total_bytes() <= MAX_DECODE_BYTES, || {
        format!("{label} decoded buffer exceeds {MAX_DECODE_BYTES} bytes")
    })?;
    // Match ImageReader::decode: reserve the output separately from decoder
    // scratch. max_alloc is best-effort; explicit total_bytes is our hard check.
    let mut scratch_limits = image::Limits::default();
    scratch_limits.max_image_width = Some(MAX_IMAGE_EDGE);
    scratch_limits.max_image_height = Some(MAX_IMAGE_EDGE);
    scratch_limits.max_alloc = Some(MAX_DECODE_BYTES - decoder.total_bytes());
    decoder
        .set_limits(scratch_limits)
        .map_err(|error| image_error(error, label))?;
    Ok(Box::new(decoder))
}

pub(crate) fn load_rgba(path: &Path, budget: &mut SceneBudget) -> Result<image::RgbaImage> {
    let bytes = read_bounded(path, MAX_ASSET_BYTES)?;
    let label = path.display().to_string();
    let decoder = decoder(&bytes, &label)?;
    budget.reserve(image_bytes(decoder.dimensions(), 4, &label)?, &label)?;
    Ok(image::DynamicImage::from_decoder(decoder)
        .map_err(|error| image_error(error, &label))?
        .into_rgba8())
}

#[derive(Deserialize, Serialize)]
pub struct AssetFile {
    pub file: String,
    pub sha256: String,
}

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn local_filename(file: &str) -> bool {
    let mut components = Path::new(file).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn read_hashed(folder: &Path, asset: &AssetFile) -> Result<Vec<u8>> {
    read_hashed_bounded(folder, asset, MAX_ASSET_BYTES)
}

pub(crate) fn read_hashed_bounded(
    folder: &Path,
    asset: &AssetFile,
    maximum: u64,
) -> Result<Vec<u8>> {
    ensure!(
        local_filename(&asset.file) && valid_hash(&asset.sha256),
        "Invalid asset filename or hash"
    );
    let path = folder.join(&asset.file);
    let bytes = read_bounded(&path, maximum.min(MAX_ASSET_BYTES))?;
    ensure!(
        sha256(&bytes).eq_ignore_ascii_case(&asset.sha256),
        "Corrupt asset: {}",
        asset.file
    );
    Ok(bytes)
}

pub(crate) fn decode_image(
    bytes: &[u8],
    dimensions: (u32, u32),
    label: &str,
) -> Result<image::DynamicImage> {
    image_bytes(dimensions, 4, label)?;
    let decoder = decoder(bytes, label)?;
    ensure!(
        decoder.dimensions() == dimensions,
        "Incorrect image dimensions: {label}"
    );
    image::DynamicImage::from_decoder(decoder)
        .map_err(|error| image_error(error, label))
        .with_context(|| format!("Cannot decode {label}"))
}

#[cfg(test)]
mod tests;
