use rune_cfg::RuneConfig;

/// A border's flat RGB color (always fully opaque) - matches old halley's
/// own `DecorationBorderColor` (`layout/types.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TitlebarButtonPosition {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TitlebarContentPosition {
    Left,
    #[default]
    Center,
    Right,
}

/// Compositor-owned titlebar styling.
///
/// `height_px` is the requested height. Rendering raises it when necessary to
/// fit enabled controls, application icons, or the effective title text size.
#[derive(Clone, Debug, PartialEq)]
pub struct Titlebars {
    pub enabled: bool,
    pub button_position: TitlebarButtonPosition,
    pub title_position: TitlebarContentPosition,
    pub show_buttons: bool,
    pub show_icons: bool,
    pub show_title: bool,
    /// Optional title-only font size. `None` inherits the global font size.
    pub text_size_px: Option<u16>,
    pub radius_px: i32,
    pub height_px: i32,
    pub color_focused: BorderColor,
    pub color_unfocused: BorderColor,
    pub foreground_color_focused: BorderColor,
    pub foreground_color_unfocused: BorderColor,
    pub button_hover_color: BorderColor,
    pub button_pressed_color: BorderColor,
    /// Optional custom button-glyph SVG paths (close/minimize/maximize/
    /// unmaximize). Relative paths resolve against the configuration file.
    /// When a path is set, the bundled glyph is replaced by that file's
    /// alpha mask and drawn in the runtime button tint.
    pub icon_paths: TitlebarIconPaths,
    /// Optional explicit tints for the four button glyphs, overriding the
    /// state-derived foreground. Unset slots keep the default behavior.
    pub icon_colors: TitlebarIconColors,
    /// When true, the titlebar background draws in the window's border
    /// color (focused/unfocused) instead of its own, so the frame and the
    /// decoration read as one surface covered by both borders.
    pub follow_border: bool,
}

/// Per-glyph custom SVG sources for titlebar buttons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitlebarIconPaths {
    pub close: Option<std::path::PathBuf>,
    pub minimize: Option<std::path::PathBuf>,
    pub maximize: Option<std::path::PathBuf>,
    pub unmaximize: Option<std::path::PathBuf>,
}

/// Per-glyph explicit tints; `None` keeps the state color.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TitlebarIconColors {
    pub close: Option<BorderColor>,
    pub minimize: Option<BorderColor>,
    pub maximize: Option<BorderColor>,
    pub unmaximize: Option<BorderColor>,
}

impl Default for Titlebars {
    fn default() -> Self {
        Self {
            enabled: true,
            button_position: TitlebarButtonPosition::Right,
            title_position: TitlebarContentPosition::Center,
            show_buttons: true,
            show_icons: false,
            show_title: true,
            text_size_px: None,
            radius_px: 8,
            height_px: 32,
            color_focused: BorderColor {
                r: 0xd6 as f32 / 255.0,
                g: 0x5d as f32 / 255.0,
                b: 0x26 as f32 / 255.0,
            },
            color_unfocused: BorderColor {
                r: 0x47 as f32 / 255.0,
                g: 0x4d as f32 / 255.0,
                b: 0x59 as f32 / 255.0,
            },
            foreground_color_focused: BorderColor {
                r: 0x10 as f32 / 255.0,
                g: 0x14 as f32 / 255.0,
                b: 0x18 as f32 / 255.0,
            },
            foreground_color_unfocused: BorderColor {
                r: 0xf4 as f32 / 255.0,
                g: 0xf5 as f32 / 255.0,
                b: 0xf7 as f32 / 255.0,
            },
            button_hover_color: BorderColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
            },
            button_pressed_color: BorderColor {
                r: 0x10 as f32 / 255.0,
                g: 0x14 as f32 / 255.0,
                b: 0x18 as f32 / 255.0,
            },
            icon_paths: TitlebarIconPaths::default(),
            icon_colors: TitlebarIconColors::default(),
            follow_border: false,
        }
    }
}

/// Primary compositor window styling.
///
/// The radius describes the client-content corner. The renderer grows the
/// outer border radius concentrically by the configured border width. It also
/// clips unmanaged X11 popup content even though those surfaces receive no
/// compositor border or titlebar.
#[derive(Clone, Debug, PartialEq)]
pub struct Decorations {
    pub border_width_px: i32,
    pub border_radius_px: i32,
    pub border_color_focused: BorderColor,
    pub border_color_unfocused: BorderColor,
    pub resize_using_border: bool,
    pub secondary_border: SecondaryBorder,
    pub titlebars: Titlebars,
}

