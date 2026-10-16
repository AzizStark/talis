//! GTK OpenGL lifecycle for cached photorealistic scene playback.
mod arm_cadence;
mod beauty;
mod buttons;
mod cache;
mod gpu_profile;
mod knob;
mod regional;
mod sleeve;
mod stack;
#[cfg(test)]
mod tests;
mod utils;
mod view_lut;
use crate::preview::PreviewAsset;
use anyhow::{Context, Result};
use glow::HasContext;
pub use gpu_profile::{FrameInfo as GpuFrameInfo, GpuFrameTiming, GpuStage};
use libloading::Library;
use std::{ffi::c_void, rc::Rc};
#[cfg(test)]
pub(crate) use utils::fitted_viewport;

pub struct Renderer {
    gl: Rc<glow::Context>,
    _epoxy: Library,
    source: beauty::GpuBeauty,
    pub gpu_ms: Option<f64>,
    info: RendererInfo,
}

/// Captured once with the owning context current; reading it never invokes GL.
#[derive(Clone, Debug)]
pub(crate) struct RendererInfo {
    pub version: String,
    pub shading_language: String,
    pub vendor: String,
    pub renderer: String,
    pub scene_size: (u32, u32),
}

impl RendererInfo {
    pub(crate) fn software_rendering(&self) -> bool {
        crate::graphics::software_renderer(&self.renderer)
    }
}

impl Renderer {
    pub(crate) fn info(&self) -> &RendererInfo {
        &self.info
    }
    /// Output-pixel bounds, bottom-left origin. None selects the full-frame fallback.
    pub fn regional_rect(&self, width: i32, height: i32, scale: i32) -> Option<[i32; 4]> {
        self.source.regional_rect(width, height, scale)
    }
    /// # Safety
    /// Owning context and a GTK target of exactly region[2] × region[3] are current.
    /// Returns whether the separate full-size static background needs refreshing.
    pub unsafe fn render_regional(
        &mut self,
        width: i32,
        height: i32,
        progress: f32,
        region: [i32; 4],
    ) -> Result<bool> {
        let result = unsafe {
            self.source.render_target_at(
                width,
                height,
                progress,
                std::time::Instant::now(),
                Some(region),
            )
        };
        self.gpu_ms = self.source.profiler.as_ref().and_then(|p| p.latest_ms);
        result
    }
    /// # Safety
    /// Owning context and full-size GTK background framebuffer are current.
    pub unsafe fn present_background(&self) {
        unsafe {
            self.source.present_background();
        }
    }
    /// Opt-in diagnostics only. Unsupported timing must not prevent rendering.
    /// # Safety
    /// The owning OpenGL context must be current.
    pub unsafe fn enable_gpu_timing(&mut self, detailed: bool) {
        let result = (|| -> Result<()> {
            let symbol = unsafe {
                self._epoxy
                    .get::<*const gpu_profile::GetQueryIv>(b"epoxy_glGetQueryiv\0")
            }
            .context("Cannot resolve GPU timestamp capability query")?;
            self.source.profiler = Some(unsafe {
                gpu_profile::GpuProfiler::new(self.gl.clone(), **symbol, detailed)
            }?);
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!("GPU timing unavailable; rendering continues: {error:#}");
        }
    }

    /// Completed, originally tagged frames; taking samples never waits on GL.
    pub fn take_gpu_timings(&mut self) -> Vec<GpuFrameTiming> {
        self.source
            .profiler
            .as_mut()
            .map_or_else(Vec::new, |p| p.take())
    }
    /// Opt-in only for fresh, advancing music playback. None keeps exact rendering.
    pub fn set_arm_cache_target(&mut self, target: Option<f32>) {
        self.source.set_arm_cache_target(target);
    }
    pub fn set_sleeve_visible(&mut self, visible: bool) {
        self.source.set_sleeve_visible(visible);
    }
    pub fn knob_supported(&self) -> bool {
        self.source.knob_supported()
    }
    pub fn set_knob_angle(&mut self, angle: f32) -> bool {
        self.source.set_knob_angle(angle)
    }
    #[cfg(test)]
    pub(crate) fn knob_update_count(&self) -> u64 {
        self.source.knob_update_count()
    }
    pub fn buttons_supported(&self) -> bool {
        self.source.buttons_supported()
    }
    pub fn set_button_progress(&mut self, progress: [f32; 3]) -> bool {
        self.source.set_button_progress(progress)
    }
    #[cfg(test)]
    pub(crate) fn button_update_count(&self) -> u64 {
        self.source.button_update_count()
    }
    #[cfg(test)]
    pub(crate) fn cache_update_counts(&self) -> (u64, u64, u64) {
        self.source.cache_update_counts()
    }
    pub fn set_artwork_angle(&mut self, angle: f32) {
        self.source.set_artwork_angle(angle);
    }
    pub fn set_record_fallback_enabled(&mut self, enabled: bool) {
        self.source.set_record_fallback_enabled(enabled);
    }
    pub fn artwork_supported(&self) -> bool {
        self.source.artwork_supported()
    }
    pub fn record_artwork_supported(&self) -> bool {
        self.source.record_artwork_supported()
    }
    pub fn vinyl_supported(&self) -> bool {
        self.source.vinyl_supported()
    }
    pub fn scene_aspect(&self) -> f32 {
        self.source.scene_aspect()
    }
    /// # Safety
    /// The owning OpenGL context must be current.
    pub unsafe fn set_artwork(
        &mut self,
        revision: u64,
        image: Option<&image::RgbaImage>,
        settings: &crate::artwork_geometry::ArtworkSettings,
    ) -> Result<()> {
        unsafe { self.source.set_artwork(revision, image, settings) }
    }
    /// # Safety
    /// The owning context must be current. Uploads/draws only on artwork changes.
    pub unsafe fn set_previous_artwork(
        &mut self,
        revision: u64,
        image: Option<&image::RgbaImage>,
    ) -> Result<()> {
        unsafe { self.source.set_previous_artwork(revision, image) }
    }
    /// Initialize graphics resources using GTK's GL dispatcher.
    ///
    /// # Safety
    /// The caller must make the GLArea's OpenGL context current on this thread.
    pub unsafe fn new(asset: PreviewAsset) -> Result<Self> {
        unsafe {
            Self::new_impl(
                asset,
                #[cfg(test)]
                true,
            )
        }
    }
    // The unshared control is compiled only into tests; normal launches always
    // deduplicate identical LUTs. No user setting or process-global GL cache.
    unsafe fn new_impl(asset: PreviewAsset, #[cfg(test)] share_luts: bool) -> Result<Self> {
        let epoxy =
            unsafe { Library::new("libepoxy.so.0") }.context("Cannot load libepoxy.so.0")?;
        // libepoxy exports variables containing dispatch-function pointers. Keep the
        // library alive and read the stored pointer, not the variable's own address.
        let gl = Rc::new(unsafe {
            glow::Context::from_loader_function(|name| {
                let symbol_name = format!("epoxy_{name}\0");
                match epoxy.get::<*const *const c_void>(symbol_name.as_bytes()) {
                    Ok(address) => **address,
                    Err(_) => std::ptr::null(),
                }
            })
        });
        let info = RendererInfo {
            version: unsafe { gl.get_parameter_string(glow::VERSION) },
            shading_language: unsafe { gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION) },
            vendor: unsafe { gl.get_parameter_string(glow::VENDOR) },
            renderer: unsafe { gl.get_parameter_string(glow::RENDERER) },
            scene_size: match &asset.scene {
                crate::preview::PreviewScene::Beauty(image) => image.dimensions(),
                crate::preview::PreviewScene::Arm(assets) => assets.base.dimensions(),
            },
        };
        tracing::info!(
            version = %info.version,
            shading_language = %info.shading_language,
            vendor = %info.vendor,
            renderer = %info.renderer,
            software_rendering = info.software_rendering(),
            "OpenGL initialized"
        );
        let mut source = match asset.scene {
            crate::preview::PreviewScene::Beauty(image) => unsafe {
                beauty::GpuBeauty::upload(gl.clone(), image)
            },
            crate::preview::PreviewScene::Arm(assets) => unsafe {
                beauty::GpuBeauty::upload_arm(gl.clone(), *assets)
            },
        }?;
        source.set_scene_variant(asset.variant);
        #[cfg(test)]
        {
            source.view_luts.share = share_luts;
        }
        if let Some(stack) = asset.stack_sleeve {
            unsafe {
                source.upload_stack(stack)?;
            }
        }
        if let Some(empty_stand) = asset.empty_stand {
            unsafe {
                source.upload_empty_stand(empty_stand)?;
            }
        }
        if let Some(sleeve) = asset.sleeve {
            unsafe {
                source.upload_sleeve(sleeve)?;
            }
        }
        if let Some(lighting) = asset.record_lighting {
            unsafe {
                source.upload_record_lighting(lighting)?;
            }
        }
        if let Some(vinyl) = asset.vinyl_detail {
            unsafe {
                source.upload_vinyl(vinyl)?;
            }
        }
        if let Some(buttons) = asset.buttons {
            unsafe {
                source.upload_buttons(buttons)?;
            }
        }
        if let Some(knob) = asset.knob {
            unsafe {
                source.upload_knob(knob)?;
            }
        }
        Ok(Self {
            gl,
            _epoxy: epoxy,
            source,
            gpu_ms: None,
            info,
        })
    }

