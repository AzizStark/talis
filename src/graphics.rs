//! Backend selection before GTK initialization; one X11 context-failure retry.
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, os::unix::process::CommandExt, process::Command};

/// Only failure to create the scene's actual GL context may select a fallback.
/// Shader, texture, asset and later rendering errors must retain their identity.
#[derive(Debug)]
pub(crate) struct ContextInitializationError {
    reason: String,
    native_x11: bool,
}
impl ContextInitializationError {
    pub(crate) fn new(reason: String, native_x11: bool) -> Self {
        Self { reason, native_x11 }
    }
}
impl std::fmt::Display for ContextInitializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OpenGL initialization failed: GTK could not create the scene GL context: {}",
            self.reason
        )
    }
}
impl std::error::Error for ContextInitializationError {}

fn has_flag(value: &str, flags: &[&str]) -> bool {
    value
        .split(|c: char| c == ',' || c == ':' || c == ';' || c.is_ascii_whitespace())
        .any(|value| flags.iter().any(|flag| flag.eq_ignore_ascii_case(value)))
}

fn eligible(error: &anyhow::Error, mode: &str, wayland: bool, debug: &str, disabled: &str) -> bool {
    error
        .downcast_ref::<ContextInitializationError>()
        .is_some_and(|error| error.native_x11)
        && mode == "auto"
        && !wayland
        && !has_flag(
            debug,
            &[
                "all",
                "gl-disable",
                "gl-glx",
                "gl-egl",
                "gl-gles",
                "gl-legacy",
            ],
        )
        && !has_flag(disabled, &["all", "gl", "gl-api", "gles-api", "egl", "glx"])
}

fn recovery_command(
    executable: &OsStr,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments).env("TALIS_GL_BACKEND", "glx");
    command
}

/// Called only after the failed application has shut down and released its
/// desktop lease. exec preserves arguments, cwd and PID without a second owner.
/// Environment changes belong to the new process, never to GTK's worker threads.
pub fn recover(error: anyhow::Error) -> Result<()> {
    let mode = std::env::var("TALIS_GL_BACKEND").unwrap_or_else(|_| "auto".into());
    let wayland = crate::desktop::Session::detect() == crate::desktop::Session::Wayland;
    let debug = std::env::var("GDK_DEBUG").unwrap_or_default();
    let disabled = std::env::var("GDK_DISABLE").unwrap_or_default();
    if !eligible(&error, &mode, wayland, &debug, &disabled) {
        return if error.is::<ContextInitializationError>() && mode == "glx" {
            Err(error.context("GLX context creation failed; no further backend retries"))
        } else {
            Err(error)
        };
    }
    let executable = std::env::current_exe().context("Locate Talis for GLX recovery")?;
    tracing::warn!(%error, "Automatic X11 GL context failed; retrying once with GLX");
    // The retry explicitly selects glx, so its own failure cannot retry again.
    let launch_error = recovery_command(executable.as_os_str(), std::env::args_os().skip(1)).exec();
    Err(error.context(format!("Could not start the GLX retry: {launch_error}")))
}

fn setting(
    mode: &str,
    wayland: bool,
    gtk: (u32, u32),
) -> Result<Option<(&'static str, &'static str)>> {
    match mode {
        "auto" => Ok(None),
        "glx" if wayland => bail!("GLX requires X11; use TALIS_GL_BACKEND=auto on Wayland"),
        "glx" if gtk >= (4, 16) => Ok(Some(("GDK_DISABLE", "egl"))),
        "glx" => Ok(Some(("GDK_DEBUG", "gl-glx"))),
        _ => bail!("TALIS_GL_BACKEND must be auto or glx"),
    }
}

/// # Safety
/// Call only during single-threaded process startup, before GTK or workers start.
pub unsafe fn configure(experimental_wayland: bool) -> Result<()> {
    let mode = std::env::var("TALIS_GL_BACKEND").unwrap_or_else(|_| "auto".into());
    let wayland = experimental_wayland
        || crate::desktop::Session::detect() == crate::desktop::Session::Wayland
        || (crate::desktop::Session::detect() != crate::desktop::Session::X11
            && std::env::var_os("WAYLAND_DISPLAY").is_some());
    if let Some((key, flag)) =
        setting(&mode, wayland, (gtk::major_version(), gtk::minor_version()))?
    {
        let existing = std::env::var(key).unwrap_or_default();
        if !existing.split(',').any(|value| value == flag) {
            let value = if existing.is_empty() {
                flag.into()
            } else {
                format!("{existing},{flag}")
            };
            // SAFETY: main calls this before GTK initialization, logging setup
            // or any application threads. No other thread reads the environment.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
