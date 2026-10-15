//! Cached beauty display and a cropped Blender arm-pose compositor.
use super::{
    cache::{self, PoseKey, SceneCache},
    gpu_profile::{FrameInfo, GpuProfiler, GpuStage},
    utils::{compile, fitted_viewport},
};
use anyhow::{Result, ensure};
use glow::HasContext;
use std::rc::Rc;

mod resources;
use resources::Resources;

pub struct GpuBeauty {
    pub(super) view_luts: super::view_lut::ViewLutPool,
    pub(super) profiler: Option<GpuProfiler>,
    variant: crate::config::SceneVariant,
    knob: Option<super::knob::GpuKnob>,
    stack: Option<super::stack::GpuStack>,
    buttons: Option<super::buttons::GpuButtons>,
    resources: Resources,
    aspect: f32,
    sampler: Option<glow::NativeUniformLocation>,
    pose_count: usize,
    arm_cadence: Option<super::arm_cadence::ArmCadence>,
    arm_cache_target: Option<f32>,
    pose_uniform: Option<glow::NativeUniformLocation>,
    flow_pair_uniform: Option<glow::NativeUniformLocation>,
    artwork_revision: Option<u64>,
    has_foreground: bool,
    artwork_present: bool,
    record_fallback_enabled: bool,
    default_artwork: Option<image::RgbaImage>,
    #[cfg(test)]
    artwork_uploads: u64,
    artwork_enabled: bool,
    artwork_transform: [f32; 9],
    artwork_settings: Option<crate::artwork_geometry::ArtworkSettings>,
    artwork_uniforms: ArtworkUniforms,
    artwork_angle: f32,
    record_brightness: f32,
    sleeve: Option<super::sleeve::GpuPaperResponse>,
    record_lighting: Option<super::sleeve::GpuPaperResponse>,
    vinyl: Option<super::sleeve::GpuPaperResponse>,
    vinyl_crop: Option<[f32; 4]>,
    vinyl_baked_detail: bool,
    sleeve_enabled: bool,
    cache: SceneCache,
    record_uniforms: ArtworkUniforms,
    arm_crop: Option<[f32; 4]>,
    sleeve_crop: Option<[f32; 4]>,
    sleeve_visible: bool,
    base_dirty: bool,
}
struct ArtworkUniforms {
    sampler: Option<glow::NativeUniformLocation>,
    enabled: Option<glow::NativeUniformLocation>,
    transform: Option<glow::NativeUniformLocation>,
    rotation: Option<glow::NativeUniformLocation>,
    sleeve_enabled: Option<glow::NativeUniformLocation>,
    record_lighting_enabled: Option<glow::NativeUniformLocation>,
    record_brightness: Option<glow::NativeUniformLocation>,
    vinyl_enabled: Option<glow::NativeUniformLocation>,
    vinyl_appearance: Option<glow::NativeUniformLocation>,
    cache_size: Option<glow::NativeUniformLocation>,
    cache_origin: Option<glow::NativeUniformLocation>,
    scene_viewport: Option<glow::NativeUniformLocation>,
}
impl ArtworkUniforms {
    unsafe fn new(gl: &glow::Context, program: glow::NativeProgram) -> Self {
        unsafe {
            // Active 2D/3D samplers must have distinct units even when the sleeve
            // branch is disabled or optional lighting assets are unavailable.
            gl.use_program(Some(program));
            for (name, unit) in [
                ("uButtons", 12),
                ("uSleeveIllumination", 5),
                ("uSleeveResidual", 6),
                ("uSleeveView", 7),
                ("uRecordIllumination", 5),
                ("uRecordResidual", 6),
                ("uRecordView", 7),
                ("uVinylIllumination", 8),
                ("uVinylResidual", 9),
                ("uVinylView", 10),
                ("uVinylDetail", 11),
            ] {
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit);
            }
            gl.use_program(None);
            Self {
                sampler: gl.get_uniform_location(program, "uArtwork"),
                enabled: gl.get_uniform_location(program, "uArtworkEnabled"),
                transform: gl.get_uniform_location(program, "uArtworkTransform"),
                rotation: gl.get_uniform_location(program, "uArtworkRotation"),
                sleeve_enabled: gl.get_uniform_location(program, "uSleeveEnabled"),
                record_lighting_enabled: gl.get_uniform_location(program, "uRecordEnabled"),
                record_brightness: gl.get_uniform_location(program, "uRecordBrightness"),
                vinyl_enabled: gl.get_uniform_location(program, "uVinylEnabled"),
                vinyl_appearance: gl.get_uniform_location(program, "uVinylAppearance"),
                cache_size: gl.get_uniform_location(program, "uCacheSize"),
                cache_origin: gl.get_uniform_location(program, "uCacheOrigin"),
                scene_viewport: gl.get_uniform_location(program, "uSceneViewport"),
            }
        }
    }
}
fn fragment(source: &str) -> String {
    source
        .replace("// KNOB_FUNCTIONS", include_str!("../../shaders/knob.glsl"))
        .replace(
            "// BUTTON_FUNCTIONS",
            include_str!("../../shaders/buttons.glsl"),
        )
        .replace(
            "// ARTWORK_FUNCTIONS",
            include_str!("../../shaders/artwork.glsl"),
        )
        .replace(
            "// SLEEVE_FUNCTIONS",
            include_str!("../../shaders/sleeve.glsl"),
        )
        .replace(
            "// RECORD_LIGHTING_FUNCTIONS",
            include_str!("../../shaders/record_lighting.glsl"),
        )
        .replace(
            "// VINYL_FUNCTIONS",
            include_str!("../../shaders/vinyl_detail.glsl"),
        )
        .replace(
            "// VINYL_REFERENCE_FUNCTIONS",
            include_str!("../../shaders/vinyl_reference.glsl"),
        )
}
impl GpuBeauty {
    #[cfg(test)]
    pub(super) fn test_views(&self) -> Vec<glow::NativeTexture> {
        let mut views = self
            .stack
            .as_ref()
            .map_or_else(Vec::new, |s| vec![s.test_view()]);
        views.extend(
            [&self.sleeve, &self.record_lighting, &self.vinyl]
                .into_iter()
                .flatten()
                .map(|response| response.test_view()),
        );
        views
    }
    #[cfg(test)]
    pub(super) unsafe fn test_foreground(&self) -> Vec<u8> {
        unsafe { self.cache.test_foreground() }
    }
    #[cfg(test)]
    pub(super) fn stack_updates(&self) -> u64 {
        self.stack.as_ref().map_or(0, |s| s.updates)
    }
    #[cfg(test)]
    pub(super) unsafe fn test_arm_region(&mut self, region: [f32; 4]) {
        self.arm_crop = Some(region);
        self.base_dirty = true;
        unsafe {
            let gl = &self.resources.gl;
            gl.use_program(self.resources.program);
            gl.uniform_4_f32_slice(
                gl.get_uniform_location(self.resources.program.unwrap(), "uUpdateCrop")
                    .as_ref(),
                &region,
            );
            gl.use_program(None);
        }
    }
    pub(super) unsafe fn upload_stack(
        &mut self,
        assets: crate::sleeve_assets::SleeveAssets,
    ) -> Result<()> {
        self.stack = Some(unsafe { super::stack::GpuStack::upload(&mut self.view_luts, assets)? });
        Ok(())
    }
    pub(super) unsafe fn set_previous_artwork(
        &mut self,
        revision: u64,
        image: Option<&image::RgbaImage>,
    ) -> Result<()> {
        if let Some(stack) = &mut self.stack {
            unsafe {
                stack.set_artwork(revision, image)?;
            }
        }
        Ok(())
    }
    pub fn set_sleeve_visible(&mut self, visible: bool) {
        if self.sleeve_visible != visible {
            self.sleeve_visible = visible;
            self.base_dirty |= self.resources.empty_stand.is_some();
            self.cache.sleeve_dirty = true;
        }
    }
    pub(super) unsafe fn upload_empty_stand(
        &mut self,
        assets: crate::stand_assets::StandAssets,
    ) -> Result<()> {
        unsafe {
            let gl = &self.resources.gl;
            let image = assets.image;
            let limit = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
            ensure!(
                image.width() <= limit && image.height() <= limit,
                "Empty stand exceeds GPU texture limits"
            );
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            self.resources.empty_stand = Some(texture);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
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
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.base_dirty = true;
            Ok(())
        }
    }
    // Test-only A/B control; never changes sleeve sampling or artwork pixels.
    #[cfg(test)]
    pub(super) unsafe fn test_record_filter(&self, value: f32, mipmaps: bool) -> bool {
        unsafe {
            if let Some(sampler) = self.resources.record_sampler {
                let gl = &self.resources.gl;
                gl.sampler_parameter_f32(
                    sampler,
                    glow::TEXTURE_MAX_ANISOTROPY_EXT,
                    value.clamp(
                        1.0,
                        gl.get_parameter_f32(glow::MAX_TEXTURE_MAX_ANISOTROPY_EXT),
                    ),
                );
                gl.sampler_parameter_i32(
                    sampler,
                    glow::TEXTURE_MIN_FILTER,
                    if mipmaps {
                        glow::LINEAR_MIPMAP_LINEAR
                    } else {
                        glow::LINEAR
                    } as i32,
                );
                true
            } else {
                false
            }
        }
    }

