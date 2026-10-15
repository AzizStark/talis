//! GL resource ownership and failure-safe teardown. The owning context must be current.
use glow::HasContext;
use std::rc::Rc;

pub(super) struct Resources {
    pub(super) gl: Rc<glow::Context>,
    pub(super) program: Option<glow::NativeProgram>,
    pub(super) vao: Option<glow::NativeVertexArray>,
    pub(super) texture: Option<glow::NativeTexture>,
    pub(super) empty_stand: Option<glow::NativeTexture>,
    pub(super) poses: Option<glow::NativeTexture>,
    pub(super) flow: Option<glow::NativeTexture>,
    pub(super) tube_background: Option<glow::NativeTexture>,
    pub(super) artwork: Option<glow::NativeTexture>,
    pub(super) record_sampler: Option<glow::NativeSampler>,
    pub(super) vinyl_detail: Option<glow::NativeTexture>,
}
impl Drop for Resources {
    fn drop(&mut self) {
        // upload errors and normal teardown both occur with the owning context current.
        unsafe {
            if let Some(sampler) = self.record_sampler {
                self.gl.delete_sampler(sampler);
            }
            if let Some(texture) = self.vinyl_detail {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.artwork {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.tube_background {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.flow {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.poses {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.texture {
                self.gl.delete_texture(texture);
            }
            if let Some(texture) = self.empty_stand {
                self.gl.delete_texture(texture);
            }
            if let Some(vao) = self.vao {
                self.gl.delete_vertex_array(vao);
            }
            if let Some(program) = self.program {
                self.gl.delete_program(program);
            }
        }
    }
}
