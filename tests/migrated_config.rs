//! The migrated user configuration in `halley.rune.0.8-mikuri` (repo root)
//! must load through the same strict runtime parser the compositor uses on
//! every reload. This guards against pre-0.8 spellings sneaking back in and
//! silently taking the whole file down — a broken file means Halley starts
//! with defaults and every keybind, gesture, and output setting is lost.

use std::path::Path;

#[test]
fn migrated_user_config_parses_with_strict_runtime_parser() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("halley.rune.0.8-mikuri");
    assert!(
        path.exists(),
        "halley.rune.0.8-mikuri must exist at the repo root"
    );
    match halley_config::load_runtime_config_at(&path) {
        Ok(config) => {
            // Spot-checks that prove the sections actually loaded instead of
            // silently falling back to defaults.
            assert_eq!(config.input.mouse.accel_speed, Some(-0.5));
            assert!(config.input.gestures.enabled);
            assert!(config.fullscreen.preserve_top_panels);
            assert_eq!(
                config.clusters.core_shape,
                halley_config::ClusterCoreShape::Square
            );
            assert_eq!(config.nodes.shape, halley_config::NodeShape::Square);
            assert!(!config.effects.blur.enabled);
            assert!(
                config
                    .keybinds
                    .binds
                    .iter()
                    .any(|bind| bind.action == halley_config::Action::PointerPanField),
                "the pan-field mouse bind must survive the strict parse"
            );
            assert!(
                config
                    .keybinds
                    .binds
                    .iter()
                    .any(|bind| bind.action == halley_config::Action::ToggleFullscreen),
                "the fullscreen bind must survive the strict parse"
            );
        }
        Err(error) => {
            panic!("halley.rune.0.8-mikuri failed the strict parser: {error}");
        }
    }
}