    pub fn knob_supported(&self) -> bool {
        self.knob.is_some()
    }
    pub fn set_knob_angle(&mut self, angle: f32) -> bool {
        self.knob.as_mut().is_some_and(|knob| knob.set_angle(angle))
    }
    #[cfg(test)]
    pub(super) fn knob_update_count(&self) -> u64 {
        self.knob.as_ref().map_or(0, |k| k.updates)
    }
    pub unsafe fn upload_knob(&mut self, assets: crate::knob_assets::KnobAssets) -> Result<()> {
        self.knob = Some(unsafe {
            super::knob::GpuKnob::new(
                self.resources.gl.clone(),
                self.resources.program.unwrap(),
                assets,
            )?
        });
        Ok(())
    }
    pub(super) fn set_scene_variant(&mut self, variant: crate::config::SceneVariant) {
        self.variant = variant;
        // Applied only by the small record-artwork pass, after its calibrated
        // view transform. No rebake, extra pass or static-cache invalidation.
        self.record_brightness = match variant {
            crate::config::SceneVariant::Day => 1.0,
            crate::config::SceneVariant::Night => 0.85,
        };
    }
    pub fn buttons_supported(&self) -> bool {
        self.buttons.is_some()
    }
    pub fn set_button_progress(&mut self, progress: [f32; 3]) -> bool {
        self.buttons
            .as_mut()
            .is_some_and(|buttons| buttons.set_progress(progress))
    }
    #[cfg(test)]
    pub(super) fn button_update_count(&self) -> u64 {
        self.cache.button_updates
    }
    pub unsafe fn upload_buttons(
        &mut self,
        assets: crate::button_assets::ButtonAssets,
    ) -> Result<()> {
        self.buttons = Some(unsafe {
            super::buttons::GpuButtons::new(
                self.resources.gl.clone(),
                self.resources.program.unwrap(),
                assets,
            )?
        });
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn cache_update_counts(&self) -> (u64, u64, u64) {
        (
            self.cache.full_updates,
            self.cache.arm_updates,
            self.cache.sleeve_updates,
        )
    }
    pub fn set_artwork_angle(&mut self, angle: f32) {
        self.artwork_angle = angle;
    }
    /// Default center art is opt-in for an active player, never a sleeve/history source.
    pub fn set_record_fallback_enabled(&mut self, enabled: bool) {
        let enabled = enabled && self.has_foreground;
        if self.record_fallback_enabled != enabled {
            self.record_fallback_enabled = enabled;
            self.artwork_revision = None;
        }
    }
    fn record_artwork_active(&self) -> bool {
        self.artwork_enabled
            && self.has_foreground
            && (self.artwork_present || self.record_fallback_enabled)
    }
    #[cfg(test)]
    pub(super) fn test_record_draws(&self) -> u64 {
        self.cache.record_draws
    }
    #[cfg(test)]
    pub(super) fn test_artwork_uploads(&self) -> u64 {
        self.artwork_uploads
    }
    pub fn set_arm_cache_target(&mut self, target: Option<f32>) {
        self.arm_cache_target = target;
    }
    pub fn scene_aspect(&self) -> f32 {
        self.aspect
    }
    pub fn artwork_supported(&self) -> bool {
        self.has_foreground || self.sleeve.is_some()
    }
    pub fn record_artwork_supported(&self) -> bool {
        self.has_foreground
    }
    pub fn vinyl_supported(&self) -> bool {
        self.pose_count > 0 && self.has_foreground && self.vinyl.is_some()
    }
    /// # Safety
    /// The owning context must be current.
    pub unsafe fn upload_vinyl(&mut self, assets: crate::vinyl_assets::VinylAssets) -> Result<()> {
        unsafe {
            self.vinyl_baked_detail = assets.baked_detail;
            let gl = &self.resources.gl;
            ensure!(
                assets.size <= gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32,
                "Vinyl detail exceeds GPU texture limit"
            );
            let m = &assets.response.manifest;
            self.vinyl_crop = Some([
                m.crop[0] as f32 / m.width as f32,
                m.crop[1] as f32 / m.height as f32,
                m.crop[2] as f32 / m.width as f32,
                m.crop[3] as f32 / m.height as f32,
            ]);
            self.vinyl = Some(super::sleeve::GpuPaperResponse::upload(
                &mut self.view_luts,
                self.cache.program.unwrap(),
                assets.response,
                "uVinyl",
                8,
            )?);
            gl.use_program(self.cache.program);
            gl.uniform_1_i32(
                gl.get_uniform_location(self.cache.program.unwrap(), "uVinylBakedDetail")
                    .as_ref(),
                i32::from(assets.baked_detail),
            );
            gl.use_program(None);
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            self.resources.vinyl_detail = Some(texture);
            gl.active_texture(glow::TEXTURE11);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                assets.size as i32,
                assets.size as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&assets.detail)),
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
            // The record is strongly foreshortened. Optional anisotropic
            // filtering preserves sparse scratches without shimmer/white halos.
            if gl
                .supported_extensions()
                .contains("GL_EXT_texture_filter_anisotropic")
            {
                const MAX_ANISOTROPY: u32 = 0x84ff;
                const TEXTURE_ANISOTROPY: u32 = 0x84fe;
                gl.tex_parameter_f32(
                    glow::TEXTURE_2D,
                    TEXTURE_ANISOTROPY,
                    gl.get_parameter_f32(MAX_ANISOTROPY).min(8.0),
                );
            }
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            Ok(())
        }
    }
    /// # Safety
    /// The owning context must be current; call after selecting the final program.
    pub unsafe fn upload_sleeve(
        &mut self,
        assets: crate::sleeve_assets::SleeveAssets,
    ) -> Result<()> {
        let m = &assets.manifest;
        self.sleeve_crop = Some([
            m.crop[0] as f32 / m.width as f32,
            m.crop[1] as f32 / m.height as f32,
            m.crop[2] as f32 / m.width as f32,
            m.crop[3] as f32 / m.height as f32,
        ]);
        self.cache.sleeve_dirty = true;
        self.sleeve = Some(unsafe {
            super::sleeve::GpuPaperResponse::upload(
                &mut self.view_luts,
                self.resources.program.unwrap(),
                assets,
                "uSleeve",
                5,
            )?
        });
        Ok(())
    }
    /// # Safety
    /// The context must be current; lighting belongs only to the record pass.
    pub unsafe fn upload_record_lighting(
        &mut self,
        assets: crate::sleeve_assets::SleeveAssets,
    ) -> Result<()> {
        self.record_lighting = Some(unsafe {
            super::sleeve::GpuPaperResponse::upload(
                &mut self.view_luts,
                self.cache.program.unwrap(),
                assets,
                "uRecord",
                5,
            )?
        });
        Ok(())
    }
    /// # Safety
    /// The owning context must be current when uploading and dropping the image.
    pub unsafe fn upload(gl: Rc<glow::Context>, image: image::RgbaImage) -> Result<Self> {
        unsafe {
            let limit = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
            ensure!(
                image.width() > 0 && image.height() > 0,
                "Beauty image is empty"
            );
            ensure!(
                image.width() <= limit && image.height() <= limit,
                "Beauty exceeds the GPU texture limit ({limit}); render at a smaller resolution"
            );
            let mut resources = Resources {
                gl: gl.clone(),
                program: None,
                vao: None,
                texture: None,
                empty_stand: None,
                poses: None,
                flow: None,
                tube_background: None,
                artwork: None,
                record_sampler: None,
                vinyl_detail: None,
            };
            resources.program = Some(compile(
                &gl,
                include_str!("../../shaders/beauty.vert"),
                &fragment(include_str!("../../shaders/beauty.frag")),
            )?);
            resources.vao = Some(gl.create_vertex_array().map_err(anyhow::Error::msg)?);
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            resources.texture = Some(texture);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            // Not SRGB8_ALPHA8: the shader intentionally passes Blender's encoded RGB through.
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
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let sampler = gl.get_uniform_location(resources.program.unwrap(), "uBeauty");
            let artwork_uniforms = ArtworkUniforms::new(&gl, resources.program.unwrap());
            let cache = SceneCache::new(
                gl.clone(),
                &fragment(include_str!("../../shaders/record.frag")),
            )?;
            let record_uniforms = ArtworkUniforms::new(&gl, cache.program.unwrap());
            gl.use_program(cache.program);
            gl.uniform_1_i32(
                gl.get_uniform_location(cache.program.unwrap(), "uSceneCache")
                    .as_ref(),
                0,
            );
            gl.uniform_1_i32(
                gl.get_uniform_location(cache.program.unwrap(), "uForegroundCache")
                    .as_ref(),
                1,
            );
            gl.use_program(None);
            tracing::info!(
                texture_mib = image.as_raw().len() as f64 / (1024.0 * 1024.0),
                "Cached Blender beauty source; no runtime lighting or ray tracing"
            );
            Ok(Self {
                view_luts: super::view_lut::ViewLutPool::new(gl.clone()),
                profiler: None,
                variant: crate::config::SceneVariant::Day,
                buttons: None,
                knob: None,
                stack: None,
                resources,
                aspect: image.width() as f32 / image.height() as f32,
                sampler,
                pose_count: 0,
                arm_cadence: None,
                arm_cache_target: None,
                pose_uniform: None,
                flow_pair_uniform: None,
                artwork_revision: None,
                has_foreground: true,
                artwork_present: false,
                record_fallback_enabled: false,
                default_artwork: None,
                #[cfg(test)]
                artwork_uploads: 0,
                artwork_enabled: false,
                artwork_transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                artwork_settings: None,
                artwork_uniforms,
                artwork_angle: 0.0,
                record_brightness: 1.0,
                sleeve: None,
                record_lighting: None,
                vinyl: None,
                vinyl_crop: None,
                vinyl_baked_detail: false,
                sleeve_enabled: true,
                cache,
                record_uniforms,
                arm_crop: None,
                sleeve_crop: None,
                sleeve_visible: true,
                base_dirty: false,
            })
        }
    }
    /// # Safety
    /// The owning context must be current during upload and teardown.
    pub unsafe fn upload_arm(
        gl: Rc<glow::Context>,
        mut assets: crate::arm_assets::ArmPoseAssets,
    ) -> Result<Self> {
        unsafe {
            assets.manifest.validate()?;
            let has_foreground = assets
                .flow
                .as_ref()
                .is_some_and(|flow| flow.edges.is_some());
            if let Some(edges) = assets.flow.as_ref().and_then(|flow| flow.edges.as_ref()) {
                edges.apply_masks(&mut assets.poses)?;
            }
            let dimensions = (assets.manifest.crop[2], assets.manifest.crop[3]);
            ensure!(
                assets.base.dimensions() == (assets.manifest.width, assets.manifest.height)
                    && assets.poses.len() == assets.manifest.frames.len()
                    && assets
                        .poses
                        .iter()
                        .all(|image| image.dimensions() == dimensions),
                "Arm texture buffers do not match the manifest"
            );
            let mut beauty = Self::upload(gl.clone(), assets.base)?;
            beauty.has_foreground = has_foreground;
            let count = assets.poses.len();
            ensure!(
                count as i32 <= gl.get_parameter_i32(glow::MAX_ARRAY_TEXTURE_LAYERS),
                "Too many arm poses for this GPU"
            );
            let [x, y, w, h] = assets.manifest.crop;
            beauty.arm_crop = Some([
                x as f32 / assets.manifest.width as f32,
                y as f32 / assets.manifest.height as f32,
                w as f32 / assets.manifest.width as f32,
                h as f32 / assets.manifest.height as f32,
            ]);
            if let Some([ux, uy, uw, uh]) = assets.update_crop {
                beauty.arm_crop = Some([
                    ux as f32 / assets.manifest.width as f32,
                    uy as f32 / assets.manifest.height as f32,
                    uw as f32 / assets.manifest.width as f32,
                    uh as f32 / assets.manifest.height as f32,
                ]);
            }
            let interpolate = assets
                .flow
                .as_ref()
                .is_some_and(|flow| !flow.fields.is_empty());
            let shader = fragment(if interpolate {
                include_str!("../../shaders/arm_flow.frag")
            } else {
                include_str!("../../shaders/arm.frag")
            });
            let program = compile(&gl, include_str!("../../shaders/beauty.vert"), &shader)?;
            gl.delete_program(beauty.resources.program.replace(program).unwrap());
            beauty.sampler = gl.get_uniform_location(program, "uBeauty");
            beauty.artwork_uniforms = ArtworkUniforms::new(&gl, program);
            beauty.pose_uniform =
                gl.get_uniform_location(program, if interpolate { "uPose" } else { "uPoseLayer" });
            beauty.pose_count = count;
            beauty.upload_poses(&assets.poses, w, h)?;
            gl.use_program(Some(program));
            gl.uniform_4_f32_slice(
                gl.get_uniform_location(program, "uUpdateCrop").as_ref(),
                &beauty.arm_crop.unwrap(),
            );
            gl.uniform_1_i32(gl.get_uniform_location(program, "uPoses").as_ref(), 1);
            gl.uniform_4_f32(
                gl.get_uniform_location(program, "uCrop").as_ref(),
                x as f32 / assets.manifest.width as f32,
                y as f32 / assets.manifest.height as f32,
                w as f32 / assets.manifest.width as f32,
                h as f32 / assets.manifest.height as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "uCropPixels").as_ref(),
                w as f32,
                h as f32,
            );
            if let Some(flow) = assets.flow {
                if interpolate {
                    beauty.upload_flow(flow, &assets.manifest)?;
                }
            }
            gl.use_program(None);
            tracing::info!(
                poses = count,
                pose_texture_mib = w as f64 * h as f64 * 4.0 * count as f64 / 1048576.0,
                flow = beauty.resources.flow.is_some(),
                "Arm pose array cached; changed crops feed the retained scene"
            );
            Ok(beauty)
        }
    }

    /// Upload pose layers; ownership is registered before subsequent GL calls.
    unsafe fn upload_poses(&mut self, poses: &[image::RgbaImage], w: u32, h: u32) -> Result<()> {
        unsafe {
            let gl = self.resources.gl.clone();
            let count = poses.len();
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            self.resources.poses = Some(texture);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.tex_image_3d(
                glow::TEXTURE_2D_ARRAY,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                count as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (index, image) in poses.iter().enumerate() {
                gl.tex_sub_image_3d(
                    glow::TEXTURE_2D_ARRAY,
                    0,
                    0,
                    0,
                    index as i32,
                    w as i32,
                    h as i32,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(image.as_raw())),
                );
            }
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D_ARRAY,
                    parameter,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);

            Ok(())
        }
    }

    /// Upload bidirectional fields and optional mixed-edge correction.
    unsafe fn upload_flow(
        &mut self,
        mut flow: crate::arm_flow::ArmFlowAssets,
        manifest: &crate::arm_assets::PoseManifest,
    ) -> Result<()> {
        unsafe {
            let gl = self.resources.gl.clone();
            let count = self.pose_count;
            let program = self.resources.program.unwrap();
            flow.manifest.validate(manifest)?;
            ensure!(
                flow.max_displacements.len() == flow.fields.len(),
                "Arm motion bounds do not match the flow fields"
            );
            self.arm_cadence = Some(super::arm_cadence::ArmCadence::new(
                (manifest.width, manifest.height),
                &flow.max_displacements,
            ));
            let (fw, fh) = (flow.manifest.width, flow.manifest.height);
            ensure!(
                flow.fields.len() == count - 1
                    && flow
                        .fields
                        .iter()
                        .all(|field| field.dimensions() == (fw, fh)),
                "Flow texture buffers do not match the manifest"
            );
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            self.resources.flow = Some(texture);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.tex_image_3d(
                glow::TEXTURE_2D_ARRAY,
                0,
                glow::RGBA16 as i32,
                fw as i32,
                fh as i32,
                (count - 1) as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_SHORT,
                glow::PixelUnpackData::Slice(None),
            );
            for (index, field) in flow.fields.iter().enumerate() {
                let bytes: Vec<u8> = field
                    .as_raw()
                    .iter()
                    .flat_map(|value| value.to_ne_bytes())
                    .collect();
                gl.tex_sub_image_3d(
                    glow::TEXTURE_2D_ARRAY,
                    0,
                    0,
                    0,
                    index as i32,
                    fw as i32,
                    fh as i32,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_SHORT,
                    glow::PixelUnpackData::Slice(Some(&bytes)),
                );
            }
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D_ARRAY,
                    parameter,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.uniform_1_i32(gl.get_uniform_location(program, "uFlow").as_ref(), 2);
            self.flow_pair_uniform = gl.get_uniform_location(program, "uFlowPair");
            if let Some(edges) = flow.edges.take() {
                self.upload_background(&edges.background)?;
            }
            tracing::info!(
                flow_texture_mib = fw as f64 * fh as f64 * 8.0 * (count - 1) as f64 / 1048576.0,
                "Bidirectional motion interpolation; no flow estimation per frame"
            );

            Ok(())
        }
    }

    unsafe fn upload_background(&mut self, background: &image::RgbaImage) -> Result<()> {
        unsafe {
            let gl = self.resources.gl.clone();
            let program = self.resources.program.unwrap();
            let (w, h) = background.dimensions();
            let texture = gl.create_texture().map_err(anyhow::Error::msg)?;
            self.resources.tube_background = Some(texture);
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(background.as_raw())),
            );
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.uniform_1_i32(
                gl.get_uniform_location(program, "uTubeBackground").as_ref(),
                3,
            );
            gl.uniform_1_i32(gl.get_uniform_location(program, "uEdgeLayers").as_ref(), 1);
            tracing::info!(
                texture_mib = w as f64 * h as f64 * 4.0 / 1048576.0,
                "Background edge correction; foreground masks use existing pose alpha"
            );

            Ok(())
        }
    }
    /// # Safety
    /// The owning OpenGL context must be current. Image upload happens only on revision change.
    pub unsafe fn set_artwork(
        &mut self,
        revision: u64,
        image: Option<&image::RgbaImage>,
        settings: &crate::artwork_geometry::ArtworkSettings,
    ) -> Result<()> {
        if self.artwork_settings.as_ref() != Some(settings) {
            if self.artwork_enabled != settings.enabled
                || self.sleeve_enabled != settings.sleeve_enabled
                || self
                    .artwork_settings
                    .as_ref()
                    .map(|old| old.sleeve_shadow_lift)
                    != Some(settings.sleeve_shadow_lift)
            {
                self.cache.sleeve_dirty = true;
            }
            self.artwork_transform = settings.transform()?;
            self.artwork_enabled = settings.enabled;
            self.sleeve_enabled = settings.sleeve_enabled;
            self.artwork_settings = Some(settings.clone());
        }
        if self.artwork_revision == Some(revision) {
            return Ok(());
        }
        // A default-center change must not redraw the stationary sleeve.
        if self.artwork_present || image.is_some() {
            self.cache.sleeve_dirty = true;
        }
        if image.is_none() && self.record_fallback_enabled && self.default_artwork.is_none() {
            self.default_artwork = Some(
                image::load_from_memory(include_bytes!(
                    "../../assets/artwork/fallbacks/record-label.png"
                ))?
                .to_rgba8(),
            );
        }
        unsafe {
            let gl = &self.resources.gl;
            self.artwork_present = image.is_some();
            let image = image.or(if self.record_fallback_enabled {
                self.default_artwork.as_ref()
            } else {
                None
            });
            if let Some(image) = image {
                if self.resources.artwork.is_none() {
                    self.resources.artwork = Some(gl.create_texture().map_err(anyhow::Error::msg)?);
                }
                if self.resources.record_sampler.is_none()
                    && (gl
                        .supported_extensions()
                        .contains("GL_EXT_texture_filter_anisotropic")
                        || gl
                            .supported_extensions()
                            .contains("GL_ARB_texture_filter_anisotropic"))
                {
                    // A label-only sampler keeps the shared sleeve texture's
                    // filtering unchanged. Cap anisotropy; no extra texture/pass.
                    let sampler = gl.create_sampler().map_err(anyhow::Error::msg)?;
                    self.resources.record_sampler = Some(sampler);
                    gl.sampler_parameter_i32(
                        sampler,
                        glow::TEXTURE_MIN_FILTER,
                        glow::LINEAR_MIPMAP_LINEAR as i32,
                    );
                    gl.sampler_parameter_i32(
                        sampler,
                        glow::TEXTURE_MAG_FILTER,
                        glow::LINEAR as i32,
                    );
                    for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                        gl.sampler_parameter_i32(sampler, parameter, glow::CLAMP_TO_EDGE as i32);
                    }
                    gl.sampler_parameter_f32(
                        sampler,
                        glow::TEXTURE_MAX_ANISOTROPY_EXT,
                        gl.get_parameter_f32(glow::MAX_TEXTURE_MAX_ANISOTROPY_EXT)
                            .clamp(1.0, 8.0),
                    );
                }
                gl.active_texture(glow::TEXTURE4);
                gl.bind_texture(glow::TEXTURE_2D, self.resources.artwork);
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
                for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
                }
                // One shared upload and mip chain for sleeve and rotating label
                // minification; generated only when the artwork changes.
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_LINEAR as i32,
                );
                for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
                }
                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.active_texture(glow::TEXTURE0);
                #[cfg(test)]
                {
                    self.artwork_uploads += 1;
                }
                tracing::info!(
                    width = image.width(),
                    height = image.height(),
                    fallback = !self.artwork_present,
                    "Center artwork uploaded once; sleeve uses fetched artwork only"
                );
            }
            self.artwork_revision = Some(revision);
        }
        Ok(())
    }
    /// # Safety
    /// GTK's context and GLArea framebuffer must be current.
    pub unsafe fn render(&mut self, width: i32, height: i32, arm_progress: f32) -> Result<()> {
        unsafe { self.render_at(width, height, arm_progress, std::time::Instant::now()) }
    }
    /// Injected monotonic timestamp for deterministic cadence regressions.
    /// Safety: same current-context/framebuffer requirements as `render`.
    pub(super) unsafe fn render_at(
        &mut self,
        width: i32,
        height: i32,
        arm_progress: f32,
        now: std::time::Instant,
    ) -> Result<()> {
        unsafe {
            self.render_target_at(width, height, arm_progress, now, None)
                .map(|_| ())
        }
    }
    /// Conservative union of all continually changing calibrated surfaces.
    pub(super) fn regional_rect(&self, width: i32, height: i32, scale: i32) -> Option<[i32; 4]> {
        let mut crops: Vec<[f32; 4]> = self.arm_crop.into_iter().collect();
        crops.extend(self.vinyl_crop);
        crops.extend(self.buttons.as_ref().map(|b| b.crop));
        crops.extend(self.knob.as_ref().map(|k| k.crop));
        if let Some(settings) = &self.artwork_settings {
            let corners = &settings.corners;
            let left = corners.iter().map(|p| p[0]).fold(1.0_f64, f64::min);
            let top = corners.iter().map(|p| p[1]).fold(1.0_f64, f64::min);
            let right = corners.iter().map(|p| p[0]).fold(0.0_f64, f64::max);
            let bottom = corners.iter().map(|p| p[1]).fold(0.0_f64, f64::max);
            crops.push([
                left as f32,
                top as f32,
                (right - left) as f32,
                (bottom - top) as f32,
            ]);
        }
        super::regional::region(
            &crops,
            fitted_viewport(width, height, self.aspect),
            (width, height),
            scale,
        )
    }
    pub(super) unsafe fn present_background(&self) {
        unsafe {
            let gl = &self.resources.gl;
            let read = cache::framebuffer(gl, glow::READ_FRAMEBUFFER_BINDING);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::FRAMEBUFFER_SRGB);
            // Separate native roots may snapshot the background before the
            // first animation frame. The cache FBO has no attachments until
            // resize/composition, so even a zero-size blit raises 0x506.
            // Initialize only GTK's valid target; the first composed frame
            // requests a background refresh with the complete scene pixels.
            if self.cache.pose.is_none() {
                gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.0, 0.0, 0.0, 1.0]);
            } else {
                self.cache.present();
            }
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        }
    }
    pub(super) unsafe fn render_target_at(
        &mut self,
        width: i32,
        height: i32,
        arm_progress: f32,
        now: std::time::Instant,
        region: Option<[i32; 4]>,
    ) -> Result<bool> {
        if width <= 0 || height <= 0 {
            if let Some(profiler) = &mut self.profiler {
                profiler.latest_ms = None;
            }
            return Ok(false);
        }
        if let Some(profiler) = &mut self.profiler {
            profiler.begin_frame(FrameInfo {
                id: 0,
                dimensions: (width, height),
                scene: self.variant,
                submitted_at: now,
                transition: self.cache.dimensions != (width, height)
                    || self.base_dirty
                    || self.stack.as_ref().is_some_and(|s| s.dirty)
                    || self.cache.sleeve_dirty
                    || self.buttons.as_ref().is_some_and(|b| b.dirty)
                    || self.knob.as_ref().is_some_and(|k| k.dirty),
            });
        }
        let result = unsafe { self.render_scene_at(width, height, arm_progress, now, region) };
        if let Some(profiler) = &mut self.profiler {
            profiler.end_frame(result.is_ok());
        }
        result
    }
    fn begin_stage(&mut self) -> Option<usize> {
        self.profiler.as_mut().and_then(|p| p.begin_stage())
    }
    fn end_stage(&mut self, stage: GpuStage, begin: Option<usize>) {
        if let Some(profiler) = &mut self.profiler {
            profiler.end_stage(stage, begin);
        }
    }
    unsafe fn render_scene_at(
        &mut self,
        width: i32,
        height: i32,
        arm_progress: f32,
        now: std::time::Instant,
        region: Option<[i32; 4]>,
    ) -> Result<bool> {
        unsafe {
            let gl = self.resources.gl.clone();
            let draw = cache::framebuffer(&gl, glow::DRAW_FRAMEBUFFER_BINDING);
            let read = cache::framebuffer(&gl, glow::READ_FRAMEBUFFER_BINDING);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.color_mask(true, true, true, true);
            let resized = self.cache.resize(width, height);
            // Never leave GTK's target replaced on a resource-allocation error.
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            let resized = resized?;
            let viewport = fitted_viewport(width, height, self.aspect);
            let vinyl_enabled = self.vinyl_supported()
                && self.artwork_settings.as_ref().is_some_and(|s| {
                    s.vinyl_rotation && (s.vinyl_opacity > 0.0 || self.vinyl_baked_detail)
                });
            let background_changed = resized
                || self.base_dirty
                || self.cache.sleeve_dirty
                || self.stack.as_ref().is_some_and(|s| s.dirty);
            let [origin_x, origin_y, output_width, output_height] =
                region.unwrap_or([0, 0, width, height]);
            ensure!(
                origin_x >= 0
                    && origin_y >= 0
                    && output_width > 0
                    && output_height > 0
                    && origin_x
                        .checked_add(output_width)
                        .is_some_and(|x| x <= width)
                    && origin_y
                        .checked_add(output_height)
                        .is_some_and(|y| y <= height),
                "Invalid regional presentation target"
            );
            // All partial updates must use the SAME retained arm pose. Mixing a
            // fresh pose into a sleeve/button/knob overlap would split RGB and
            // foreground coverage across the cached scene.
            let arm_progress = if resized || self.base_dirty || self.cache.pose.is_none() {
                arm_progress
            } else {
                self.arm_cadence.as_ref().map_or(arm_progress, |cadence| {
                    cadence.select(
                        arm_progress,
                        self.arm_cache_target,
                        (viewport.2, viewport.3),
                        now,
                    )
                })
            };
            let pose = PoseKey::new(arm_progress, self.pose_count, self.resources.flow.is_some());
            let arm_changed = resized || self.base_dirty || self.cache.pose != Some(pose);
            self.cache.bind();
            if resized || self.base_dirty {
                let timing = self.begin_stage();
                gl.viewport(0, 0, width, height);
                gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.035, 0.025, 0.018, 1.0]);
                gl.clear_buffer_f32_slice(glow::COLOR, 1, &[0.0, 0.0, 0.0, 0.0]);
                self.draw_scene(width, height, arm_progress);
                self.end_stage(GpuStage::FullScene, timing);
                self.cache.full_updates += 1;
            } else {
                gl.enable(glow::SCISSOR_TEST);
                if self.cache.pose != Some(pose) {
                    if let Some(crop) = self.arm_crop {
                        let [x, y, w, h] = cache::region(crop, viewport);
                        gl.scissor(x, y, w, h);
                        let timing = self.begin_stage();
                        self.draw_scene(width, height, arm_progress);
                        self.end_stage(GpuStage::Arm, timing);
                        self.cache.arm_updates += 1;
                    }
                }
                if self.cache.sleeve_dirty {
                    if let Some(crop) = self.sleeve_crop {
                        let [x, y, w, h] = cache::region(crop, viewport);
                        gl.scissor(x, y, w, h);
                        let timing = self.begin_stage();
                        self.draw_scene(width, height, arm_progress);
                        self.end_stage(GpuStage::Sleeve, timing);
                        self.cache.sleeve_updates += 1;
                    }
                }
                if let Some(buttons) = &self.buttons {
                    if buttons.dirty {
                        let [x, y, w, h] = cache::region(buttons.crop, viewport);
                        gl.scissor(x, y, w, h);
                        let timing = self.begin_stage();
                        self.draw_scene(width, height, arm_progress);
                        self.end_stage(GpuStage::Buttons, timing);
                        self.cache.button_updates += 1;
                    }
                }
            }
            // Recompose only this tiny retained patch. Arm/sleeve updates that
            // overlap it already include the same fixed knob/marker shader.
            if let Some(knob) = &self.knob {
                if !resized && knob.dirty {
                    gl.enable(glow::SCISSOR_TEST);
                    let [x, y, w, h] = cache::region(knob.crop, viewport);
                    gl.scissor(x, y, w, h);
                    let timing = self.begin_stage();
                    self.draw_scene(width, height, arm_progress);
                    self.end_stage(GpuStage::Knob, timing);
                }
            }
            if let Some(knob) = &mut self.knob {
                if resized || knob.dirty {
                    knob.updates += 1;
                }
                knob.dirty = false;
            }
            if let Some(buttons) = &mut self.buttons {
                buttons.dirty = false;
            }
            self.cache.pose = Some(pose);
            if arm_changed {
                if let Some(cadence) = &mut self.arm_cadence {
                    cadence.commit(arm_progress, now);
                }
            }
            let stack_changed = self.stack.as_ref().is_some_and(|stack| {
                stack.dirty
                    || resized
                    || self.base_dirty
                    || (arm_changed
                        && self.arm_crop.is_some_and(|a| {
                            a[0] < stack.crop[0] + stack.crop[2]
                                && a[0] + a[2] > stack.crop[0]
                                && a[1] < stack.crop[1] + stack.crop[3]
                                && a[1] + a[3] > stack.crop[1]
                        }))
            });
            if stack_changed {
                let crop = self.stack.as_ref().unwrap().crop;
                let [x, y, w, h] = cache::region(crop, viewport);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(x, y, w, h);
                gl.viewport(viewport.0, viewport.1, viewport.2, viewport.3);
                let timing = self.begin_stage();
                // The empty-stand bake changes only the left sleeve. Keep this
                // independent right-hand patch on its calibrated beauty base:
                // separate bake noise must not alter it when playback stops.
                self.stack.as_mut().unwrap().draw(self.resources.texture);
                self.end_stage(GpuStage::PreviousSleeve, timing);
            }
            self.cache.sleeve_dirty = false;
            self.base_dirty = false;
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            let timing = self.begin_stage();
            self.cache
                .present_region([origin_x, origin_y, output_width, output_height]);
            self.end_stage(GpuStage::Presentation, timing);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
            if self.record_artwork_active() || vinyl_enabled {
                let timing = self.begin_stage();
                // Bound the label pass to the original fitted vinyl quadrilateral;
                // size changes remain inside it, and no room/sleeve shaders run.
                // The authored surface fit is independent of label diameter/fit.
                let corners = &self.artwork_settings.as_ref().unwrap().corners;
                let mut left = corners.iter().map(|p| p[0]).fold(1.0, f64::min);
                let mut right = corners.iter().map(|p| p[0]).fold(0.0, f64::max);
                let mut top = corners.iter().map(|p| p[1]).fold(1.0, f64::min);
                let mut bottom = corners.iter().map(|p| p[1]).fold(0.0, f64::max);
                if vinyl_enabled {
                    let [x, y, w, h] = self.vinyl_crop.unwrap();
                    left = left.min(x as f64);
                    right = right.max((x + w) as f64);
                    top = top.min(y as f64);
                    bottom = bottom.max((y + h) as f64);
                }
                let [x, y, w, h] = cache::region(
                    [
                        left as f32,
                        top as f32,
                        (right - left) as f32,
                        (bottom - top) as f32,
                    ],
                    viewport,
                );
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(x - origin_x, y - origin_y, w, h);
                gl.viewport(
                    viewport.0 - origin_x,
                    viewport.1 - origin_y,
                    viewport.2,
                    viewport.3,
                );
                let program = self.cache.program.unwrap();
                gl.use_program(Some(program));
                gl.uniform_1_f32(
                    self.record_uniforms.record_brightness.as_ref(),
                    self.record_brightness,
                );
                if let Some(settings) = &self.artwork_settings {
                    gl.uniform_2_f32(
                        self.record_uniforms.vinyl_appearance.as_ref(),
                        settings.vinyl_opacity as f32,
                        settings.vinyl_brightness as f32,
                    );
                }
                self.cache.bind_record();
                if let Some(vinyl) = &self.vinyl {
                    vinyl.bind(vinyl_enabled, 0.0);
                    gl.active_texture(glow::TEXTURE11);
                    gl.bind_texture(glow::TEXTURE_2D, self.resources.vinyl_detail);
                } else {
                    gl.uniform_1_i32(self.record_uniforms.vinyl_enabled.as_ref(), 0);
                }
                if let Some(lighting) = &self.record_lighting {
                    lighting.bind(
                        self.artwork_settings
                            .as_ref()
                            .is_some_and(|settings| settings.record_lighting),
                        0.0,
                    );
                } else {
                    gl.uniform_1_i32(self.record_uniforms.record_lighting_enabled.as_ref(), 0);
                }
                gl.uniform_2_f32(
                    self.record_uniforms.cache_size.as_ref(),
                    width as f32,
                    height as f32,
                );
                gl.uniform_2_f32(
                    self.record_uniforms.cache_origin.as_ref(),
                    origin_x as f32,
                    origin_y as f32,
                );
                gl.uniform_4_f32(
                    self.record_uniforms.scene_viewport.as_ref(),
                    viewport.0 as f32,
                    viewport.1 as f32,
                    viewport.2 as f32,
                    viewport.3 as f32,
                );
                gl.uniform_1_i32(self.record_uniforms.sampler.as_ref(), 4);
                gl.uniform_1_i32(
                    self.record_uniforms.enabled.as_ref(),
                    i32::from(self.record_artwork_active()),
                );
                gl.uniform_matrix_3_f32_slice(
                    self.record_uniforms.transform.as_ref(),
                    false,
                    &self.artwork_transform,
                );
                let (sine, cosine) = self.artwork_angle.sin_cos();
                gl.uniform_2_f32(self.record_uniforms.rotation.as_ref(), cosine, sine);
                gl.active_texture(glow::TEXTURE4);
                gl.bind_texture(glow::TEXTURE_2D, self.resources.artwork);
                gl.bind_sampler(4, self.resources.record_sampler);
                gl.bind_vertex_array(self.resources.vao);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                gl.bind_sampler(4, None);
                gl.bind_vertex_array(None);
                if let Some(lighting) = &self.record_lighting {
                    lighting.unbind();
                }
                if let Some(vinyl) = &self.vinyl {
                    vinyl.unbind();
                }
                for unit in [0, 1, 4, 11] {
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, None);
                }
                self.cache.record_draws += 1;
                self.end_stage(GpuStage::Record, timing);
            }
            gl.disable(glow::SCISSOR_TEST);
            gl.active_texture(glow::TEXTURE0);
            gl.use_program(None);
            Ok(background_changed)
        }
    }
    /// Recompute source RGB and foreground coverage only inside the active scissor.
    unsafe fn draw_scene(&self, width: i32, height: i32, arm_progress: f32) {
        unsafe {
            if width <= 0 || height <= 0 {
                return;
            }
            let gl = &self.resources.gl;
            let (x, y, w, h) = fitted_viewport(width, height, self.aspect);
            gl.viewport(x, y, w, h);
            gl.use_program(self.resources.program);
            if let Some(knob) = &self.knob {
                knob.bind();
            }
            if let Some(buttons) = &self.buttons {
                buttons.bind();
            }
            if let Some(sleeve) = &self.sleeve {
                sleeve.bind(
                    self.sleeve_visible
                        && self.artwork_enabled
                        && self.sleeve_enabled
                        && self.artwork_present,
                    self.artwork_settings
                        .as_ref()
                        .map_or(0.0, |settings| settings.sleeve_shadow_lift as f32),
                );
            } else {
                gl.uniform_1_i32(self.artwork_uniforms.sleeve_enabled.as_ref(), 0);
            }
            gl.uniform_1_i32(self.artwork_uniforms.sampler.as_ref(), 4);
            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, self.resources.artwork);
            gl.active_texture(glow::TEXTURE0);
            let texture = if self.sleeve_visible {
                self.resources.texture
            } else {
                self.resources.empty_stand.or(self.resources.texture)
            };
            gl.bind_texture(glow::TEXTURE_2D, texture);
            gl.uniform_1_i32(self.sampler.as_ref(), 0);
            if self.pose_count > 0 {
                if self.resources.flow.is_some() {
                    let (a, b, t) =
                        crate::arm_assets::interpolation_samples(arm_progress, self.pose_count);
                    gl.uniform_3_f32(self.pose_uniform.as_ref(), a as f32, b as f32, t);
                    gl.uniform_1_f32(
                        self.flow_pair_uniform.as_ref(),
                        a.min(self.pose_count - 2) as f32,
                    );
                    gl.active_texture(glow::TEXTURE2);
                    gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.resources.flow);
                    if self.resources.tube_background.is_some() {
                        gl.active_texture(glow::TEXTURE3);
                        gl.bind_texture(glow::TEXTURE_2D, self.resources.tube_background);
                    }
                } else {
                    let pose = crate::arm_assets::nearest_pose_index(arm_progress, self.pose_count);
                    gl.uniform_1_f32(self.pose_uniform.as_ref(), pose as f32);
                }
                gl.active_texture(glow::TEXTURE1);
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, self.resources.poses);
            }
            gl.bind_vertex_array(self.resources.vao);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
            if let Some(knob) = &self.knob {
                knob.unbind();
            }
            if let Some(sleeve) = &self.sleeve {
                sleeve.unbind();
            }
            gl.active_texture(glow::TEXTURE4);
            gl.bind_texture(glow::TEXTURE_2D, None);
            if self.resources.tube_background.is_some() {
                gl.active_texture(glow::TEXTURE3);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            if self.resources.flow.is_some() {
                gl.active_texture(glow::TEXTURE2);
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
                gl.active_texture(glow::TEXTURE1);
            }
            if self.pose_count > 0 {
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE12);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.active_texture(glow::TEXTURE0);
            gl.use_program(None);
        }
    }
}
