//! Detached process launching for session actions.

use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use super::environment::LaunchEnvironment;

/// Spawns a user-provided command line detached from the compositor, with
/// `WAYLAND_DISPLAY` set to Halley's socket rather than the ambient host.
///
/// The shell is intentional: loose keybinds may contain arguments, quoting,
/// pipelines, or substitutions. The config is user-authored and has the
/// same authority as starting a command from their own shell.
pub(super) fn spawn_detached(
    command_line: &str,
    wayland_display: &OsStr,
    x11_display: Option<&OsStr>,
    cursor_size: u8,
    environment: &LaunchEnvironment,
) {
    spawn_detached_with_env(
        command_line,
        wayland_display,
        x11_display,
        cursor_size,
        environment,
        &[],
    );
}

pub(super) fn spawn_detached_with_env(
    command_line: &str,
    wayland_display: &OsStr,
    x11_display: Option<&OsStr>,
    cursor_size: u8,
    environment: &LaunchEnvironment,
    extra_environment: &[(&str, &str)],
) {
    let mut process = detached_process_with_env(
        command_line,
        wayland_display,
        x11_display,
        cursor_size,
        environment,
        extra_environment,
    );

    match process.spawn() {
        Ok(mut child) => {
            // The immediate child exits after its own fork. The command's
            // shell is already reparented and will be reaped independently.
            match child.wait() {
                Ok(_) => eventline::debug!(
                    "spawn: launched {command_line:?} (WAYLAND_DISPLAY={wayland_display:?}, DISPLAY={x11_display:?})"
                ),
                Err(err) => {
                    eventline::warn!("spawn: failed to reap intermediate process: {err}")
                }
            }
        }
        Err(err) => eventline::error!("spawn: failed to launch {command_line:?}: {err}"),
    }
}

#[cfg(test)]
fn detached_process(
    command_line: &str,
    wayland_display: &OsStr,
    x11_display: Option<&OsStr>,
    cursor_size: u8,
    environment: &LaunchEnvironment,
) -> Command {
    detached_process_with_env(
        command_line,
        wayland_display,
        x11_display,
        cursor_size,
        environment,
        &[],
    )
}

fn detached_process_with_env(
    command_line: &str,
    wayland_display: &OsStr,
    x11_display: Option<&OsStr>,
    cursor_size: u8,
    environment: &LaunchEnvironment,
    extra_environment: &[(&str, &str)],
) -> Command {
    let mut process = configured_process(
        command_line,
        wayland_display,
        x11_display,
        cursor_size,
        environment,
        extra_environment,
    );

    // Safety: only async-signal-safe calls between fork and exec - a raw
    // fork() plus an immediate _exit() (never std::process::exit, which
    // isn't safe to run again after a raw fork - it may re-run Rust's
    // normal shutdown machinery a second time).
    unsafe {
        process.pre_exec(|| match libc::fork() {
            -1 => Err(io::Error::last_os_error()),
            0 => Ok(()),
            _ => libc::_exit(0),
        });
    }

    process
}

fn configured_process(
    command_line: &str,
    wayland_display: &OsStr,
    x11_display: Option<&OsStr>,
    cursor_size: u8,
    environment: &LaunchEnvironment,
    extra_environment: &[(&str, &str)],
) -> Command {
    let mut process = Command::new("sh");
    environment.apply_to(&mut process);
    process
        .arg("-c")
        .arg(command_line)
        .env("WAYLAND_DISPLAY", wayland_display)
        .env("XCURSOR_SIZE", cursor_size.to_string())
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(display) = x11_display {
        process.env("DISPLAY", display);
    }
    process.envs(extra_environment.iter().copied());
    process
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_use_a_shell_and_halley_display() {
        let process = detached_process(
            "grim -g \"$(slurp)\" ~/shot.png",
            OsStr::new("wayland-9"),
            Some(OsStr::new(":12")),
            32,
            &LaunchEnvironment::default(),
        );
        assert_eq!(process.get_program(), "sh");
        assert_eq!(
            process.get_args().collect::<Vec<_>>(),
            ["-c", "grim -g \"$(slurp)\" ~/shot.png"]
        );
        assert!(process.get_envs().any(|(name, value)| {
            name == "WAYLAND_DISPLAY" && value == Some(OsStr::new("wayland-9"))
        }));
        assert!(
            process
                .get_envs()
                .any(|(name, value)| name == "DISPLAY" && value == Some(OsStr::new(":12")))
        );
        assert!(
            process
                .get_envs()
                .any(|(name, value)| { name == "XCURSOR_SIZE" && value == Some(OsStr::new("32")) })
        );
    }

    #[test]
    fn unavailable_xwayland_removes_ambient_display() {
        let process = detached_process(
            "foot",
            OsStr::new("wayland-2"),
            None,
            24,
            &LaunchEnvironment::default(),
        );
        assert!(
            process
                .get_envs()
                .any(|(name, value)| name == "DISPLAY" && value.is_none())
        );
    }

    #[test]
    fn configured_environment_is_inherited_but_session_values_win() {
        let environment = LaunchEnvironment::new(&std::collections::BTreeMap::from([
            ("CUSTOM".to_string(), "value".to_string()),
            ("DISPLAY".to_string(), ":99".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wrong".to_string()),
            ("XCURSOR_SIZE".to_string(), "99".to_string()),
        ]));
        let process = detached_process(
            "true",
            OsStr::new("wayland-4"),
            Some(OsStr::new(":8")),
            24,
            &environment,
        );
        let env = process
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            env.get(OsStr::new("CUSTOM")),
            Some(&Some(OsStr::new("value")))
        );
        assert_eq!(
            env.get(OsStr::new("WAYLAND_DISPLAY")),
            Some(&Some(OsStr::new("wayland-4")))
        );
        assert_eq!(
            env.get(OsStr::new("DISPLAY")),
            Some(&Some(OsStr::new(":8")))
        );
        assert_eq!(
            env.get(OsStr::new("XCURSOR_SIZE")),
            Some(&Some(OsStr::new("24")))
        );
    }
}
