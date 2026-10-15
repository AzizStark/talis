//! Shared shader compilation and fixed-camera viewport fitting.
use anyhow::{Result, ensure};
use glow::HasContext;

/// # Safety
/// The owning OpenGL context must be current.
pub unsafe fn compile(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::NativeProgram> {
    unsafe {
        let program = gl.create_program().map_err(anyhow::Error::msg)?;
        let mut shaders = Vec::new();
        let result = (|| -> Result<()> {
            for (kind, source) in [
                (glow::VERTEX_SHADER, vertex),
                (glow::FRAGMENT_SHADER, fragment),
            ] {
                let shader = gl.create_shader(kind).map_err(anyhow::Error::msg)?;
                shaders.push(shader);
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                ensure!(
                    gl.get_shader_compile_status(shader),
                    "Shader compile failed: {}",
                    gl.get_shader_info_log(shader)
                );
                gl.attach_shader(program, shader);
            }
            gl.link_program(program);
            ensure!(
                gl.get_program_link_status(program),
                "Program link failed: {}",
                gl.get_program_info_log(program)
            );
            Ok(())
        })();
        for shader in shaders {
            gl.delete_shader(shader);
        }
        if let Err(error) = result {
            gl.delete_program(program);
            return Err(error);
        }
        Ok(program)
    }
}

pub use crate::transport_geometry::fitted_viewport;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_aspect_is_preserved() {
        assert_eq!(fitted_viewport(1600, 1000, 16.0 / 9.0), (0, 50, 1600, 900));
        assert_eq!(fitted_viewport(2000, 900, 16.0 / 9.0), (200, 0, 1600, 900));
    }
}
