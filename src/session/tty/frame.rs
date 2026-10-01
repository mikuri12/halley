use std::time::Duration;

use calloop::RegistrationToken;

use crate::frame_clock::FrameClock;

/// The redraw work an output owes and the kernel/timer event that currently
/// gates it. Keeping these transitions together prevents input, DRM, and
/// timer handlers from independently interpreting the same state.
#[derive(Debug, Default)]
enum RedrawState {
    #[default]
    Idle,
    Suspended,
    Queued,
    WaitingForVBlank {
        redraw_needed: bool,
    },
    WaitingForEstimatedVBlank(RegistrationToken),
    WaitingForEstimatedVBlankAndQueued(RegistrationToken),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum EstimatedVblankTimer {
    AlreadyArmed,
    ArmAfter(Duration),
}

/// Work that becomes safe when the kernel reports a page flip.
///
/// A queued redraw takes precedence over releasing another client frame
/// callback: the buffers for that redraw have not been latched yet. Once a
/// frame is submitted, its callbacks are released immediately instead.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum VblankAction {
    Ignore,
    Redraw,
    SendCallbacks,
}

/// Result of filtering a kernel VBlank through the output's fixed-refresh
/// cadence. Some drivers occasionally report a second event far too early;
/// delaying its completion avoids turning that glitch into a compositor-side
/// busy loop.
pub(super) struct VblankThrottleResult {
    pub delay: Option<Duration>,
    pub cancel_timer: Option<RegistrationToken>,
    pub warn: bool,
}

pub(super) struct OutputFrameState {
    clock: FrameClock,
    redraw: RedrawState,
    last_camera_sample: Duration,
    unfinished_animations: bool,
    frame_callback_sequence: u32,
    last_vblank_timestamp: Option<Duration>,
    vblank_throttle_timer: Option<RegistrationToken>,
    vblank_throttle_warned: bool,
}

impl OutputFrameState {
    pub fn new(refresh_interval: Duration) -> Self {
        Self {
            clock: FrameClock::new(Some(refresh_interval)),
            redraw: RedrawState::default(),
            last_camera_sample: crate::frame_clock::monotonic_now(),
            unfinished_animations: false,
            frame_callback_sequence: 0,
            last_vblank_timestamp: None,
            vblank_throttle_timer: None,
            vblank_throttle_warned: false,
        }
    }

    pub fn advance_frame_callback_sequence(&mut self) {
        self.frame_callback_sequence = self.frame_callback_sequence.wrapping_add(1);
    }

    pub fn frame_callback_sequence(&self) -> u32 {
        self.frame_callback_sequence
    }

    pub fn throttle_vblank(
        &mut self,
        timestamp: Option<Duration>,
        refresh_interval: Duration,
    ) -> VblankThrottleResult {
        let cancel_timer = self.vblank_throttle_timer.take();
        let Some(timestamp) = timestamp else {
            return VblankThrottleResult {
                delay: None,
                cancel_timer,
                warn: false,
            };
        };

        if let Some(last) = self.last_vblank_timestamp {
            let passed = timestamp.saturating_sub(last);
            if passed < refresh_interval / 2 {
                let warn = !self.vblank_throttle_warned;
                self.vblank_throttle_warned = true;
                return VblankThrottleResult {
                    delay: Some(refresh_interval.saturating_sub(passed)),
                    cancel_timer,
                    warn,
                };
            }
        }

        self.last_vblank_timestamp = Some(timestamp);
        VblankThrottleResult {
            delay: None,
            cancel_timer,
            warn: false,
        }
    }

    pub fn vblank_throttle_timer_armed(&mut self, token: RegistrationToken) {
        self.vblank_throttle_timer = Some(token);
    }

    pub fn vblank_throttle_timer_fired(&mut self) {
        self.vblank_throttle_timer = None;
    }

    pub fn queue_redraw(&mut self) {
        self.redraw = match std::mem::take(&mut self.redraw) {
            RedrawState::Suspended => RedrawState::Suspended,
            RedrawState::Idle => RedrawState::Queued,
            RedrawState::WaitingForEstimatedVBlank(token) => {
                RedrawState::WaitingForEstimatedVBlankAndQueued(token)
            }
            value @ (RedrawState::Queued | RedrawState::WaitingForEstimatedVBlankAndQueued(_)) => {
                value
            }
            RedrawState::WaitingForVBlank { .. } => RedrawState::WaitingForVBlank {
                redraw_needed: true,
            },
        };
    }

    pub fn is_redraw_queued(&self) -> bool {
        matches!(
            self.redraw,
            RedrawState::Queued | RedrawState::WaitingForEstimatedVBlankAndQueued(_)
        )
    }