    /// Render into GTK's currently bound framebuffer, without assuming framebuffer zero.
    ///
    /// # Safety
    /// The owning OpenGL context and GLArea framebuffer must be current.
    pub unsafe fn render(&mut self, width: i32, height: i32, arm_progress: f32) -> Result<()> {
        unsafe {
            self.gpu_ms = None;
            let result = self.source.render(width, height, arm_progress);
            self.gpu_ms = self.source.profiler.as_ref().and_then(|p| p.latest_ms);
            result
        }
    }

    /// # Safety
    /// The GLArea framebuffer must be current. This diagnostic readback blocks.
    pub unsafe fn capture(&self, width: i32, height: i32, path: &std::path::Path) -> Result<()> {
        anyhow::ensure!(width > 0 && height > 0, "Cannot capture an empty viewport");
        let mut pixels = vec![0; width as usize * height as usize * 4];
        unsafe {
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            self.gl.read_pixels(
                0,
                0,
                width,
                height,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .context("Invalid captured framebuffer")?;
        image::imageops::flip_vertical(&image)
            .save(path)
            .with_context(|| format!("Cannot save {}", path.display()))?;
        tracing::info!(path = %path.display(), "Captured real OpenGL scene framebuffer");
        Ok(())
    }

    /// # Safety
    /// The owning GL context must be current; intended for smoke diagnostics only.
    pub unsafe fn check_error(&self) -> Result<()> {
        let error = unsafe { self.gl.get_error() };
        anyhow::ensure!(error == glow::NO_ERROR, "OpenGL error {error:#x}");
        Ok(())
    }

    /// Release GPU resources before the GLArea is unrealized.
    ///
    /// # Safety
    /// The owning OpenGL context must still be current on this thread.
    pub unsafe fn destroy(self) {
        drop(self.source);
    }
}
