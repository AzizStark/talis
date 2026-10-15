//! Renderer-local sharing of immutable, identically sampled colour lookup tables.
//! No global/context-crossing cache and no retained CPU image copies.
use anyhow::{Result, ensure};
use glow::HasContext;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    edge: u32,
    digest: [u8; 32],
}
impl Key {
    fn new(edge: u32, bytes: &[u8]) -> Result<Self> {
        let expected = (edge as usize)
            .checked_pow(3)
            .and_then(|n| n.checked_mul(4));
        ensure!(
            edge > 0 && expected == Some(bytes.len()),
            "Invalid RGBA8 colour lookup buffer"
        );
        Ok(Self {
            edge,
            digest: Sha256::digest(bytes).into(),
        })
    }
}

pub(super) struct ViewLut {
    gl: Rc<glow::Context>,
    pub texture: glow::NativeTexture,
}
impl Drop for ViewLut {
    fn drop(&mut self) {
        // Like the other GPU resource owners, the final release requires the
        // owning context current. Each response holds a strong reference.
        unsafe {
            self.gl.delete_texture(self.texture);
        }
    }
}

pub(super) struct ViewLutPool {
    gl: Rc<glow::Context>,
    entries: HashMap<Key, Weak<ViewLut>>,
    #[cfg(test)]
    pub share: bool,
}
impl ViewLutPool {
    pub fn new(gl: Rc<glow::Context>) -> Self {
        Self {
            gl,
            entries: HashMap::new(),
            #[cfg(test)]
            share: true,
        }
    }
    pub fn context(&self) -> Rc<glow::Context> {
        self.gl.clone()
    }

    /// The pool fixes format (RGBA8), linear filtering, clamp on all axes and
    /// level-zero sampling. Any future sampling variant must use a distinct key.
    /// Only call with this pool's owning GL context current, including teardown.
    pub unsafe fn acquire(&mut self, edge: u32, bytes: &[u8], unit: u32) -> Result<Rc<ViewLut>> {
        let key = Key::new(edge, bytes)?;
        let gl = &self.gl;
        unsafe {
            ensure!(
                edge <= gl.get_parameter_i32(glow::MAX_3D_TEXTURE_SIZE) as u32,
                "Colour lookup exceeds GPU texture limits"
            );
            self.entries.retain(|_, value| value.strong_count() > 0);
            #[cfg(test)]
            let sharing = self.share;
            #[cfg(not(test))]
            let sharing = true;
            if sharing {
                if let Some(texture) = self.entries.get(&key).and_then(Weak::upgrade) {
                    tracing::debug!(edge, "Reused renderer-local colour lookup texture");
                    return Ok(texture);
                }
            }
            let lut = Rc::new(ViewLut {
                gl: gl.clone(),
                texture: gl.create_texture().map_err(anyhow::Error::msg)?,
            });
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_3D, Some(lut.texture));
            gl.tex_image_3d(
                glow::TEXTURE_3D,
                0,
                glow::RGBA8 as i32,
                edge as i32,
                edge as i32,
                edge as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(bytes)),
            );
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_3D, parameter, glow::LINEAR as i32);
            }
            for parameter in [
                glow::TEXTURE_WRAP_S,
                glow::TEXTURE_WRAP_T,
                glow::TEXTURE_WRAP_R,
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_3D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_3D, None);
            gl.active_texture(glow::TEXTURE0);
            if sharing {
                self.entries.insert(key, Rc::downgrade(&lut));
            }
            tracing::debug!(
                edge,
                texture_mib = bytes.len() as f64 / 1048576.0,
                "Uploaded renderer-local colour lookup texture"
            );
            Ok(lut)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_identity_uses_content_and_dimensions_not_path_or_calibration() {
        let a = vec![12; 2 * 2 * 2 * 4];
        assert_eq!(Key::new(2, &a).unwrap(), Key::new(2, &a.clone()).unwrap());
        let mut b = a.clone();
        b[17] ^= 1;
        assert_ne!(Key::new(2, &a).unwrap(), Key::new(2, &b).unwrap());
        assert_ne!(
            Key::new(2, &a).unwrap(),
            Key::new(3, &[12; 3 * 3 * 3 * 4]).unwrap()
        );
    }
    #[test]
    fn invalid_lookup_buffers_are_rejected_before_gpu_allocation() {
        assert!(Key::new(0, &[]).is_err());
        assert!(Key::new(2, &[0; 31]).is_err());
        assert!(Key::new(2, &[0; 33]).is_err());
        assert!(Key::new(u32::MAX, &[0; 4]).is_err());
    }
}