    pub fn replace_clock(&mut self, refresh_interval: Duration) {
        self.clock = FrameClock::new(Some(refresh_interval));
        self.last_camera_sample = crate::frame_clock::monotonic_now();
        self.unfinished_animations = false;
    }

    pub fn set_vrr(&mut self, vrr: bool) {
        self.clock.set_vrr(vrr);
    }

    pub fn next_frame_sample(&mut self, now: Duration) -> (Duration, Duration) {
        let target = self.clock.next_presentation_time(now);
        let dt = target.saturating_sub(self.last_camera_sample);
        self.last_camera_sample = target;
        (target, dt)
    }

    pub fn on_vblank(&mut self, presented: Option<Duration>) -> (VblankAction, Option<String>) {
        self.clock.presented(presented);
        let (redraw_needed, unexpected_state) = match std::mem::take(&mut self.redraw) {
            RedrawState::Suspended => {
                self.redraw = RedrawState::Suspended;
                return (VblankAction::Ignore, None);
            }
            RedrawState::WaitingForVBlank { redraw_needed } => {
                (redraw_needed || self.unfinished_animations, None)
            }
            other => (true, Some(format!("{other:?}"))),
        };
        self.redraw = if redraw_needed {
            RedrawState::Queued
        } else {
            RedrawState::Idle
        };

        let action = if redraw_needed {
            VblankAction::Redraw
        } else {
            VblankAction::SendCallbacks
        };
        (action, unexpected_state)
    }

    /// Records a real page flip and returns an estimated-VBlank timer that
    /// became obsolete, if one was armed.
    pub fn frame_submitted(&mut self, animating: bool) -> Option<RegistrationToken> {
        let timer = match std::mem::take(&mut self.redraw) {
            RedrawState::WaitingForEstimatedVBlank(token)
            | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => Some(token),
            _ => None,
        };
        self.unfinished_animations = animating;
        self.redraw = RedrawState::WaitingForVBlank {
            redraw_needed: animating,
        };
        timer
    }

    /// Records a render with no page flip. A previously armed fallback is
    /// reused; otherwise the caller receives the delay for one new timer.
    pub fn frame_skipped(&mut self, animating: bool, now: Duration) -> EstimatedVblankTimer {
        self.unfinished_animations = animating;
        match std::mem::take(&mut self.redraw) {
            RedrawState::Queued => {}
            RedrawState::WaitingForEstimatedVBlank(token)
            | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => {
                self.redraw = RedrawState::WaitingForEstimatedVBlank(token);
                return EstimatedVblankTimer::AlreadyArmed;
            }
            RedrawState::Suspended | RedrawState::Idle | RedrawState::WaitingForVBlank { .. } => {
                unreachable!("frame_skipped called from an unexpected redraw state")
            }
        }

        let due = self.clock.next_presentation_time(now);
        EstimatedVblankTimer::ArmAfter(due.saturating_sub(now).max(Duration::from_millis(1)))
    }

    pub fn timer_armed(&mut self, token: RegistrationToken) {
        self.redraw = RedrawState::WaitingForEstimatedVBlank(token);
    }

    pub fn estimated_vblank_fired(&mut self) -> Result<bool, String> {
        match std::mem::take(&mut self.redraw) {
            RedrawState::Suspended => {
                self.redraw = RedrawState::Suspended;
                Ok(false)
            }
            RedrawState::WaitingForEstimatedVBlank(_) if self.unfinished_animations => {
                self.redraw = RedrawState::Queued;
                Ok(false)
            }
            RedrawState::WaitingForEstimatedVBlank(_) => Ok(true),
            RedrawState::WaitingForEstimatedVBlankAndQueued(_) => {
                self.redraw = RedrawState::Queued;
                Ok(false)
            }
            other => Err(format!("{other:?}")),
        }
    }

    pub fn suspend(&mut self, now: Duration) -> Vec<RegistrationToken> {
        let mut timers = Vec::with_capacity(2);
        if let Some(token) = self.vblank_throttle_timer.take() {
            timers.push(token);
        }
        if let Some(token) = match std::mem::take(&mut self.redraw) {
            RedrawState::WaitingForEstimatedVBlank(token)
            | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => Some(token),
            _ => None,
        } {
            timers.push(token);
        }
        self.clock.reset();
        self.last_vblank_timestamp = None;
        self.last_camera_sample = now;
        self.unfinished_animations = false;
        self.redraw = RedrawState::Suspended;
        timers
    }

