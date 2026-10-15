//! Two immutable tiny patches; only the marker angle changes at runtime.
use crate::knob_assets::KnobAssets;
use anyhow::{Result, ensure};
use glow::HasContext;
use std::rc::Rc;
pub(super) struct GpuKnob {
    gl: Rc<glow::Context>,
    textures: Vec<glow::NativeTexture>,
    uniforms: [Option<glow::NativeUniformLocation>; 8],
    transform: [f32; 9],
    size: [f32; 2],
    marker: [f32; 3],
    angle: f32,
    pub crop: [f32; 4],
    pub dirty: bool,
    pub updates: u64,
}
impl GpuKnob {
    pub unsafe fn new(
        gl: Rc<glow::Context>,
        program: glow::NativeProgram,
        a: KnobAssets,
    ) -> Result<Self> {
        let m = a.manifest;
        unsafe {
            ensure!(
                gl.get_parameter_i32(glow::MAX_TEXTURE_IMAGE_UNITS) >= 15,
                "Knob requires 15 texture units"
            );
            let mut value = Self {
                gl: gl.clone(),
                textures: vec![],
                uniforms: [
                    "uKnobEnabled",
                    "uKnobCrop",
                    "uKnobPixels",
                    "uKnobTransform",
                    "uKnobMarker",
                    "uKnobAngle",
                    "uKnobClean",
                    "uKnobInk",
                ]
                .map(|name| gl.get_uniform_location(program, name)),
                transform: a.transform,
                size: [m.crop[2] as f32, m.crop[3] as f32],
                marker: [m.marker_width, m.marker_inner, m.marker_outer],
                angle: 0.0,
                crop: [
                    m.crop[0] as f32 / m.width as f32,
                    m.crop[1] as f32 / m.height as f32,
                    m.crop[2] as f32 / m.width as f32,
                    m.crop[3] as f32 / m.height as f32,
                ],
                dirty: true,
                updates: 0,
            };
            for (index, image) in [a.clean, a.ink].iter().enumerate() {
                let texture = value.gl.create_texture().map_err(anyhow::Error::msg)?;
                value.textures.push(texture);
                value.gl.active_texture(glow::TEXTURE0 + 13 + index as u32);
                value.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                value.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                value.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    m.crop[2] as i32,
                    m.crop[3] as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(image.as_raw())),
                );
                for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    value
                        .gl
                        .tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
                }
                for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    value.gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        parameter,
                        glow::CLAMP_TO_EDGE as i32,
                    );
                }
                value.gl.bind_texture(glow::TEXTURE_2D, None);
            }
            value.gl.active_texture(glow::TEXTURE0);
            Ok(value)
        }
    }
    pub fn set_angle(&mut self, angle: f32) -> bool {
        if !angle.is_finite() {
            return false;
        }
        let angle = angle.clamp(0.0, std::f32::consts::TAU);
        if (angle - self.angle).abs() < 1e-6 {
            return false;
        }
        self.angle = angle;
        self.dirty = true;
        true
    }
    pub unsafe fn bind(&self) {
        unsafe {
            let gl = &self.gl;
            let u = &self.uniforms;
            gl.uniform_1_i32(u[0].as_ref(), 1);
            gl.uniform_4_f32_slice(u[1].as_ref(), &self.crop);
            gl.uniform_2_f32_slice(u[2].as_ref(), &self.size);
            gl.uniform_matrix_3_f32_slice(u[3].as_ref(), false, &self.transform);
            gl.uniform_3_f32_slice(u[4].as_ref(), &self.marker);
            // Exact same shader angle at 0% and 100%; retain unwrapped motion
            // above so the marker still travels through a complete revolution.
            gl.uniform_1_f32(u[5].as_ref(), self.angle.rem_euclid(std::f32::consts::TAU));
            for i in 0..2 {
                gl.uniform_1_i32(u[6 + i].as_ref(), 13 + i as i32);
                gl.active_texture(glow::TEXTURE0 + 13 + i as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[i]));
            }
        }
    }
    pub unsafe fn unbind(&self) {
        unsafe {
            for i in 0..2 {
                self.gl.active_texture(glow::TEXTURE0 + 13 + i);
                self.gl.bind_texture(glow::TEXTURE_2D, None);
            }
        }
    }
}
impl Drop for GpuKnob {
    fn drop(&mut self) {
        unsafe {
            for texture in &self.textures {
                self.gl.delete_texture(*texture);
            }
        }
    }
}
