//! Backend selection before GTK initialization; one context-failure retry.
mod diagnostics;
use anyhow::{Context, Result, bail};
pub(crate) use diagnostics::software_renderer;
use std::{ffi::OsStr, os::unix::process::CommandExt, process::Command};

/// Only failure to create the scene's actual GL context may select a fallback.
/// Shader, texture, asset and later rendering errors must retain their identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayBackend {
    X11,
    Wayland,
    Other,
}

#[derive(Debug)]
pub(crate) struct ContextInitializationError {
    reason: String,
    backend: DisplayBackend,
}
impl ContextInitializationError {
    pub(crate) fn new(reason: String, backend: DisplayBackend) -> Self {
        Self { reason, backend }
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
        .is_some_and(|error| error.backend == DisplayBackend::X11)
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

fn wayland_eligible(
    error: &anyhow::Error,
    mode: &str,
    debug: &str,
    disabled: &str,
    retried: bool,
    gtk: (u32, u32),
) -> bool {
    error
        .downcast_ref::<ContextInitializationError>()
        .is_some_and(|error| error.backend == DisplayBackend::Wayland)
        && mode == "auto"
        && !retried
        && gtk >= (4, 16)
        && !has_flag(
            debug,
            &[
                "all",
                "gl-disable",
                "gl-glx",
                "gl-egl",
                "gl-gles",
                "gl-legacy",
                "gl-prefer-gl",
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

fn wayland_recovery_command(
    executable: &OsStr,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    disabled: &str,
) -> Command {
    let mut command = Command::new(executable);
    let disabled = if disabled.is_empty() {
        "gles-api".to_owned()
    } else {
        format!("{disabled},gles-api")
    };
    command
        .args(arguments)
        .env("GDK_DISABLE", disabled)
        .env("TALIS_WAYLAND_GL_RETRY", "1");
    command
}

/// Called only after the failed application has shut down and released its
/// desktop lease. exec preserves arguments, cwd and PID without a second owner.
/// Environment changes belong to the new process, never to GTK's worker threads.
pub fn recover(error: anyhow::Error) -> Result<()> {
    recover_with_version(error, (gtk::major_version(), gtk::minor_version()))
}

fn recover_with_version(error: anyhow::Error, gtk: (u32, u32)) -> Result<()> {
    let mode = std::env::var("TALIS_GL_BACKEND").unwrap_or_else(|_| "auto".into());
    let wayland = crate::desktop::Session::detect() == crate::desktop::Session::Wayland;
    let debug = std::env::var("GDK_DEBUG").unwrap_or_default();
    let disabled = std::env::var("GDK_DISABLE").unwrap_or_default();
    if eligible(&error, &mode, wayland, &debug, &disabled) {
        let executable = std::env::current_exe().context("Locate Talis for GLX recovery")?;
        tracing::warn!(%error, "Automatic X11 GL context failed; retrying once with GLX");
        // The retry explicitly selects glx, so its own failure cannot retry again.
        let launch_error =
            recovery_command(executable.as_os_str(), std::env::args_os().skip(1)).exec();
        return Err(error.context(format!("Could not start the GLX retry: {launch_error}")));
    }
    let retried_wayland = std::env::var_os("TALIS_WAYLAND_GL_RETRY").is_some();
    if wayland_eligible(&error, &mode, &debug, &disabled, retried_wayland, gtk) {
        let executable = std::env::current_exe().context("Locate Talis for Wayland recovery")?;
        tracing::warn!(%error, "Automatic Wayland GL context failed; retrying once with desktop OpenGL");
        // A new process must select GTK's API before any context is created.
        let launch_error = wayland_recovery_command(
            executable.as_os_str(),
            std::env::args_os().skip(1),
            &disabled,
        )
        .exec();
        return Err(error.context(format!(
            "Could not start the Wayland GL retry: {launch_error}"
        )));
    }
    if error.is::<ContextInitializationError>() && mode == "glx" {
        Err(error.context("GLX context creation failed; no further backend retries"))
    } else if error
        .downcast_ref::<ContextInitializationError>()
        .is_some_and(|failure| failure.backend == DisplayBackend::Wayland && retried_wayland)
    {
        Err(error.context("Wayland desktop OpenGL retry failed; no further backend retries"))
    } else {
        Err(error)
    }
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

fn presentation_renderer(gtk: (u32, u32), overridden: bool) -> Option<&'static str> {
    if overridden {
        return None;
    }
    // GTK 4.14 introduced the unified GL renderer as `ngl`; 4.18 renamed
    // it to `gl`. Earlier supported GTK versions have the older GL renderer.
    Some(if ((4, 14)..(4, 18)).contains(&gtk) {
        "ngl"
    } else {
        "gl"
    })
}

/// Prefer OpenGL presentation for the GL scene and GTK windows on every desktop.
/// An explicit GSK_RENDERER, including an empty value, belongs to the caller.
///
/// # Safety
/// Call only during single-threaded process startup, before GTK or workers start.
pub unsafe fn configure_presentation() {
    if let Some(renderer) = presentation_renderer(
        (gtk::major_version(), gtk::minor_version()),
        std::env::var_os("GSK_RENDERER").is_some(),
    ) {
        // SAFETY: the caller guarantees no GTK or worker threads have started.
        unsafe {
            std::env::set_var("GSK_RENDERER", renderer);
        }
    }
}

/// # Safety
/// Call only during single-threaded process startup, before GTK or workers start.
pub unsafe fn configure(experimental_wayland: bool) -> Result<()> {
    let mode = std::env::var("TALIS_GL_BACKEND").unwrap_or_else(|_| "auto".into());
    let gtk = (gtk::major_version(), gtk::minor_version());
    let wayland = experimental_wayland
        || crate::desktop::Session::detect() == crate::desktop::Session::Wayland
        || (crate::desktop::Session::detect() != crate::desktop::Session::X11
            && std::env::var_os("WAYLAND_DISPLAY").is_some());
    let backend_setting = setting(&mode, wayland, gtk)?;
    // SAFETY: configure has the same single-threaded startup precondition.
    unsafe {
        configure_presentation();
    }
    if let Some((key, flag)) = backend_setting {
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