/// A second border drawn outside the primary one, separated by a gap.
/// Disabled by default; mirrors old halley's `SecondaryBorderConfig`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecondaryBorder {
    pub enabled: bool,
    pub size_px: i32,
    pub gap_px: i32,
    pub color_focused: BorderColor,
    pub color_unfocused: BorderColor,
}

impl Default for SecondaryBorder {
    fn default() -> Self {
        Self {
            enabled: false,
            size_px: 4,
            gap_px: 0,
            color_focused: BorderColor {
                r: 0xfa as f32 / 255.0,
                g: 0xbd as f32 / 255.0,
                b: 0x2f as f32 / 255.0,
            },
            color_unfocused: BorderColor {
                r: 0x1f as f32 / 255.0,
                g: 0x1f as f32 / 255.0,
                b: 0x1f as f32 / 255.0,
            },
        }
    }
}

impl Default for Decorations {
    /// Same defaults as old halley's `PrimaryBorderConfig`.
    fn default() -> Self {
        Self {
            border_width_px: 3,
            border_radius_px: 8,
            border_color_focused: BorderColor {
                r: 0xf4 as f32 / 255.0,
                g: 0xf5 as f32 / 255.0,
                b: 0xf7 as f32 / 255.0,
            },
            border_color_unfocused: BorderColor {
                r: 0x47 as f32 / 255.0,
                g: 0x4d as f32 / 255.0,
                b: 0x59 as f32 / 255.0,
            },
            resize_using_border: true,
            secondary_border: SecondaryBorder::default(),
            titlebars: Titlebars::default(),
        }
    }
}

/// Parse the `decorations.border` section - tolerant of a missing section
/// or missing individual keys (each just falls back to `Decorations`'s
/// default), matching `parse_keybinds`'s style. Unlike old halley's own
/// parser, this doesn't need to enumerate kebab/snake-case alias spellings
/// by hand - rune-cfg's dotted-path getters already try both case
/// conventions for every path segment. `colour`/`color` is a genuine
/// spelling alias rather than a case convention though, so that part still
/// needs both paths tried explicitly, same as old halley did.
pub fn parse_decorations(config: &RuneConfig) -> Decorations {
    let defaults = Decorations::default();

    let border_width_px = config
        .get_or("decorations.border.size", defaults.border_width_px)
        .clamp(0, 64);
    let border_radius_px = config
        .get_or(
            "decorations.border.radius",
            config.get_or("decorations.border-radius", defaults.border_radius_px),
        )
        .clamp(0, 256);
    let border_color_focused = parse_color(
        config,
        &[
            "decorations.border.colour-focused",
            "decorations.border.color-focused",
        ],
        defaults.border_color_focused,
    );
    let border_color_unfocused = parse_color(
        config,
        &[
            "decorations.border.colour-unfocused",
            "decorations.border.color-unfocused",
        ],
        defaults.border_color_unfocused,
    );
    let titlebar_defaults = defaults.titlebars;
    let button_position = match config
        .get_or("decorations.titlebars.button-position", "right".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "left" => TitlebarButtonPosition::Left,
        _ => TitlebarButtonPosition::Right,
    };
    let title_position = match config
        .get_or("decorations.titlebars.title-position", "center".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "left" => TitlebarContentPosition::Left,
        "right" => TitlebarContentPosition::Right,
        _ => TitlebarContentPosition::Center,
    };
    let titlebars = Titlebars {
        enabled: config.get_or("decorations.titlebars.enabled", titlebar_defaults.enabled),
        button_position,
        title_position,
        show_buttons: config.get_or(
            "decorations.titlebars.show-buttons",
            titlebar_defaults.show_buttons,
        ),
        show_icons: config.get_or(
            "decorations.titlebars.show-icons",
            titlebar_defaults.show_icons,
        ),
        show_title: config.get_or(
            "decorations.titlebars.show-title",
            titlebar_defaults.show_title,
        ),
        text_size_px: config
            .get_optional::<u64>("decorations.titlebars.text-size")
            .ok()
            .flatten()
            .map(|size| size.clamp(1, 96) as u16),
        radius_px: config
            .get_or("decorations.titlebars.radius", titlebar_defaults.radius_px)
            .clamp(0, 96),
        height_px: config
            .get_or("decorations.titlebars.height", titlebar_defaults.height_px)
            .clamp(1, 96),
        color_focused: parse_color(
            config,
            &[
                "decorations.titlebars.colour-focused",
                "decorations.titlebars.color-focused",
            ],
            titlebar_defaults.color_focused,
        ),
        color_unfocused: parse_color(
            config,
            &[
                "decorations.titlebars.colour-unfocused",
                "decorations.titlebars.color-unfocused",
            ],
            titlebar_defaults.color_unfocused,
        ),
        foreground_color_focused: parse_color(
            config,
            &[
                "decorations.titlebars.foreground-colour-focused",
                "decorations.titlebars.foreground-color-focused",
            ],
            titlebar_defaults.foreground_color_focused,
        ),
        foreground_color_unfocused: parse_color(
            config,
            &[
                "decorations.titlebars.foreground-colour-unfocused",
                "decorations.titlebars.foreground-color-unfocused",
            ],
            titlebar_defaults.foreground_color_unfocused,
        ),
        button_hover_color: parse_color(
            config,
            &[
                "decorations.titlebars.button-hover-colour",
                "decorations.titlebars.button-hover-color",
            ],
            titlebar_defaults.button_hover_color,
        ),
        button_pressed_color: parse_color(
            config,
            &[
                "decorations.titlebars.button-pressed-colour",
                "decorations.titlebars.button-pressed-color",
            ],
            titlebar_defaults.button_pressed_color,
        ),
        icon_paths: parse_titlebar_icon_paths(config),
        icon_colors: parse_titlebar_icon_colors(config),
        follow_border: config.get_or(
            "decorations.titlebars.follow-border",
            titlebar_defaults.follow_border,
        ),
    };

    Decorations {
        border_width_px,
        border_radius_px,
        border_color_focused,
        border_color_unfocused,
        resize_using_border: config.get_or(
            "decorations.resize-using-border",
            defaults.resize_using_border,
        ),
        secondary_border: parse_secondary_border(config, defaults.secondary_border),
        titlebars,
    }
}

