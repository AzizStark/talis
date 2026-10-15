//! Static previous-track printed face. Separate program reuses paper texture units;
//! no continuous draw or sampler pressure on the arm/record programs.
use super::{sleeve::GpuPaperResponse, utils::compile};
use anyhow::{Result, ensure};
use glow::HasContext;
use std::borrow::Cow;
use std::rc::Rc;

pub(super) struct GpuStack {
    gl: Rc<glow::Context>,
    program: Option<glow::NativeProgram>,
    vao: Option<glow::NativeVertexArray>,
    artwork: Option<glow::NativeTexture>,
    response: Option<GpuPaperResponse>,
    revision: Option<u64>,
    pub crop: [f32; 4],
    pub dirty: bool,
    pub updates: u64,
}
impl Drop for GpuStack {
    fn drop(&mut self) {
        unsafe {
            if let Some(t) = self.artwork {
                self.gl.delete_texture(t);
            }
            if let Some(v) = self.vao {
                self.gl.delete_vertex_array(v);
            }
            if let Some(p) = self.program {
                self.gl.delete_program(p);
            }
        }
    }
}
fn fallback() -> Result<image::RgbaImage> {
    // Exported from the editable SVG; photo/logo composition is not runtime work.
    // Keep the same 512px footprint as the old neutral sleeve.
    Ok(image::load_from_memory(include_bytes!(
        "../../assets/artwork/fallbacks/stack-sleeve.png"
    ))?
    .to_rgba8())
}
fn opaque_paper(image: &image::RgbaImage) -> Cow<'_, image::RgbaImage> {
    if image.pixels().all(|p| p[3] == 255) {
        return Cow::Borrowed(image);
    }
    // Transparent album pixels represent unprinted paper, not the original
    // baked Thriller artwork underneath. This runs only when the image changes.
    let mut paper = image::RgbaImage::from_pixel(
        image.width(),
        image.height(),
        image::Rgba([220, 214, 202, 255]),
    );
    image::imageops::overlay(&mut paper, image, 0, 0);
    Cow::Owned(paper)
}
impl GpuStack {
    #[cfg(test)]
    pub(super) fn test_view(&self) -> glow::NativeTexture {
        self.response.as_ref().unwrap().test_view()
    }
    pub(super) unsafe fn upload(
        luts: &mut super::view_lut::ViewLutPool,
        assets: crate::sleeve_assets::SleeveAssets,
    ) -> Result<Self> {
        let gl = luts.context();
        let m = &assets.manifest;
        let crop = [
            m.crop[0] as f32 / m.width as f32,
            m.crop[1] as f32 / m.height as f32,
            m.crop[2] as f32 / m.width as f32,
            m.crop[3] as f32 / m.height as f32,
        ];
        let mut stack = Self {
            gl: gl.clone(),
            program: None,
            vao: None,
            artwork: None,
            response: None,
            revision: None,
            crop,
            dirty: true,
            updates: 0,
        };
        let shader = include_str!("../../shaders/stack.frag").replace(
            "// PAPER_RESPONSE",
            include_str!("../../shaders/sleeve.glsl"),
        );
        unsafe {
            let p = compile(&gl, include_str!("../../shaders/beauty.vert"), &shader)?;
            stack.program = Some(p);
            stack.vao = Some(gl.create_vertex_array().map_err(anyhow::Error::msg)?);
            stack.response = Some(GpuPaperResponse::upload(luts, p, assets, "uSleeve", 5)?);
            gl.use_program(Some(p));
            gl.uniform_1_i32(gl.get_uniform_location(p, "uBeauty").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(p, "uArtwork").as_ref(), 4);
            gl.use_program(None);
            stack.set_artwork(0, None)?;
        }
        Ok(stack)
    }
    pub(super) unsafe fn set_artwork(
        &mut self,
        revision: u64,
        image: Option<&image::RgbaImage>,
    ) -> Result<()> {
        if self.revision == Some(revision) {
            return Ok(());
        }
        let neutral;
        let image = match image {
            Some(image) => image,
            None => {
                neutral = fallback()?;
                &neutral
            }
        };
        let image = opaque_paper(image);
        let gl = &self.gl;
        unsafe {
            let limit = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
            ensure!(
                image.width() > 0
                    && image.height() > 0
                    && image.width() <= limit
                    && image.height() <= limit,
                "Previous-track artwork exceeds GPU limits"
            );
            if self.artwork.is_none() {
                self.artwork = Some(gl.create_texture().map_err(anyhow::Error::msg)?);
            }
            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, self.artwork);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                image.width() as i32,
                image.height() as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(image.as_raw())),
            );
            gl.generate_mipmap(glow::TEXTURE_2D);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            if gl
                .supported_extensions()
                .contains("GL_EXT_texture_filter_anisotropic")
                || gl
                    .supported_extensions()
                    .contains("GL_ARB_texture_filter_anisotropic")
            {
                gl.tex_parameter_f32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAX_ANISOTROPY_EXT,
                    gl.get_parameter_f32(glow::MAX_TEXTURE_MAX_ANISOTROPY_EXT)
                        .clamp(1.0, 8.0),
                );
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
        }
        self.revision = Some(revision);
        self.dirty = true;
        Ok(())
    }
    /// Caller owns the retained framebuffer, viewport and stack-only scissor.
    pub(super) unsafe fn draw(&mut self, base: Option<glow::NativeTexture>) {
        unsafe {
            let gl = &self.gl;
            gl.use_program(self.program);
            self.response.as_ref().unwrap().bind(true, 0.0);
            gl.active_texture(glow::TEXTURE4);
            gl.bind_sampler(4, None);
            gl.bind_texture(glow::TEXTURE_2D, self.artwork);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, base);
            gl.bind_vertex_array(self.vao);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
            self.response.as_ref().unwrap().unbind();
            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
        self.dirty = false;
        self.updates += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_sleeve_is_opaque_bounded_and_preserves_photo_and_logo() {
        let image = fallback().unwrap();
        assert_eq!(image.dimensions(), (512, 512));
        assert!(image.pixels().all(|p| p[3] == 255));
        assert!(
            image.get_pixel(10, 10).0[..3].iter().all(|c| *c < 100),
            "photo missing from upper dark region"
        );
        assert!(
            image
                .enumerate_pixels()
                .any(|(_, y, p)| y > 200 && p[0] > 100 && p[0] > p[2].saturating_add(20)),
            "warm photographic detail missing"
        );
        assert!(
            image.pixels().any(|p| p.0 == [232, 221, 200, 255]),
            "ivory logo missing"
        );
        let source = include_str!("../../assets/artwork/fallbacks/stack-sleeve.svg");
        assert!(!source.contains("<text"));
        assert!(!source.contains("stroke="));
        assert!(!source.contains("<circle"));
        assert!(source.contains("talis-logo.png"));
        assert!(source.contains("david-dintsh-XdRf3vsLh5Y-unsplash.jpg"));
        assert!(source.contains("x=\"342.39\" y=\"30\" width=\"137.61\" height=\"99\""));
        assert!(source.contains("fill=\"#F1EBE0\" opacity=\"0.10\""));
        let ivory = |p: &image::Rgba<u8>| p.0 == [232, 221, 200, 255];
        assert!(
            image
                .enumerate_pixels()
                .any(|(x, y, p)| x > 342 && x < 480 && y < 129 && ivory(p)),
            "top-right logo missing"
        );
        assert!(
            !image
                .enumerate_pixels()
                .any(|(x, y, p)| x < 300 && y < 140 && ivory(p)),
            "old left/centered logo remains"
        );
    }
    #[test]
    fn transparent_print_is_paper_and_opaque_art_is_not_copied() {
        let mut art = image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]));
        assert!(matches!(opaque_paper(&art), Cow::Borrowed(_)));
        art.put_pixel(0, 0, image::Rgba([255, 0, 0, 0]));
        let flattened = opaque_paper(&art);
        assert_eq!(flattened.get_pixel(0, 0).0, [220, 214, 202, 255]);
        assert_eq!(flattened.get_pixel(1, 0), art.get_pixel(1, 0));
    }
}
