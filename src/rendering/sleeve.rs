//! Shared GPU ownership for baked sleeve/record paper responses on separate passes.
use crate::sleeve_assets::SleeveAssets;
use anyhow::{Result, ensure};
use glow::HasContext;
use std::rc::Rc;

pub(super) struct GpuPaperResponse {
    gl: Rc<glow::Context>,
    paper_textures: Vec<glow::NativeTexture>,
    view: Rc<super::view_lut::ViewLut>,
    enabled: Option<glow::NativeUniformLocation>,
    shadow_lift: Option<glow::NativeUniformLocation>,
    first_unit: u32,
}
impl Drop for GpuPaperResponse {
    fn drop(&mut self) {
        unsafe {
            for texture in &self.paper_textures {
                self.gl.delete_texture(*texture);
            }
        }
    }
}
impl GpuPaperResponse {
    pub(super) unsafe fn upload(
        luts: &mut super::view_lut::ViewLutPool,
        program: glow::NativeProgram,
        assets: SleeveAssets,
        prefix: &str,
        first_unit: u32,
    ) -> Result<Self> {
        unsafe {
            let gl = luts.context();
            let m = &assets.manifest;
            let limit = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
            ensure!(
                m.crop[2] <= limit
                    && m.crop[3] <= limit
                    && m.lut_edge <= gl.get_parameter_i32(glow::MAX_3D_TEXTURE_SIZE) as u32,
                "Sleeve exceeds this GPU's texture limits"
            );
            let mut sleeve = Self {
                view: luts.acquire(m.lut_edge, &assets.view_lut, first_unit + 2)?,
                enabled: gl.get_uniform_location(program, &format!("{prefix}Enabled")),
                shadow_lift: gl.get_uniform_location(program, &format!("{prefix}ShadowLift")),
                gl: gl.clone(),
                first_unit,
                paper_textures: vec![],
            };
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            for (unit, bytes) in [
                (first_unit, &assets.illumination),
                (first_unit + 1, &assets.residual),
            ] {
                let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
                sleeve.paper_textures.push(texture);
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA16F as i32,
                    m.crop[2] as i32,
                    m.crop[3] as i32,
                    0,
                    glow::RGBA,
                    glow::HALF_FLOAT,
                    glow::PixelUnpackData::Slice(Some(bytes)),
                );
                parameters(&gl, glow::TEXTURE_2D);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            Self::configure(&gl, program, &assets, prefix, first_unit);
            gl.active_texture(glow::TEXTURE0);
            Ok(sleeve)
        }
    }
    /// Set calibration uniforms for another pass without duplicating textures.
    pub(super) unsafe fn configure(
        gl: &glow::Context,
        program: glow::NativeProgram,
        assets: &SleeveAssets,
        prefix: &str,
        first_unit: u32,
    ) {
        unsafe {
            let m = &assets.manifest;
            gl.use_program(Some(program));
            gl.uniform_3_f32(
                gl.get_uniform_location(program, &format!("{prefix}LightReference"))
                    .as_ref(),
                assets.light_reference[0],
                assets.light_reference[1],
                assets.light_reference[2],
            );
            for (suffix, unit) in [
                ("Illumination", first_unit),
                ("Residual", first_unit + 1),
                ("View", first_unit + 2),
            ] {
                gl.uniform_1_i32(
                    gl.get_uniform_location(program, &format!("{prefix}{suffix}"))
                        .as_ref(),
                    unit as i32,
                );
            }
            gl.uniform_4_f32(
                gl.get_uniform_location(program, &format!("{prefix}Crop"))
                    .as_ref(),
                m.crop[0] as f32 / m.width as f32,
                m.crop[1] as f32 / m.height as f32,
                m.crop[2] as f32 / m.width as f32,
                m.crop[3] as f32 / m.height as f32,
            );
            gl.uniform_3_f32(
                gl.get_uniform_location(program, &format!("{prefix}Lut"))
                    .as_ref(),
                m.lut_log_min,
                1.0 / (m.lut_log_max - m.lut_log_min),
                m.lut_edge as f32,
            );
            gl.uniform_matrix_3_f32_slice(
                gl.get_uniform_location(program, &format!("{prefix}Transform"))
                    .as_ref(),
                false,
                &assets.transform,
            );
            gl.use_program(None);
        }
    }
    pub(super) unsafe fn bind(&self, enabled: bool, shadow_lift: f32) {
        unsafe {
            self.gl
                .uniform_1_i32(self.enabled.as_ref(), i32::from(enabled));
            self.gl
                .uniform_1_f32(self.shadow_lift.as_ref(), shadow_lift);
            self.bind_textures();
        }
    }
    pub(super) unsafe fn bind_textures(&self) {
        unsafe {
            for (i, texture) in self.paper_textures.iter().enumerate() {
                self.gl
                    .active_texture(glow::TEXTURE0 + self.first_unit + i as u32);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
            }
            self.gl.active_texture(glow::TEXTURE0 + self.first_unit + 2);
            self.gl
                .bind_texture(glow::TEXTURE_3D, Some(self.view.texture));
        }
    }
    pub(super) unsafe fn unbind(&self) {
        unsafe {
            for i in 0..self.paper_textures.len() {
                self.gl
                    .active_texture(glow::TEXTURE0 + self.first_unit + i as u32);
                self.gl.bind_texture(glow::TEXTURE_2D, None);
            }
            self.gl.active_texture(glow::TEXTURE0 + self.first_unit + 2);
            self.gl.bind_texture(glow::TEXTURE_3D, None);
        }
    }
    #[cfg(test)]
    pub(super) fn test_view(&self) -> glow::NativeTexture {
        self.view.texture
    }
}
unsafe fn parameters(gl: &glow::Context, target: u32) {
    unsafe {
        for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
            gl.tex_parameter_i32(target, parameter, glow::LINEAR as i32);
        }
        for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
            gl.tex_parameter_i32(target, parameter, glow::CLAMP_TO_EDGE as i32);
        }
    }
}
