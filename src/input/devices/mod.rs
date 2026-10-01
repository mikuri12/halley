mod settings;

use halley_config::DeviceKind;
use smithay::reexports::input::{Device, DeviceCapability};

use settings::{DeviceDefaults, apply};

struct ManagedDevice {
    device: Device,
    kind: DeviceKind,
    defaults: DeviceDefaults,
    touch_capable: bool,
}

/// Owns only physical libinput configuration and device-presence facts.
/// Wayland seat capabilities and event routing remain session policy.
#[derive(Default)]
pub struct PhysicalInputDevices {
    devices: Vec<ManagedDevice>,
    touch_count: usize,
}

impl PhysicalInputDevices {
    /// Adds and configures one physical device. Returns `true` only when the
    /// first touchscreen appears and the session should advertise wl_touch.
    pub fn added(&mut self, mut device: Device, input: &halley_config::Input) -> bool {
        let Some(kind) = classify(&device) else {
            return false;
        };
        let touch_capable = device.has_capability(DeviceCapability::Touch);
        let defaults = DeviceDefaults::read(&device);
        apply(&mut device, kind, &defaults, input);
        eventline::debug!("input: managing {:?} {:?}", kind, device.name().as_ref());
        let first_touch = touch_capable && self.touch_count == 0;
        self.touch_count += usize::from(touch_capable);
        self.devices.push(ManagedDevice {
            device,
            kind,
            defaults,
            touch_capable,
        });
        first_touch
    }

    /// Removes one device. Returns `true` only when the final touchscreen
    /// disappears and the session should remove wl_touch.
    pub fn removed(&mut self, device: &Device) -> bool {
        let before = self.touch_count;
        self.devices.retain(|candidate| {
            if candidate.device == *device {
                self.touch_count = self
                    .touch_count
                    .saturating_sub(usize::from(candidate.touch_capable));
                false
            } else {
                true
            }
        });
        before > 0 && self.touch_count == 0
    }

    pub fn reload(&mut self, input: &halley_config::Input) {
        for managed in &mut self.devices {
            apply(&mut managed.device, managed.kind, &managed.defaults, input);
        }
    }

    /// Push a keyboard LED mask (Caps/Num/Scroll Lock) to every managed
    /// device.
    ///
    /// The kernel never updates keyboard LEDs on its own: userspace must call
    /// `libinput_device_led_update()` whenever the xkb LED state changes.
    /// Smithay only invokes `SeatHandler::led_state_changed()` on *changes*,
    /// so this is also called on device add to give keyboards plugged in
    /// mid-session (or present at startup) the current mask instead of the
    /// stale one the kernel booted with. Devices without LEDs ignore it.
    pub fn sync_keyboard_leds(&mut self, leds: smithay::reexports::input::Led) {
        for managed in &mut self.devices {
            managed.device.led_update(leds);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeviceFacts {
    pointer: bool,
    touch: bool,
    tablet_tool: bool,
    tablet_pad: bool,
    tap_fingers: u32,
    trackball: bool,
    trackpoint: bool,
}

fn classify_facts(facts: DeviceFacts) -> Option<DeviceKind> {
    if facts.touch && !facts.tablet_tool && !facts.tablet_pad {
        return Some(DeviceKind::Touchscreen);
    }
    if !facts.pointer || facts.tablet_tool || facts.tablet_pad {
        return None;
    }
    if facts.tap_fingers > 0 {
        Some(DeviceKind::Touchpad)
    } else if facts.trackball {
        Some(DeviceKind::Trackball)
    } else if facts.trackpoint {
        Some(DeviceKind::Trackpoint)
    } else {
        Some(DeviceKind::Mouse)
    }
}

fn classify(device: &Device) -> Option<DeviceKind> {
    let (trackball, trackpoint) = unsafe { device.udev_device() }.map_or((false, false), |udev| {
        (
            udev.property_value("ID_INPUT_TRACKBALL").is_some(),
            udev.property_value("ID_INPUT_POINTINGSTICK").is_some(),
        )
    });
    classify_facts(DeviceFacts {
        pointer: device.has_capability(DeviceCapability::Pointer),
        touch: device.has_capability(DeviceCapability::Touch),
        tablet_tool: device.has_capability(DeviceCapability::TabletTool),
        tablet_pad: device.has_capability(DeviceCapability::TabletPad),
        tap_fingers: device.config_tap_finger_count(),
        trackball,
        trackpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse_facts() -> DeviceFacts {
        DeviceFacts {
            pointer: true,
            touch: false,
            tablet_tool: false,
            tablet_pad: false,
            tap_fingers: 0,
            trackball: false,
            trackpoint: false,
        }
    }

    #[test]
    fn classifies_each_desktop_device_without_overlap() {
        assert_eq!(classify_facts(mouse_facts()), Some(DeviceKind::Mouse));
        assert_eq!(
            classify_facts(DeviceFacts {
                tap_fingers: 3,
                ..mouse_facts()
            }),
            Some(DeviceKind::Touchpad)
        );
        assert_eq!(
            classify_facts(DeviceFacts {
                trackpoint: true,
                ..mouse_facts()
            }),
            Some(DeviceKind::Trackpoint)
        );
        assert_eq!(
            classify_facts(DeviceFacts {
                trackball: true,
                ..mouse_facts()
            }),
            Some(DeviceKind::Trackball)
        );
        assert_eq!(
            classify_facts(DeviceFacts {
                pointer: false,
                touch: true,
                ..mouse_facts()
            }),
            Some(DeviceKind::Touchscreen)
        );
    }

    #[test]
    fn excludes_tablet_and_non_pointer_devices() {
        for facts in [
            DeviceFacts {
                pointer: false,
                ..mouse_facts()
            },
            DeviceFacts {
                tablet_tool: true,
                ..mouse_facts()
            },
            DeviceFacts {
                tablet_pad: true,
                ..mouse_facts()
            },
        ] {
            assert_eq!(classify_facts(facts), None);
        }
    }
}