    pub fn resume(&mut self, now: Duration) {
        debug_assert!(matches!(self.redraw, RedrawState::Suspended));
        self.clock.reset();
        self.last_vblank_timestamp = None;
        self.last_camera_sample = now;
        self.unfinished_animations = false;
        self.redraw = RedrawState::Queued;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> OutputFrameState {
        OutputFrameState::new(Duration::from_millis(10))
    }

    fn registration_token() -> RegistrationToken {
        let event_loop = calloop::EventLoop::<()>::try_new().expect("test event loop");
        event_loop
            .handle()
            .insert_source(calloop::timer::Timer::immediate(), |_, _, _| {
                calloop::timer::TimeoutAction::Drop
            })
            .expect("test timer")
    }

    #[test]
    fn queued_requests_are_idempotent() {
        let mut state = state();
        state.queue_redraw();
        state.queue_redraw();

        assert!(state.is_redraw_queued());
    }

    #[test]
    fn request_while_waiting_for_vblank_is_not_lost() {
        let mut state = state();
        state.queue_redraw();
        assert_eq!(state.frame_submitted(false), None);

        state.queue_redraw();
        let (action, unexpected) = state.on_vblank(None);

        assert_eq!(unexpected, None);
        assert_eq!(action, VblankAction::Redraw);
        assert!(state.is_redraw_queued());
    }

    #[test]
    fn unfinished_animation_requests_the_next_real_frame() {
        let mut state = state();
        state.queue_redraw();
        state.frame_submitted(true);

        assert_eq!(state.on_vblank(None), (VblankAction::Redraw, None));
        assert!(state.is_redraw_queued());
    }

    #[test]
    fn settled_frame_returns_to_idle_after_vblank() {
        let mut state = state();
        state.queue_redraw();
        state.frame_submitted(false);

        assert_eq!(state.on_vblank(None), (VblankAction::SendCallbacks, None));
        assert!(!state.is_redraw_queued());
    }

    #[test]
    fn skipped_frame_arms_a_nonzero_fallback_delay() {
        let mut state = state();
        state.queue_redraw();

        assert_eq!(
            state.frame_skipped(false, Duration::from_secs(5)),
            EstimatedVblankTimer::ArmAfter(Duration::from_millis(1))
        );
    }

    #[test]
    fn estimated_vblank_releases_callbacks_after_a_settled_skipped_frame() {
        let mut state = state();
        state.queue_redraw();
        state.frame_skipped(false, Duration::from_secs(5));
        state.timer_armed(registration_token());

        assert_eq!(state.estimated_vblank_fired(), Ok(true));
        assert!(!state.is_redraw_queued());
    }

    #[test]
    fn redraw_queued_during_estimated_wait_takes_precedence_over_callbacks() {
        let mut state = state();
        state.queue_redraw();
        state.frame_skipped(false, Duration::from_secs(5));
        state.timer_armed(registration_token());
        state.queue_redraw();

        assert_eq!(state.estimated_vblank_fired(), Ok(false));
        assert!(state.is_redraw_queued());
    }

    #[test]
    fn vblank_throttle_delays_only_implausibly_early_events() {
        let mut state = state();
        let refresh = Duration::from_millis(10);

        let first = state.throttle_vblank(Some(Duration::from_millis(100)), refresh);
        assert_eq!(first.delay, None);
        assert!(!first.warn);

        let early = state.throttle_vblank(Some(Duration::from_millis(103)), refresh);
        assert_eq!(early.delay, Some(Duration::from_millis(7)));
        assert!(early.warn);

        let normal = state.throttle_vblank(Some(Duration::from_millis(110)), refresh);
        assert_eq!(normal.delay, None);
        assert!(!normal.warn);
    }

    #[test]
    fn vblank_throttle_warning_is_bounded_and_replaces_its_timer() {
        let mut state = state();
        let refresh = Duration::from_millis(10);
        assert_eq!(
            state
                .throttle_vblank(Some(Duration::from_millis(100)), refresh)
                .delay,
            None
        );

        let early = state.throttle_vblank(Some(Duration::from_millis(102)), refresh);
        assert!(early.warn);
        state.vblank_throttle_timer_armed(registration_token());

        let repeated = state.throttle_vblank(Some(Duration::from_millis(103)), refresh);
        assert_eq!(repeated.delay, Some(Duration::from_millis(7)));
        assert!(!repeated.warn);
        assert!(repeated.cancel_timer.is_some());
    }

    #[test]
    fn suspended_output_ignores_redraws_and_late_vblanks_until_resume() {
        let mut state = state();
        state.queue_redraw();
        state.frame_submitted(false);
        drop(state.suspend(Duration::from_secs(1)));

        state.queue_redraw();
        assert!(!state.is_redraw_queued());
        assert_eq!(state.on_vblank(None), (VblankAction::Ignore, None));
        assert!(!state.is_redraw_queued());

        state.resume(Duration::from_secs(2));
        assert!(state.is_redraw_queued());
    }
}