fn parse_secondary_border(config: &RuneConfig, defaults: SecondaryBorder) -> SecondaryBorder {
    let enabled = config.get_or(
        "decorations.secondary-border.enabled",
        defaults.enabled,
    );
    let size_px = config
        .get_or("decorations.secondary-border.size", defaults.size_px)
        .clamp(0, 32);
    let gap_px = config
        .get_or("decorations.secondary-border.gap", defaults.gap_px)
        .clamp(0, 64);
    let color_focused = parse_color(
        config,
        &[
            "decorations.secondary-border.colour-focused",
            "decorations.secondary-border.color-focused",
        ],
        defaults.color_focused,
    );
    let color_unfocused = parse_color(
        config,
        &[
            "decorations.secondary-border.colour-unfocused",
            "decorations.secondary-border.color-unfocused",
        ],
        defaults.color_unfocused,
    );
    SecondaryBorder {
        enabled,
        size_px,
        gap_px,
        color_focused,
        color_unfocused,
    }
}

/// Loads `Decorations` from the user's config file, falling back to
/// `Decorations::default()` on any failure (missing `$HOME`, unwritable
/// config dir, parse error) - mirrors `input::keybinds::load_keybinds`'s
/// load-or-default shape, but self-contained here since decorations need no
/// backend-specific resolution.
pub fn load_decorations() -> Decorations {
    let Some(path) = crate::config_path() else {
        eprintln!("decorations: no config path resolvable, using defaults");
        return Decorations::default();
    };

    if let Err(err) = crate::bootstrap_default_config_at(&path) {
        eprintln!("decorations: failed to bootstrap default config: {err}");
    }

    match RuneConfig::from_file(&path) {
        Ok(config) => parse_decorations(&config),
        Err(err) => {
            eprintln!("decorations: failed to load {path:?}, using defaults: {err}");
            Decorations::default()
        }
    }
}

fn parse_color(config: &RuneConfig, paths: &[&str], default: BorderColor) -> BorderColor {
    let Some(raw) = paths
        .iter()
        .find_map(|path| config.get_optional::<String>(path).ok().flatten())
    else {
        return default;
    };
    parse_hex_rgb(raw.trim())
        .map(|(r, g, b)| BorderColor { r, g, b })
        .unwrap_or(default)
}

