//! Persistent scene pixels: only dirty screen regions are recomposited.
use super::utils::compile;
use anyhow::{Result, ensure};
use glow::HasContext;
use std::{num::NonZeroU32, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PoseKey {
    Static,
    Stepped(usize),
    Flow(usize, usize, f32),
}
impl PoseKey {
    pub(super) fn new(progress: f32, count: usize, flow: bool) -> Self {
        if count == 0 {
            Self::Static
        } else if flow {
            let (a, b, t) = crate::arm_assets::interpolation_samples(progress, count);
            Self::Flow(a, b, t)
        } else {
            Self::Stepped(crate::arm_assets::nearest_pose_index(progress, count))
        }
    }
}

/// Scene-normalized top-left rectangle -> conservative GL scissor.
pub(super) fn region(crop: [f32; 4], viewport: (i32, i32, i32, i32)) -> [i32; 4] {
    let (x, y, w, h) = viewport;
    // Expand for filtering/AA at projected patch boundaries. All pixels are
    // recomputed from source textures, never blended into yesterday's result.
    let left = ((crop[0] * w as f32).floor() as i32 - 2).clamp(0, w);
    let right = (((crop[0] + crop[2]) * w as f32).ceil() as i32 + 2).clamp(0, w);
    let bottom = (((1.0 - crop[1] - crop[3]) * h as f32).floor() as i32 - 2).clamp(0, h);
    let top = (((1.0 - crop[1]) * h as f32).ceil() as i32 + 2).clamp(0, h);
    [x + left, y + bottom, right - left, top - bottom]
}
pub(super) unsafe fn framebuffer(
    gl: &glow::Context,
    parameter: u32,
) -> Option<glow::NativeFramebuffer> {
    unsafe { NonZeroU32::new(gl.get_parameter_i32(parameter) as u32).map(glow::NativeFramebuffer) }
}

pub(super) struct SceneCache {
    gl: Rc<glow::Context>,
    framebuffer: Option<glow::NativeFramebuffer>,
    textures: Vec<glow::NativeTexture>,
    pub program: Option<glow::NativeProgram>,
    pub dimensions: (i32, i32),
    pub pose: Option<PoseKey>,
    pub sleeve_dirty: bool,
    pub full_updates: u64,
    pub arm_updates: u64,
    pub sleeve_updates: u64,
    pub record_draws: u64,
    pub button_updates: u64,
}
impl SceneCache {
    #[cfg(test)]
    pub(super) unsafe fn test_foreground(&self) -> Vec<u8> {
        unsafe {
            let old = framebuffer(&self.gl, glow::READ_FRAMEBUFFER_BINDING);
            let read = self.gl.get_parameter_i32(glow::READ_BUFFER) as u32;
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, self.framebuffer);
            self.gl.read_buffer(glow::COLOR_ATTACHMENT1);
            let (w, h) = self.dimensions;
            let alignment = self.gl.get_parameter_i32(glow::PACK_ALIGNMENT);
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            let mut pixels = vec![0; (w * h) as usize];
            self.gl.read_pixels(
                0,
                0,
                w,
                h,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, alignment);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, old);
            self.gl.read_buffer(read);
            pixels
        }
    }
    pub(super) unsafe fn new(gl: Rc<glow::Context>, fragment: &str) -> Result<Self> {
        let mut cache = Self {
            gl,
            framebuffer: None,
            textures: vec![],
            program: None,
            dimensions: (0, 0),
            pose: None,
            sleeve_dirty: true,
            full_updates: 0,
            arm_updates: 0,
            sleeve_updates: 0,
            record_draws: 0,
            button_updates: 0,
        };
        unsafe {
            cache.program = Some(compile(
                &cache.gl,
                include_str!("../../shaders/beauty.vert"),
                fragment,
            )?);
            cache.framebuffer = Some(cache.gl.create_framebuffer().map_err(anyhow::Error::msg)?);
            for _ in 0..2 {
                cache
                    .textures
                    .push(cache.gl.create_texture().map_err(anyhow::Error::msg)?);
            }
        }
        Ok(cache)
    }
    /// Caller restores GTK's draw framebuffer even if allocation fails.
    pub(super) unsafe fn resize(&mut self, width: i32, height: i32) -> Result<bool> {
        if self.dimensions == (width, height) {
            return Ok(false);
        }
        unsafe {
            let gl = &self.gl;
            ensure!(
                width > 0
                    && height > 0
                    && width <= gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE)
                    && height <= gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE),
                "Scene cache exceeds GPU texture limits"
            );
            gl.active_texture(glow::TEXTURE0);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.framebuffer);
            for (index, texture) in self.textures.iter().enumerate() {
                gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    if index == 0 { glow::RGBA8 } else { glow::R8 } as i32,
                    width,
                    height,
                    0,
                    if index == 0 { glow::RGBA } else { glow::RED },
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::NEAREST as i32);
                }
                for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
                }
                gl.framebuffer_texture_2d(
                    glow::DRAW_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0 + index as u32,
                    glow::TEXTURE_2D,
                    Some(*texture),
                    0,
                );
            }
            gl.draw_buffers(&[glow::COLOR_ATTACHMENT0, glow::COLOR_ATTACHMENT1]);
            gl.bind_texture(glow::TEXTURE_2D, None);
            ensure!(
                gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE,
                "Cannot allocate the retained scene framebuffer"
            );
            self.dimensions = (width, height);
            self.pose = None;
            self.sleeve_dirty = true;
            tracing::info!(
                width,
                height,
                texture_mib = width as f64 * height as f64 * 5.0 / 1048576.0,
                "Retained scene cache allocated; only changed patches are recomposited"
            );
        }
        Ok(true)
    }
    pub(super) unsafe fn bind(&self) {
        unsafe {
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.framebuffer);
        }
    }
    pub(super) unsafe fn present(&self) {
        unsafe { self.present_region([0, 0, self.dimensions.0, self.dimensions.1]) }
    }
    /// Copy an opaque region to a same-size GTK target. Source is always global.
    pub(super) unsafe fn present_region(&self, region: [i32; 4]) {
        unsafe {
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, self.framebuffer);
            self.gl.read_buffer(glow::COLOR_ATTACHMENT0);
            let [x, y, w, h] = region;
            // GTK owns its render target and can replace/discard it between
            // callbacks. Present the retained opaque RGB, not stale GTK pixels.
            self.gl.blit_framebuffer(
                x,
                y,
                x + w,
                y + h,
                0,
                0,
                w,
                h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
        }
    }
    pub(super) unsafe fn bind_record(&self) {
        unsafe {
            self.gl.active_texture(glow::TEXTURE0);
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.textures[0]));
            self.gl.active_texture(glow::TEXTURE1);
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.textures[1]));
        }
    }
}
impl Drop for SceneCache {
    fn drop(&mut self) {
        tracing::info!(
            full_updates = self.full_updates,
            arm_updates = self.arm_updates,
            sleeve_updates = self.sleeve_updates,
            record_draws = self.record_draws,
            button_updates = self.button_updates,
            "Retained scene cache counters"
        );
        unsafe {
            for texture in &self.textures {
                self.gl.delete_texture(*texture);
            }
            if let Some(framebuffer) = self.framebuffer {
                self.gl.delete_framebuffer(framebuffer);
            }
            if let Some(program) = self.program {
                self.gl.delete_program(program);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dirty_regions_include_filter_margin_and_respect_letterboxing() {
        assert_eq!(
            region([0.25, 0.25, 0.5, 0.5], (100, 50, 800, 400)),
            [298, 148, 404, 204]
        );
        assert_eq!(
            region([0.0, 0.0, 1.0, 1.0], (100, 50, 800, 400)),
            [100, 50, 800, 400]
        );
    }
    #[test]
    fn pose_keys_skip_unchanged_steps_and_preserve_continuous_flow() {
        assert_eq!(
            PoseKey::new(0.515, 33, false),
            PoseKey::new(0.50, 33, false)
        );
        assert_ne!(PoseKey::new(0.515, 33, true), PoseKey::new(0.50, 33, true));
        assert_eq!(
            PoseKey::new(f32::NAN, 33, true),
            PoseKey::new(0.0, 33, true)
        );
        assert_eq!(PoseKey::new(0.9, 0, true), PoseKey::Static);
    }
}
