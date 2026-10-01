use smithay::input::keyboard::ModifiersState;

/// Resolve the kernel's standard Ctrl+Alt+Fn rescue chord.
///
/// Smithay's libinput backend reports XKB keycodes, which are Linux evdev
/// keycodes plus eight. Handling this below ordinary compositor key routing
/// keeps VT switching available through modal UIs, session lock, and client
/// shortcut inhibition.
pub(super) fn target_from_keycode(
    xkb_keycode: u32,
    pressed: bool,
    modifiers: ModifiersState,
) -> Option<i32> {
    if !pressed || !modifiers.ctrl || !modifiers.alt {
        return None;
    }
    match xkb_keycode {
        // KEY_F1..KEY_F10 are evdev 59..68; F11/F12 are 87/88.
        67..=76 => Some((xkb_keycode - 66) as i32),
        95 => Some(11),
        96 => Some(12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_alt() -> ModifiersState {
        ModifiersState {
            ctrl: true,
            alt: true,
            ..ModifiersState::default()
        }
    }

    #[test]
    fn ctrl_alt_function_keys_map_to_all_twelve_vts() {
        let expected = [
            (67, 1),
            (68, 2),
            (69, 3),
            (70, 4),
            (71, 5),
            (72, 6),
            (73, 7),
            (74, 8),
            (75, 9),
            (76, 10),
            (95, 11),
            (96, 12),
        ];

        for (keycode, vt) in expected {
            assert_eq!(target_from_keycode(keycode, true, ctrl_alt()), Some(vt));
        }
    }

    #[test]
    fn release_and_incomplete_chords_do_not_switch() {
        assert_eq!(target_from_keycode(67, false, ctrl_alt()), None);
        assert_eq!(
            target_from_keycode(
                67,
                true,
                ModifiersState {
                    ctrl: true,
                    ..ModifiersState::default()
                },
            ),
            None
        );
        assert_eq!(
            target_from_keycode(
                67,
                true,
                ModifiersState {
                    alt: true,
                    ..ModifiersState::default()
                },
            ),
            None
        );
    }

    #[test]
    fn ctrl_alt_non_function_key_is_ignored() {
        assert_eq!(target_from_keycode(38, true, ctrl_alt()), None);
    }
}