/// Parses `decorations.titlebars.icon-{glyph}` SVG path settings. Relative
/// paths are stored as given and resolved against the configuration file's
/// directory when the renderer loads them.
fn parse_titlebar_icon_paths(config: &RuneConfig) -> TitlebarIconPaths {
    let slot = |kebab: &str, snake: &str| -> Option<std::path::PathBuf> {
        config
            .get_optional::<String>(kebab)
            .or_else(|_| config.get_optional::<String>(snake))
            .ok()
            .flatten()
            .filter(|raw| !raw.trim().is_empty())
            .map(std::path::PathBuf::from)
    };
    TitlebarIconPaths {
        close: slot("decorations.titlebars.icon-close", "decorations.titlebars.icon_close"),
        minimize: slot(
            "decorations.titlebars.icon-minimize",
            "decorations.titlebars.icon_minimize",
        ),
        maximize: slot(
            "decorations.titlebars.icon-maximize",
            "decorations.titlebars.icon_maximize",
        ),
        unmaximize: slot(
            "decorations.titlebars.icon-unmaximize",
            "decorations.titlebars.icon_unmaximize",
        ),
    }
}

/// Parses `decorations.titlebars.icon-{glyph}-colour` explicit tints. Unset
/// slots stay `None` and keep the state-derived button foreground.
fn parse_titlebar_icon_colors(config: &RuneConfig) -> TitlebarIconColors {
    let slot = |kebab: &str, snake: &str| -> Option<BorderColor> {
        let raw = config
            .get_optional::<String>(kebab)
            .or_else(|_| config.get_optional::<String>(snake))
            .ok()
            .flatten()?;
        parse_hex_rgb(raw.trim()).map(|(r, g, b)| BorderColor { r, g, b })
    };
    TitlebarIconColors {
        close: slot(
            "decorations.titlebars.icon-close-colour",
            "decorations.titlebars.icon_close_color",
        ),
        minimize: slot(
            "decorations.titlebars.icon-minimize-colour",
            "decorations.titlebars.icon_minimize_color",
        ),
        maximize: slot(
            "decorations.titlebars.icon-maximize-colour",
            "decorations.titlebars.icon_maximize_color",
        ),
        unmaximize: slot(
            "decorations.titlebars.icon-unmaximize-colour",
            "decorations.titlebars.icon_unmaximize_color",
        ),
    }
}

