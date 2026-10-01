use std::ffi::{OsStr, OsString};

use super::environment::LaunchEnvironment;

enum OnceState {
    Unarmed,
    Pending(Vec<String>),
    Finished,
}

/// Launches configured startup commands independently of the compositor.
///
/// `autostart` is a convenience launcher, not a service manager. Long-lived
/// session services should use the user service manager when they need
/// restart, ordering, or session-lifetime semantics.
pub(super) struct Autostart {
    enabled: bool,
    once: OnceState,
    wayland_display: Option<OsString>,
}

impl Autostart {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            once: OnceState::Unarmed,
            wayland_display: None,
        }
    }

    #[cfg_attr(not(feature = "winit"), allow(dead_code))]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            once: OnceState::Finished,
            wayland_display: None,
        }
    }

    pub fn arm_once(&mut self, wayland_display: &OsStr, commands: Vec<String>) {
        if !self.enabled || !matches!(self.once, OnceState::Unarmed) {
            return;
        }
        self.wayland_display = Some(wayland_display.to_os_string());
        self.once = OnceState::Pending(commands);
    }

    pub fn run_once(
        &mut self,
        x11_display: Option<&OsStr>,
        cursor_size: u8,
        environment: &LaunchEnvironment,
    ) {
        let OnceState::Pending(commands) = std::mem::replace(&mut self.once, OnceState::Finished)
        else {
            return;
        };
        self.run_commands(&commands, x11_display, cursor_size, environment);
    }

    pub fn run_reload(
        &mut self,
        commands: &[String],
        x11_display: Option<&OsStr>,
        cursor_size: u8,
        environment: &LaunchEnvironment,
    ) {
        if self.enabled {
            self.run_commands(commands, x11_display, cursor_size, environment);
        }
    }

    fn run_commands(
        &mut self,
        commands: &[String],
        x11_display: Option<&OsStr>,
        cursor_size: u8,
        environment: &LaunchEnvironment,
    ) {
        let Some(wayland_display) = self.wayland_display.as_deref() else {
            return;
        };
        for command in commands {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            eventline::debug!(
                "autostart: launching {command:?} (WAYLAND_DISPLAY={wayland_display:?}, DISPLAY={x11_display:?})"
            );
            super::spawn::spawn_detached(
                command,
                wayland_display,
                x11_display,
                cursor_size,
                environment,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn context() -> (LaunchEnvironment, OsString) {
        (LaunchEnvironment::default(), OsString::from("wayland-9"))
    }

    #[test]
    fn once_can_only_be_armed_and_run_once() {
        let (environment, display) = context();
        let mut autostart = Autostart::enabled();
        autostart.arm_once(&display, Vec::new());
        autostart.arm_once(&display, vec!["sleep 30".to_string()]);
        autostart.run_once(None, 24, &environment);
        autostart.run_once(None, 24, &environment);

        assert!(matches!(autostart.once, OnceState::Finished));
    }

    #[test]
    fn nested_policy_suppresses_once_and_reload_commands() {
        let (environment, display) = context();
        let mut autostart = Autostart::disabled();
        autostart.arm_once(&display, vec!["sleep 30".to_string()]);
        autostart.run_once(None, 24, &environment);
        autostart.run_reload(&["sleep 30".to_string()], None, 24, &environment);

        assert!(matches!(autostart.once, OnceState::Finished));
    }

    #[test]
    fn reload_does_not_consume_once() {
        let (environment, display) = context();
        let mut autostart = Autostart::enabled();
        autostart.arm_once(&display, Vec::new());
        autostart.run_reload(&["true".to_string()], None, 24, &environment);

        assert!(matches!(autostart.once, OnceState::Pending(_)));
    }

    #[test]
    fn dropping_autostart_does_not_stop_launched_commands() {
        let (environment, display) = context();
        let marker = std::env::temp_dir().join(format!(
            "halley-autostart-detached-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let command = format!(
            "printf started > '{}'; sleep 0.1; printf finished > '{}'",
            marker.display(),
            marker.display()
        );
        let mut autostart = Autostart::enabled();
        autostart.arm_once(&display, vec![command]);
        autostart.run_once(None, 24, &environment);

        assert!(wait_for_marker(&marker, "started"));
        drop(autostart);
        assert!(wait_for_marker(&marker, "finished"));
        let _ = std::fs::remove_file(marker);
    }

    fn wait_for_marker(path: &std::path::Path, expected: &str) -> bool {
        for _ in 0..100 {
            if std::fs::read_to_string(path).is_ok_and(|value| value == expected) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}
