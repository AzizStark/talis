//! Immutable baked pose texture; only a layer index changes during a click.
use crate::button_assets::{ButtonAssets, layers};
use anyhow::{Result, ensure};
use glow::HasContext;
use std::rc::Rc;

pub(super) struct GpuButtons {
    gl: Rc<glow::Context>,
    texture: glow::NativeTexture,
    enabled: Option<glow::NativeUniformLocation>,
    crop_uniform: Option<glow::NativeUniformLocation>,
    pixels_uniform: Option<glow::NativeUniformLocation>,
    layers_uniform: Option<glow::NativeUniformLocation>,
    pub crop: [f32; 4],
    size: [f32; 2],
    steps: u32,
    layers: [i32; 3],
    pub dirty: bool,
}
impl GpuButtons {
    pub unsafe fn new(
        gl: Rc<glow::Context>,
        program: glow::NativeProgram,
        assets: ButtonAssets,
    ) -> Result<Self> {
        let m = assets.manifest;
        unsafe {
            ensure!(
                gl.get_parameter_i32(glow::MAX_ARRAY_TEXTURE_LAYERS) >= (1 + 3 * m.steps) as i32
                    && gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE)
                        >= m.crop[2].max(m.crop[3]) as i32,
                "Button atlas exceeds GPU texture limits"
            );
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            let value = Self {
                enabled: gl.get_uniform_location(program, "uButtonsEnabled"),
                crop_uniform: gl.get_uniform_location(program, "uButtonsCrop"),
                pixels_uniform: gl.get_uniform_location(program, "uButtonsPixels"),
                layers_uniform: gl.get_uniform_location(program, "uButtonsLayers"),
                crop: [
                    m.crop[0] as f32 / m.width as f32,
                    m.crop[1] as f32 / m.height as f32,
                    m.crop[2] as f32 / m.width as f32,
                    m.crop[3] as f32 / m.height as f32,
                ],
                size: [m.crop[2] as f32, m.crop[3] as f32],
                steps: m.steps,
                layers: [0; 3],
                dirty: true,
                gl,
                texture,
            };
            value.gl.active_texture(glow::TEXTURE12);
            value.gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            value.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            value.gl.tex_image_3d(
                glow::TEXTURE_2D_ARRAY,
                0,
                glow::RGBA8 as i32,
                m.crop[2] as i32,
                m.crop[3] as i32,
                (1 + 3 * m.steps) as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&assets.pixels)),
            );
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                value
                    .gl
                    .tex_parameter_i32(glow::TEXTURE_2D_ARRAY, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                value.gl.tex_parameter_i32(
                    glow::TEXTURE_2D_ARRAY,
                    parameter,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            value.gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            value.gl.active_texture(glow::TEXTURE0);
            tracing::info!(
                texture_mib = assets.pixels.len() as f64 / 1048576.0,
                steps = m.steps,
                "Baked button depths uploaded once; click animation updates only the button patch"
            );
            Ok(value)
        }
    }
    pub fn set_progress(&mut self, progress: [f32; 3]) -> bool {
        let layers = layers(progress, self.steps);
        if layers != self.layers {
            self.layers = layers;
            self.dirty = true;
            return true;
        }
        false
    }
    pub unsafe fn bind(&self) {
        unsafe {
            self.gl.uniform_1_i32(self.enabled.as_ref(), 1);
            self.gl.uniform_4_f32(
                self.crop_uniform.as_ref(),
                self.crop[0],
                self.crop[1],
                self.crop[2],
                self.crop[3],
            );
            self.gl
                .uniform_2_f32(self.pixels_uniform.as_ref(), self.size[0], self.size[1]);
            self.gl.uniform_3_i32(
                self.layers_uniform.as_ref(),
                self.layers[0],
                self.layers[1],
                self.layers[2],
            );
            self.gl.active_texture(glow::TEXTURE12);
            self.gl
                .bind_texture(glow::TEXTURE_2D_ARRAY, Some(self.texture));
        }
    }
}
impl Drop for GpuButtons {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_texture(self.texture);
        }
    }
}