/// Parses `"#rrggbb"` or `"#rgb"` into 0.0-1.0 float components - ported
/// directly from old halley's own `parse/primitives.rs::parse_hex_rgb`
/// (halley's own prior code, not a third party's).
fn parse_hex_rgb(value: &str) -> Option<(f32, f32, f32)> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let expanded = match hex.len() {
        3 => {
            let mut out = String::with_capacity(6);
            for ch in hex.chars() {
                out.push(ch);
                out.push(ch);
            }
            out
        }
        6 => hex.to_string(),
        _ => return None,
    };

    let r = u8::from_str_radix(&expanded[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&expanded[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&expanded[4..6], 16).ok()? as f32 / 255.0;
    Some((r, g, b))
}

impl BorderColor {
    /// Parses `#rrggbb` / `#rgb` into a `BorderColor`. Shared with the
    /// window-rule decoration theming so both spellings stay identical.
    pub fn parse_hex(value: &str) -> Option<Self> {
        parse_hex_rgb(value.trim()).map(|(r, g, b)| Self { r, g, b })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_border_section() {
        let config = RuneConfig::from_str(
            r##"
decorations:
  border:
    size 4
    radius 9
    colour-focused "#d65d26"
    color-unfocused "#333333"
  end
end
"##,
        )
        .expect("valid rune-cfg source");

        let decorations = parse_decorations(&config);

        assert_eq!(decorations.border_width_px, 4);
        assert_eq!(decorations.border_radius_px, 9);
        assert_eq!(
            decorations.border_color_focused,
            BorderColor {
                r: 0xd6 as f32 / 255.0,
                g: 0x5d as f32 / 255.0,
                b: 0x26 as f32 / 255.0,
            }
        );
        assert_eq!(
            decorations.border_color_unfocused,
            BorderColor {
                r: 0x33 as f32 / 255.0,
                g: 0x33 as f32 / 255.0,
                b: 0x33 as f32 / 255.0,
            }
        );
        assert!(decorations.resize_using_border);
    }

    #[test]
    fn border_resize_defaults_on_and_can_be_disabled() {
        let defaults = parse_decorations(
            &RuneConfig::from_str("decorations:\nend\n").expect("valid default config"),
        );
        assert!(defaults.resize_using_border);

        let disabled = parse_decorations(
            &RuneConfig::from_str("decorations:\n  resize-using-border false\nend\n")
                .expect("valid disabled config"),
        );
        assert!(!disabled.resize_using_border);
    }

    #[test]
    fn parses_titlebars_and_color_aliases() {
        let config = RuneConfig::from_str(
            r##"
decorations:
  titlebars:
    enabled false
    button-position "right"
    title-position "right"
    show-buttons false
    show-icons true
    show-title false
    text-size 18
    radius 12
    height 40
    color-focused "#123"
    colour-unfocused "#456"
    foreground-color-focused "#789"
    foreground-colour-unfocused "#abc"
    button-hover-color "#def"
    button-pressed-colour "#fed"
  end
end
"##,
        )
        .expect("valid rune-cfg source");

        let titlebars = parse_decorations(&config).titlebars;
        assert!(!titlebars.enabled);
        assert_eq!(titlebars.button_position, TitlebarButtonPosition::Right);
        assert_eq!(titlebars.title_position, TitlebarContentPosition::Right);
        assert!(!titlebars.show_buttons);
        assert!(titlebars.show_icons);
        assert!(!titlebars.show_title);
        assert_eq!(titlebars.text_size_px, Some(18));
        assert_eq!(titlebars.radius_px, 12);
        assert_eq!(titlebars.height_px, 40);
        assert_eq!(titlebars.color_focused.r, 0x11 as f32 / 255.0);
        assert_eq!(titlebars.color_unfocused.g, 0x55 as f32 / 255.0);
        assert_eq!(titlebars.foreground_color_focused.b, 0x99 as f32 / 255.0);
        assert_eq!(titlebars.foreground_color_unfocused.r, 0xaa as f32 / 255.0);
        assert_eq!(titlebars.button_hover_color.g, 0xee as f32 / 255.0);
        assert_eq!(titlebars.button_pressed_color.b, 0xdd as f32 / 255.0);
    }

    #[test]
    fn missing_section_falls_back_to_defaults() {
        let config = RuneConfig::from_str("keybinds:\n  mod \"super\"\nend\n")
            .expect("valid rune-cfg source");

        assert_eq!(parse_decorations(&config), Decorations::default());
    }

    #[test]
    fn defaults_use_neutral_border_orange_titlebar_and_right_controls() {
        let defaults = Decorations::default();
        assert_eq!(defaults.border_width_px, 3);
        assert_eq!(defaults.border_radius_px, 8);
        assert_eq!(
            defaults.border_color_focused,
            BorderColor {
                r: 0xf4 as f32 / 255.0,
                g: 0xf5 as f32 / 255.0,
                b: 0xf7 as f32 / 255.0,
            }
        );
        assert_eq!(
            defaults.border_color_unfocused,
            BorderColor {
                r: 0x47 as f32 / 255.0,
                g: 0x4d as f32 / 255.0,
                b: 0x59 as f32 / 255.0,
            }
        );
        assert_eq!(
            defaults.titlebars.color_focused,
            BorderColor {
                r: 0xd6 as f32 / 255.0,
                g: 0x5d as f32 / 255.0,
                b: 0x26 as f32 / 255.0,
            }
        );
        assert_eq!(
            defaults.titlebars.button_position,
            TitlebarButtonPosition::Right
        );
    }

    #[test]
    fn clamps_border_metrics_and_accepts_the_old_flat_radius_key() {
        let config = RuneConfig::from_str(
            "decorations:\n  border:\n    size 99\n  end\n  border-radius 999\nend\n",
        )
        .expect("valid rune-cfg source");
        let decorations = parse_decorations(&config);
        assert_eq!(decorations.border_width_px, 64);
        assert_eq!(decorations.border_radius_px, 256);
    }

    #[test]
    fn titlebar_metrics_are_clamped_and_invalid_positions_use_defaults() {
        let config = RuneConfig::from_str(
            "decorations:\n  titlebars:\n    height 0\n    text-size 999\n    radius 999\n    button-position \"middle\"\n    title-position \"middle\"\n  end\nend\n",
        )
        .expect("valid rune-cfg source");

        let titlebars = parse_decorations(&config).titlebars;
        assert_eq!(titlebars.height_px, 1);
        assert_eq!(titlebars.text_size_px, Some(96));
        assert_eq!(titlebars.radius_px, 96);
        assert_eq!(titlebars.button_position, TitlebarButtonPosition::Right);
        assert_eq!(titlebars.title_position, TitlebarContentPosition::Center);
    }

    #[test]
    fn titlebar_title_position_accepts_every_alignment() {
        for (value, expected) in [
            ("left", TitlebarContentPosition::Left),
            ("center", TitlebarContentPosition::Center),
            ("right", TitlebarContentPosition::Right),
        ] {
            let config = RuneConfig::from_str(&format!(
                "decorations:\n  titlebars:\n    title-position \"{value}\"\n  end\nend\n"
            ))
            .expect("valid rune-cfg source");
            assert_eq!(
                parse_decorations(&config).titlebars.title_position,
                expected
            );
        }
    }
}
