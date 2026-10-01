#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelProfile {
    Adaptive,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollMethod {
    None,
    TwoFinger,
    Edge,
    OnButtonDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickMethod {
    Clickfinger,
    ButtonAreas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Touchpad,
    Trackpoint,
    Trackball,
    Touchscreen,
}

/// Optional settings shared by device-class defaults and exact-name
/// overrides. Fields which do not apply to a device class are left unused.
/// `None` always means to restore that physical device's own default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceSettings {
    pub enabled: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<AccelProfile>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub scroll_button_lock: Option<bool>,
    pub left_handed: Option<bool>,
    pub middle_emulation: Option<bool>,
    pub tap: Option<bool>,
    pub tap_button_map: Option<TapButtonMap>,
    pub dwt: Option<bool>,
    pub click_method: Option<ClickMethod>,
    pub drag: Option<bool>,
    pub drag_lock: Option<bool>,
    pub disabled_on_external_mouse: Option<bool>,
    pub map_to_output: Option<String>,
    pub calibration_matrix: Option<[f32; 6]>,
}

impl DeviceSettings {
    pub fn overlay(&self, override_settings: &Self) -> Self {
        Self {
            enabled: override_settings.enabled.or(self.enabled),
            natural_scroll: override_settings.natural_scroll.or(self.natural_scroll),
            accel_speed: override_settings.accel_speed.or(self.accel_speed),
            accel_profile: override_settings.accel_profile.or(self.accel_profile),
            scroll_method: override_settings.scroll_method.or(self.scroll_method),
            scroll_button: override_settings.scroll_button.or(self.scroll_button),
            scroll_button_lock: override_settings
                .scroll_button_lock
                .or(self.scroll_button_lock),
            left_handed: override_settings.left_handed.or(self.left_handed),
            middle_emulation: override_settings.middle_emulation.or(self.middle_emulation),
            tap: override_settings.tap.or(self.tap),
            tap_button_map: override_settings.tap_button_map.or(self.tap_button_map),
            dwt: override_settings.dwt.or(self.dwt),
            click_method: override_settings.click_method.or(self.click_method),
            drag: override_settings.drag.or(self.drag),
            drag_lock: override_settings.drag_lock.or(self.drag_lock),
            disabled_on_external_mouse: override_settings
                .disabled_on_external_mouse
                .or(self.disabled_on_external_mouse),
            map_to_output: override_settings
                .map_to_output
                .clone()
                .or_else(|| self.map_to_output.clone()),
            calibration_matrix: override_settings
                .calibration_matrix
                .or(self.calibration_matrix),
        }
    }
}

/// Backward-compatible name for the original mouse-only public type.
pub type MouseSettings = DeviceSettings;

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceOverride {
    pub name: String,
    pub settings: DeviceSettings,
}
