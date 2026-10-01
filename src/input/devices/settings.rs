use halley_config::{
    ClickMethod as ConfigClickMethod, DeviceKind, DeviceSettings,
    ScrollMethod as ConfigScrollMethod, TapButtonMap as ConfigTapButtonMap,
};
use smithay::reexports::input::{
    AccelProfile, ClickMethod, Device, DeviceConfigError, DeviceConfigResult, DragLockState,
    ScrollButtonLockState, ScrollMethod, SendEventsMode, TapButtonMap,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DeviceDefaults {
    send_events: SendEventsMode,
    natural_scroll: bool,
    accel_speed: f64,
    accel_profile: Option<AccelProfile>,
    scroll_method: Option<ScrollMethod>,
    scroll_button: u32,
    scroll_button_lock: ScrollButtonLockState,
    left_handed: bool,
    middle_emulation: bool,
    tap: bool,
    tap_button_map: Option<TapButtonMap>,
    dwt: bool,
    click_method: Option<ClickMethod>,
    drag: bool,
    drag_lock: DragLockState,
    calibration_matrix: [f32; 6],
}

impl DeviceDefaults {
    pub(super) fn read(device: &Device) -> Self {
        Self {
            // The Rust libinput wrapper does not expose get_default_mode.
            // DeviceAdded runs before Halley applies anything, so this is the
            // stable baseline to restore on subsequent live reloads.
            send_events: device.config_send_events_mode(),
            natural_scroll: device.config_scroll_default_natural_scroll_enabled(),
            accel_speed: device.config_accel_default_speed(),
            accel_profile: device.config_accel_default_profile(),
            scroll_method: device.config_scroll_default_method(),
            scroll_button: device.config_scroll_default_button(),
            scroll_button_lock: device.config_scroll_default_button_lock(),
            left_handed: device.config_left_handed_default(),
            middle_emulation: device.config_middle_emulation_default_enabled(),
            tap: device.config_tap_default_enabled(),
            tap_button_map: device.config_tap_default_button_map(),
            dwt: device.config_dwt_default_enabled(),
            click_method: device.config_click_default_method(),
            drag: device.config_tap_default_drag_enabled(),
            drag_lock: device.config_tap_default_drag_lock_enabled(),
            calibration_matrix: device
                .config_calibration_default_matrix()
                .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
        }
    }
}

pub(super) fn apply(
    device: &mut Device,
    kind: DeviceKind,
    defaults: &DeviceDefaults,
    input: &halley_config::Input,
) {
    let configured = input.settings_for_device(kind, device.name().as_ref());
    let name = device.name().into_owned();

    apply_result(
        kind,
        &name,
        "enabled",
        device.config_send_events_set_mode(resolve_send_events(kind, defaults, &configured)),
    );

    match kind {
        DeviceKind::Mouse
        | DeviceKind::Touchpad
        | DeviceKind::Trackpoint
        | DeviceKind::Trackball => apply_pointer(device, kind, &name, defaults, &configured),
        DeviceKind::Touchscreen => {
            let matrix = configured
                .calibration_matrix
                .unwrap_or(defaults.calibration_matrix);
            apply_result(
                kind,
                &name,
                "calibration-matrix",
                device.config_calibration_set_matrix(matrix),
            );
        }
    }

    if kind == DeviceKind::Touchpad {
        apply_touchpad(device, &name, defaults, &configured);
    }
}

fn resolve_send_events(
    kind: DeviceKind,
    defaults: &DeviceDefaults,
    configured: &DeviceSettings,
) -> SendEventsMode {
    if configured.enabled == Some(false) {
        SendEventsMode::DISABLED
    } else if kind == DeviceKind::Touchpad && configured.disabled_on_external_mouse == Some(true) {
        SendEventsMode::DISABLED_ON_EXTERNAL_MOUSE
    } else if configured.enabled == Some(true)
        || configured.disabled_on_external_mouse == Some(false)
    {
        SendEventsMode::ENABLED
    } else {
        defaults.send_events
    }
}

fn apply_pointer(
    device: &mut Device,
    kind: DeviceKind,
    name: &str,
    defaults: &DeviceDefaults,
    configured: &DeviceSettings,
) {
    apply_result(
        kind,
        name,
        "natural-scroll",
        device.config_scroll_set_natural_scroll_enabled(
            configured.natural_scroll.unwrap_or(defaults.natural_scroll),
        ),
    );
    apply_result(
        kind,
        name,
        "accel-speed",
        device.config_accel_set_speed(configured.accel_speed.unwrap_or(defaults.accel_speed)),
    );
    let profile = configured
        .accel_profile
        .map(|profile| match profile {
            halley_config::AccelProfile::Adaptive => AccelProfile::Adaptive,
            halley_config::AccelProfile::Flat => AccelProfile::Flat,
        })
        .or(defaults.accel_profile);
    if let Some(profile) = profile {
        apply_result(
            kind,
            name,
            "accel-profile",
            device.config_accel_set_profile(profile),
        );
    }
    let method = configured
        .scroll_method
        .map(map_scroll_method)
        .or(defaults.scroll_method);
    if let Some(method) = method {
        apply_result(
            kind,
            name,
            "scroll-method",
            device.config_scroll_set_method(method),
        );
    }
    apply_result(
        kind,
        name,
        "scroll-button",
        device.config_scroll_set_button(configured.scroll_button.unwrap_or(defaults.scroll_button)),
    );
    let lock = configured
        .scroll_button_lock
        .map(|enabled| {
            if enabled {
                ScrollButtonLockState::Enabled
            } else {
                ScrollButtonLockState::Disabled
            }
        })
        .unwrap_or(defaults.scroll_button_lock);
    apply_result(
        kind,
        name,
        "scroll-button-lock",
        device.config_scroll_set_button_lock(lock),
    );
    apply_result(
        kind,
        name,
        "left-handed",
        device.config_left_handed_set(configured.left_handed.unwrap_or(defaults.left_handed)),
    );
    apply_result(
        kind,
        name,
        "middle-emulation",
        device.config_middle_emulation_set_enabled(
            configured
                .middle_emulation
                .unwrap_or(defaults.middle_emulation),
        ),
    );
}

fn apply_touchpad(
    device: &mut Device,
    name: &str,
    defaults: &DeviceDefaults,
    configured: &DeviceSettings,
) {
    let kind = DeviceKind::Touchpad;
    apply_result(
        kind,
        name,
        "tap",
        device.config_tap_set_enabled(configured.tap.unwrap_or(defaults.tap)),
    );
    if let Some(map) = configured
        .tap_button_map
        .map(map_tap_button_map)
        .or(defaults.tap_button_map)
    {
        apply_result(
            kind,
            name,
            "tap-button-map",
            device.config_tap_set_button_map(map),
        );
    }
    apply_result(
        kind,
        name,
        "dwt",
        device.config_dwt_set_enabled(configured.dwt.unwrap_or(defaults.dwt)),
    );
    if let Some(method) = configured
        .click_method
        .map(map_click_method)
        .or(defaults.click_method)
    {
        apply_result(
            kind,
            name,
            "click-method",
            device.config_click_set_method(method),
        );
    }
    apply_result(
        kind,
        name,
        "drag",
        device.config_tap_set_drag_enabled(configured.drag.unwrap_or(defaults.drag)),
    );
    let drag_lock = configured
        .drag_lock
        .map(|enabled| {
            if enabled {
                DragLockState::EnabledTimeout
            } else {
                DragLockState::Disabled
            }
        })
        .unwrap_or(defaults.drag_lock);
    apply_result(
        kind,
        name,
        "drag-lock",
        device.config_tap_set_drag_lock_enabled(drag_lock),
    );
}

fn map_scroll_method(method: ConfigScrollMethod) -> ScrollMethod {
    match method {
        ConfigScrollMethod::None => ScrollMethod::NoScroll,
        ConfigScrollMethod::TwoFinger => ScrollMethod::TwoFinger,
        ConfigScrollMethod::Edge => ScrollMethod::Edge,
        ConfigScrollMethod::OnButtonDown => ScrollMethod::OnButtonDown,
    }
}

fn map_tap_button_map(map: ConfigTapButtonMap) -> TapButtonMap {
    match map {
        ConfigTapButtonMap::LeftRightMiddle => TapButtonMap::LeftRightMiddle,
        ConfigTapButtonMap::LeftMiddleRight => TapButtonMap::LeftMiddleRight,
    }
}

fn map_click_method(method: ConfigClickMethod) -> ClickMethod {
    match method {
        ConfigClickMethod::Clickfinger => ClickMethod::Clickfinger,
        ConfigClickMethod::ButtonAreas => ClickMethod::ButtonAreas,
    }
}

fn apply_result(kind: DeviceKind, name: &str, setting: &str, result: DeviceConfigResult) {
    match result {
        Ok(()) | Err(DeviceConfigError::Unsupported) => {}
        Err(DeviceConfigError::Invalid) => {
            eventline::warn!("input: {:?} {name:?} rejected {setting}", kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> DeviceDefaults {
        DeviceDefaults {
            send_events: SendEventsMode::ENABLED,
            natural_scroll: false,
            accel_speed: 0.0,
            accel_profile: Some(AccelProfile::Adaptive),
            scroll_method: Some(ScrollMethod::TwoFinger),
            scroll_button: 0,
            scroll_button_lock: ScrollButtonLockState::Disabled,
            left_handed: false,
            middle_emulation: false,
            tap: false,
            tap_button_map: Some(TapButtonMap::LeftRightMiddle),
            dwt: true,
            click_method: Some(ClickMethod::ButtonAreas),
            drag: true,
            drag_lock: DragLockState::Disabled,
            calibration_matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        }
    }

    #[test]
    fn omitted_enabled_restores_original_send_events_mode() {
        let mut original = defaults();
        original.send_events = SendEventsMode::DISABLED_ON_EXTERNAL_MOUSE;
        assert_eq!(
            resolve_send_events(DeviceKind::Touchpad, &original, &DeviceSettings::default()),
            SendEventsMode::DISABLED_ON_EXTERNAL_MOUSE
        );
    }

    #[test]
    fn touchpad_external_mouse_and_explicit_enable_have_clear_precedence() {
        let original = defaults();
        let external = DeviceSettings {
            disabled_on_external_mouse: Some(true),
            ..DeviceSettings::default()
        };
        assert_eq!(
            resolve_send_events(DeviceKind::Touchpad, &original, &external),
            SendEventsMode::DISABLED_ON_EXTERNAL_MOUSE
        );
        let disabled = DeviceSettings {
            enabled: Some(false),
            disabled_on_external_mouse: Some(true),
            ..DeviceSettings::default()
        };
        assert_eq!(
            resolve_send_events(DeviceKind::Touchpad, &original, &disabled),
            SendEventsMode::DISABLED
        );
    }
}
