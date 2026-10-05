//! Opt-in frame-section profiler.
//!
//! Set `HALLEY_PROFILE=1` (or any non-empty value) to log per-frame section
//! times once per ~2 seconds per output, plus the first sample immediately.
//! Zero cost when disabled: a single atomic bool checked once per frame.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_REPORT: Mutex<Option<Instant>> = Mutex::new(None);

pub struct FrameTimer {
    enabled: bool,
    start: Instant,
    last_mark: Instant,
    sections: Vec<(&'static str, Duration)>,
    total_frames: u64,
    total_frames_at_mark: u64,
}

const REPORT_INTERVAL: Duration = Duration::from_secs(2);
const MAX_SECTIONS: usize = 24;

pub fn enabled() -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        if std::env::var_os("HALLEY_PROFILE").is_some_and(|value| !value.is_empty()) {
            ENABLED.store(true, Ordering::Relaxed);
        } else {
            return false;
        }
    }
    true
}

pub fn begin() -> FrameTimer {
    let start = Instant::now();
    FrameTimer {
        enabled: enabled(),
        start,
        last_mark: start,
        sections: Vec::with_capacity(MAX_SECTIONS),
        total_frames: 0,
        total_frames_at_mark: 0,
    }
}

impl FrameTimer {
    /// Marks a section boundary: everything since the previous mark (or the
    /// frame start) is attributed to `label`.
    pub fn mark(&mut self, label: &'static str) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        self.sections.push((label, now - self.last_mark));
        self.last_mark = now;
    }

    /// Closes the frame; logs the averaged section breakdown.
    pub fn finish(&mut self, output: &str) {
        if !self.enabled {
            return;
        }
        self.total_frames += 1;
        let now = Instant::now();
        self.sections.push(("rest", now - self.last_mark));
        let total = now - self.start;
        let should_report = {
            let Ok(mut last) = LAST_REPORT.lock() else {
                return;
            };
            let due = last.map_or(true, |last| now.duration_since(last) >= REPORT_INTERVAL);
            if due {
                *last = Some(now);
            }
            due
        };
        if !should_report {
            return;
        }
        let frames = self.total_frames - self.total_frames_at_mark;
        self.total_frames_at_mark = self.total_frames;
        let mut line = String::with_capacity(256);
        for (label, duration) in &self.sections {
            let per_frame_us = duration.as_secs_f64() / frames.max(1) as f64 * 1e6;
            let _ = write!(line, " {label}={per_frame_us:.0}us");
        }
        drain_scoped_sections(&mut line);
        drain_motion(&mut line);
        let frame_us = total.as_secs_f64() / (self.total_frames).max(1) as f64 * 1e6;
        eventline::debug!("halley-profile output={output} frames={frames} avg_frame={frame_us:.0}us{line}");
    }
}

/// Times one labeled section when profiling is enabled. Closures return
/// their value unchanged (Result included) so `?` stays at the call site.
pub fn scoped<T>(label: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    record_section(label, start.elapsed());
    result
}

static SECTIONS: Mutex<Vec<(&'static str, Duration, u64)>> = Mutex::new(Vec::new());
static MOTION: Mutex<(Duration, u64)> = Mutex::new((Duration::ZERO, 0));

/// Records one pointer-motion handling pass (input-thread cost, not frames).
pub fn record_motion(duration: Duration) {
    if let Ok(mut motion) = MOTION.lock() {
        motion.0 += duration;
        motion.1 += 1;
    }
}

fn drain_motion(line: &mut String) {
    let Ok(mut motion) = MOTION.lock() else {
        return;
    };
    let (total, count) = *motion;
    *motion = (Duration::ZERO, 0);
    if count == 0 {
        return;
    }
    let avg_us = total.as_secs_f64() / count as f64 * 1e6;
    let _ = write!(line, " input_motion:{avg_us:.0}us x{count}");
}

fn record_section(label: &'static str, duration: Duration) {
    if let Ok(mut sections) = SECTIONS.lock() {
        if let Some(entry) = sections.iter_mut().find(|(name, _, _)| *name == label) {
            entry.1 += duration;
            entry.2 += 1;
            return;
        }
        sections.push((label, duration, 1));
    }
}

/// Merges accumulated `scoped` sections into the next per-frame report and
/// clears them. Called by FrameTimer::finish so the log shows both frame
/// sections and sub-sections with per-call averages.
fn drain_scoped_sections(line: &mut String) {
    let Ok(mut sections) = SECTIONS.lock() else {
        return;
    };
    for (label, total, count) in sections.drain(..) {
        let avg_us = total.as_secs_f64() / count.max(1) as f64 * 1e6;
        let _ = write!(line, " {label}:{avg_us:.0}us x{count}");
    }
}
