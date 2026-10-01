use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use eventline::{LogLevel, LogPolicy, RunHeader};
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{Layer, filter::Targets};

const LOG_FILE_NAME: &str = "halley.log";

/// Initializes console logging first so runtime-directory or file failures
/// remain visible without preventing the compositor from starting.
pub fn init() {
    eventline::init_sync();
    eventline::enable_console_output(true);
    eventline::set_console_level(LogLevel::Info);
    eventline::enable_console_color(true);
    eventline::enable_console_duration(false);

    let runtime_dir = match halley_ipc::ensure_runtime_dir() {
        Ok(path) => path,
        Err(err) => {
            eventline::warn!("file logging unavailable: {err}");
            return;
        }
    };
    let log_path = runtime_dir.join(LOG_FILE_NAME);
    if let Err(err) = eventline::enable_file_output_rotating(
        &log_path,
        LogPolicy::default(),
        Some(RunHeader::new("halley")),
    ) {
        eventline::warn!(
            "failed to initialize file logging at {}: {err}",
            log_path.display()
        );
        return;
    }
    eventline::set_file_level(LogLevel::Debug);
    eventline::info!("file logging enabled at {}", log_path.display());
    init_smithay_bridge();
}

/// Forwards Smithay's `tracing` output into `eventline`, which otherwise never
/// sees it - Smithay is the only part of the process that logs through
/// `tracing`, so without a subscriber every DRM message it emits is discarded.
/// That blindness cost real debugging time on a DPMS wake stall: the modeset
/// path narrates what it is doing, and none of it reached the log.
///
/// Deliberately narrow, because this is diagnostic weight on a hot path.
/// Everything under `smithay` is `warn` and above; only the DRM backend is
/// opened up to `info`, which is where connector/mode changes are announced.
/// `debug` for that target is *not* wanted - the atomic surface debug-logs the
/// entire commit request, several screenfuls per modeset.
fn init_smithay_bridge() {
    let filter = Targets::new()
        .with_target("smithay", Level::WARN)
        .with_target("smithay::backend::drm", Level::INFO);

    if tracing_subscriber::registry()
        .with(EventlineLayer.with_filter(filter))
        .try_init()
        .is_err()
    {
        eventline::warn!("smithay log bridge unavailable: a tracing subscriber is already set");
    }
}

struct EventlineLayer;

static UNKNOWN_X11_ASSOCIATIONS: AtomicU64 = AtomicU64::new(0);

impl<S: tracing::Subscriber> Layer<S> for EventlineLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let message = fields.finish();
        if message.is_empty() {
            return;
        }
        let target = event.metadata().target();
        if repeated_smithay_warning(target, &message) {
            return;
        }
        match *event.metadata().level() {
            Level::ERROR => eventline::error!("{target}: {message}"),
            Level::WARN if known_benign_smithay_warning(target, &message) => {
                eventline::debug!("{target}: {message}")
            }
            Level::WARN => eventline::warn!("{target}: {message}"),
            // `info` and below land in the file only. The console is pinned to
            // `Info`, and a per-modeset connector list has no business there.
            _ => eventline::debug!("{target}: {message}"),
        }
    }
}

fn known_benign_smithay_warning(target: &str, message: &str) -> bool {
    // Smithay deliberately continues after this legacy drmSetMaster call:
    // current kernels authorize the modesetting fd without it. A genuine
    // permission/output failure still surfaces from DrmDevice/output setup.
    (target == "smithay::backend::drm::device::fd"
        && message == "Unable to become drm master, assuming unprivileged mode")
        // CreateNotify is followed by an asynchronous GetGeometry request in
        // Smithay's XWM. Some toolkit helper windows disappear before the
        // reply arrives, producing BadDrawable even though the compositor has
        // no stale state to recover. Keep every other XWM failure visible.
        || (target == "smithay::xwayland::xwm"
            && message.starts_with("Failed to handle X11 event")
            && message.contains("error_kind: Drawable")
            && message.contains("request_name: Some(\"GetGeometry\")"))
}

/// Keep Smithay's structured context. Its XWM failure event puts the useful
/// `id` and `err` values in fields rather than in `message`.
#[derive(Default)]
struct EventFields {
    message: String,
    fields: Vec<(String, String)>,
}

impl EventFields {
    fn finish(self) -> String {
        let mut rendered = self.message;
        for (name, value) in self.fields {
            if !rendered.is_empty() {
                rendered.push(' ');
            }
            let _ = write!(rendered, "{name}={value}");
        }
        rendered
    }

    fn record_value(&mut self, field: &tracing::field::Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            name if name.starts_with("log.") => {}
            name => self.fields.push((name.to_string(), value)),
        }
    }
}

impl tracing::field::Visit for EventFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_value(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field, format!("{value:?}"));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field, value.to_string());
    }
}

fn repeated_smithay_warning(target: &str, message: &str) -> bool {
    if target != "smithay::wayland::xwayland_shell"
        || !message.starts_with("Unknown X11 window associated to wl_surface in commit hook")
    {
        return false;
    }

    let count = UNKNOWN_X11_ASSOCIATIONS.fetch_add(1, Ordering::Relaxed) + 1;
    if count == 1 {
        return false;
    }
    if count.is_power_of_two() {
        eventline::warn!(
            "{target}: suppressed repeated unknown X11 surface associations count={count}"
        );
    }
    true
}

pub fn flush() {
    if let Err(err) = eventline::flush() {
        eventline::error!("failed to flush logging: {err}");
        let _ = eventline::flush();
    }
}

#[cfg(test)]
mod tests {
    use super::{EventFields, known_benign_smithay_warning};

    #[test]
    fn only_the_known_new_kernel_drm_fallback_is_downgraded() {
        assert!(known_benign_smithay_warning(
            "smithay::backend::drm::device::fd",
            "Unable to become drm master, assuming unprivileged mode"
        ));
        assert!(!known_benign_smithay_warning(
            "smithay::backend::drm::device::fd",
            "Failed to drop drm master state"
        ));
        assert!(!known_benign_smithay_warning(
            "smithay::backend::drm",
            "Unable to become drm master, assuming unprivileged mode"
        ));
    }

    #[test]
    fn only_destroyed_drawable_geometry_races_are_downgraded() {
        let race = concat!(
            "Failed to handle X11 event id=0 err=X11Error(X11Error { ",
            "error_kind: Drawable, error_code: 9, request_name: Some(\"GetGeometry\") })"
        );
        assert!(known_benign_smithay_warning("smithay::xwayland::xwm", race));
        assert!(!known_benign_smithay_warning(
            "smithay::xwayland::xwm",
            "Failed to handle X11 event id=0 err=BadWindow"
        ));
        assert!(!known_benign_smithay_warning("some::other::target", race));
    }

    #[test]
    fn structured_fields_are_retained_after_the_message() {
        let fields = EventFields {
            message: "Failed to handle X11 event".to_string(),
            fields: vec![
                ("id".to_string(), "3".to_string()),
                ("err".to_string(), "BadWindow(42)".to_string()),
            ],
        };

        assert_eq!(
            fields.finish(),
            "Failed to handle X11 event id=3 err=BadWindow(42)"
        );
    }
}
