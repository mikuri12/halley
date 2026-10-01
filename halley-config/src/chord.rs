use crate::keybinds::Modifiers;

/// Parse a chord string like `"super+shift+e"` into its modifier flags and
/// key name. The last `+`-separated segment is always the key; every
/// segment before it must be a recognized modifier name
/// (`super`, `alt`, `ctrl`, `shift`, or their `l*`/`r*` variants - matched
/// case-insensitively). Returns `None` if the key segment is empty or any
/// modifier segment isn't recognized - a chord with a typo'd modifier
/// should fail to parse, not silently drop the typo and bind something the
/// user didn't intend.
///
/// A chord with no `+` at all (e.g. `"print"`) is valid - a bare key with
/// no modifiers.
pub fn parse_chord(chord: &str) -> Option<(Modifiers, String)> {
    let mut parts: Vec<&str> = chord.split('+').collect();
    let key = parts.pop()?.trim();
    if key.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::default();
    for part in parts {
        match part.trim().to_lowercase().as_str() {
            "super" | "logo" | "mod4" => modifiers.super_key = true,
            "lsuper" | "left-super" | "lwin" | "left-logo" => modifiers.left_super = true,
            "rsuper" | "right-super" | "rwin" | "right-logo" => modifiers.right_super = true,
            "alt" => modifiers.alt = true,
            "lalt" | "left-alt" => modifiers.left_alt = true,
            "ralt" | "right-alt" => modifiers.right_alt = true,
            "ctrl" | "control" => modifiers.ctrl = true,
            "lctrl" | "left-ctrl" | "left-control" => modifiers.left_ctrl = true,
            "rctrl" | "right-ctrl" | "right-control" => modifiers.right_ctrl = true,
            "shift" => modifiers.shift = true,
            "lshift" | "left-shift" => modifiers.left_shift = true,
            "rshift" | "right-shift" => modifiers.right_shift = true,
            _ => return None,
        }
    }

    Some((modifiers, key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifier_plus_shift_plus_key() {
        let (modifiers, key) = parse_chord("super+shift+e").unwrap();
        assert!(modifiers.super_key);
        assert!(modifiers.shift);
        assert!(!modifiers.alt);
        assert!(!modifiers.ctrl);
        assert_eq!(key, "e");
    }

    #[test]
    fn parses_single_modifier_plus_key() {
        let (modifiers, key) = parse_chord("super+c").unwrap();
        assert!(modifiers.super_key);
        assert!(!modifiers.shift);
        assert_eq!(key, "c");
    }

    #[test]
    fn parses_bare_key_with_no_modifiers() {
        let (modifiers, key) = parse_chord("print").unwrap();
        assert_eq!(modifiers, Modifiers::default());
        assert_eq!(key, "print");
    }

    #[test]
    fn recognizes_alt_ctrl_and_logo_aliases() {
        let (modifiers, _) = parse_chord("alt+ctrl+q").unwrap();
        assert!(modifiers.alt);
        assert!(modifiers.ctrl);

        let (modifiers, _) = parse_chord("logo+q").unwrap();
        assert!(modifiers.super_key);

        let (modifiers, _) = parse_chord("mod4+q").unwrap();
        assert!(modifiers.super_key);
    }

    #[test]
    fn is_case_insensitive_on_modifier_names() {
        let (modifiers, key) = parse_chord("Super+SHIFT+e").unwrap();
        assert!(modifiers.super_key);
        assert!(modifiers.shift);
        assert_eq!(key, "e");
    }

    #[test]
    fn rejects_unknown_modifier_name() {
        assert!(parse_chord("super+frobnicate+e").is_none());
    }

    #[test]
    fn rejects_empty_key() {
        assert!(parse_chord("super+shift+").is_none());
        assert!(parse_chord("").is_none());
    }

    #[test]
    fn parses_per_side_modifiers() {
        let (modifiers, key) = parse_chord("lsuper+rctrl+lshift+x").unwrap();
        assert!(modifiers.left_super);
        assert!(modifiers.right_ctrl);
        assert!(modifiers.left_shift);
        assert!(!modifiers.super_key);
        assert_eq!(key, "x");
    }
}
