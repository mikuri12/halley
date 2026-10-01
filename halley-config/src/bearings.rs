use rune_cfg::RuneConfig;

pub const MIN_FADE_DISTANCE: f32 = 120.0;
pub const MAX_FADE_DISTANCE: f32 = 100_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bearings {
    pub show_distance: bool,
    pub show_icons: bool,
    pub show_pinned: bool,
    pub fade_distance: f32,
    pub blur: bool,
}

impl Default for Bearings {
    fn default() -> Self {
        Self {
            show_distance: true,
            show_icons: true,
            show_pinned: true,
            fade_distance: 1_200.0,
            blur: true,
        }
    }
}

pub fn parse_bearings(config: &RuneConfig) -> Bearings {
    let defaults = Bearings::default();
    Bearings {
        show_distance: config.get_or("bearings.show-distance", defaults.show_distance),
        show_icons: config.get_or("bearings.show-icons", defaults.show_icons),
        show_pinned: config.get_or("bearings.show-pinned", defaults.show_pinned),
        fade_distance: config
            .get_or("bearings.fade-distance", defaults.fade_distance)
            .clamp(MIN_FADE_DISTANCE, MAX_FADE_DISTANCE),
        blur: config.get_or("bearings.blur", defaults.blur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_old_halley() {
        let bearings = parse_bearings(&RuneConfig::from_str("").unwrap());
        assert!(bearings.show_distance);
        assert!(bearings.show_icons);
        assert!(bearings.show_pinned);
        assert_eq!(bearings.fade_distance, 1_200.0);
        assert!(bearings.blur);
    }

    #[test]
    fn fade_distance_is_clamped_to_the_old_safe_range() {
        let low = RuneConfig::from_str("bearings:\n  fade-distance 1\nend\n").unwrap();
        let high = RuneConfig::from_str("bearings:\n  fade-distance 1000000\nend\n").unwrap();
        assert_eq!(parse_bearings(&low).fade_distance, MIN_FADE_DISTANCE);
        assert_eq!(parse_bearings(&high).fade_distance, MAX_FADE_DISTANCE);
    }
}
