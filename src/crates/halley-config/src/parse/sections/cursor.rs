use rune_cfg::RuneConfig;

use crate::layout::{CursorActivation, DynamicCursorConfig, DynamicCursorMode, RuntimeTuning};

use super::super::primitives::{pick_bool, pick_f32, pick_string, pick_u32, pick_u64, pick_u8};

pub(crate) fn load_cursor_section(cfg: &RuneConfig, out: &mut RuntimeTuning) {
    if let Some(theme) = pick_string(cfg, &["cursor.theme"]) {
        let theme = theme.trim();
        if !theme.is_empty() {
            out.cursor.theme = theme.to_string();
        }
    }
    out.cursor.size = pick_u32(cfg, &["cursor.size"], out.cursor.size);
    out.cursor.hide_while_typing = pick_bool(
        cfg,
        &[
            "cursor.hide-while-typing",
            "cursor.hide-when-typing",
            "cursor.hide_while_typing",
            "cursor.hide_when_typing",
        ],
        out.cursor.hide_while_typing,
    );
    out.cursor.hide_after_ms = pick_u64(
        cfg,
        &[
            "cursor.hide-after-ms",
            "cursor.hide-after-inactive-ms",
            "cursor.hide_after_ms",
            "cursor.hide_after_inactive_ms",
        ],
        out.cursor.hide_after_ms,
    );
    out.cursor.hide_on_keyboard_nav = pick_bool(
        cfg,
        &["cursor.hide-on-keyboard-nav", "cursor.hide_on_keyboard_nav"],
        out.cursor.hide_on_keyboard_nav,
    );
    load_dynamic_cursor_section(cfg, &mut out.cursor.dynamic);
}

fn pick_dynamic_mode(
    cfg: &RuneConfig,
    paths: &[&str],
    default: DynamicCursorMode,
) -> DynamicCursorMode {
    let Some(raw) = pick_string(cfg, paths) else {
        return default;
    };
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "none" => DynamicCursorMode::None,
        "rotate" | "stick" => DynamicCursorMode::Rotate,
        "tilt" => DynamicCursorMode::Tilt,
        "stretch" => DynamicCursorMode::Stretch,
        _ => default,
    }
}

fn pick_activation(
    cfg: &RuneConfig,
    paths: &[&str],
    default: CursorActivation,
) -> CursorActivation {
    let Some(raw) = pick_string(cfg, paths) else {
        return default;
    };
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "linear" => CursorActivation::Linear,
        "quadratic" => CursorActivation::Quadratic,
        "negative-quadratic" | "negative_quadratic" => CursorActivation::NegativeQuadratic,
        _ => default,
    }
}

fn load_dynamic_cursor_section(cfg: &RuneConfig, out: &mut DynamicCursorConfig) {
    let root = "cursor.dynamic";
    out.enabled = pick_bool(cfg, &[&format!("{root}.enabled")], out.enabled);
    out.mode = pick_dynamic_mode(cfg, &[&format!("{root}.mode")], out.mode);
    out.threshold_deg =
        pick_f32(cfg, &[&format!("{root}.threshold")], out.threshold_deg);
    out.ignore_warps = pick_bool(
        cfg,
        &[
            format!("{root}.ignore-warps").as_str(),
            format!("{root}.ignore_warps").as_str(),
        ],
        out.ignore_warps,
    );

    out.rotate.length = pick_f32(
        cfg,
        &[
            format!("{root}.rotate.length").as_str(),
            format!("{root}.rotate_length").as_str(),
        ],
        out.rotate.length,
    );
    out.rotate.offset_deg = pick_f32(
        cfg,
        &[
            format!("{root}.rotate.offset").as_str(),
            format!("{root}.rotate_offset").as_str(),
        ],
        out.rotate.offset_deg,
    );

    out.tilt.activation = pick_activation(
        cfg,
        &[
            format!("{root}.tilt.activation").as_str(),
            format!("{root}.tilt.function").as_str(),
        ],
        out.tilt.activation,
    );
    out.tilt.limit_px_s = pick_f32(
        cfg,
        &[
            format!("{root}.tilt.limit").as_str(),
            format!("{root}.tilt_limit").as_str(),
        ],
        out.tilt.limit_px_s,
    );
    out.tilt.window_ms = pick_u64(
        cfg,
        &[
            format!("{root}.tilt.window").as_str(),
            format!("{root}.tilt_window").as_str(),
        ],
        out.tilt.window_ms,
    );
    out.tilt.full_deg = pick_f32(
        cfg,
        &[
            format!("{root}.tilt.full").as_str(),
            format!("{root}.tilt_full").as_str(),
        ],
        out.tilt.full_deg,
    );

    out.stretch.activation = pick_activation(
        cfg,
        &[
            format!("{root}.stretch.activation").as_str(),
            format!("{root}.stretch.function").as_str(),
        ],
        out.stretch.activation,
    );
    out.stretch.limit_px_s = pick_f32(
        cfg,
        &[
            format!("{root}.stretch.limit").as_str(),
            format!("{root}.stretch_limit").as_str(),
        ],
        out.stretch.limit_px_s,
    );
    out.stretch.window_ms = pick_u64(
        cfg,
        &[
            format!("{root}.stretch.window").as_str(),
            format!("{root}.stretch_window").as_str(),
        ],
        out.stretch.window_ms,
    );

    let shake = format!("{root}.shake");
    out.shake.enabled = pick_bool(cfg, &[&format!("{shake}.enabled")], out.shake.enabled);
    out.shake.threshold =
        pick_f32(cfg, &[&format!("{shake}.threshold")], out.shake.threshold);
    out.shake.base = pick_f32(cfg, &[&format!("{shake}.base")], out.shake.base);
    out.shake.speed = pick_f32(cfg, &[&format!("{shake}.speed")], out.shake.speed);
    out.shake.influence =
        pick_f32(cfg, &[&format!("{shake}.influence")], out.shake.influence);
    out.shake.limit = pick_f32(cfg, &[&format!("{shake}.limit")], out.shake.limit);
    out.shake.timeout_ms = pick_u64(
        cfg,
        &[
            format!("{shake}.timeout").as_str(),
            format!("{shake}.timeout-ms").as_str(),
        ],
        out.shake.timeout_ms,
    );
    out.shake.effects = pick_bool(cfg, &[&format!("{shake}.effects")], out.shake.effects);
    out.shake.nearest = pick_u8(cfg, &[&format!("{shake}.nearest")], out.shake.nearest);
}

#[cfg(test)]
mod tests {
    use rune_cfg::RuneConfig;

    use crate::layout::{CursorActivation, DynamicCursorMode};
    use crate::layout::RuntimeTuning;

    use super::load_cursor_section;

    #[test]
    fn cursor_section_accepts_niri_style_hide_keys() {
        let cfg = RuneConfig::from_str(
            r#"
cursor:
  hide-when-typing false
  hide-after-inactive-ms 1500
end
"#,
        )
        .expect("cursor config should parse");

        let mut out = RuntimeTuning::default();
        load_cursor_section(&cfg, &mut out);

        assert!(!out.cursor.hide_while_typing);
        assert_eq!(out.cursor.hide_after_ms, 1500);
    }

    #[test]
    fn cursor_defaults_do_not_idle_hide() {
        assert_eq!(RuntimeTuning::default().cursor.hide_after_ms, 0);
    }

    #[test]
    fn cursor_hide_on_keyboard_nav_defaults_on_and_is_toggllable() {
        assert!(RuntimeTuning::default().cursor.hide_on_keyboard_nav);

        let cfg = RuneConfig::from_str(
            r#"
cursor:
  hide-on-keyboard-nav false
end
"#,
        )
        .expect("cursor config should parse");

        let mut out = RuntimeTuning::default();
        load_cursor_section(&cfg, &mut out);
        assert!(!out.cursor.hide_on_keyboard_nav);
    }

    #[test]
    fn dynamic_cursor_defaults_to_disabled() {
        let tuning = RuntimeTuning::default();
        assert!(!tuning.cursor.dynamic.enabled);
        assert_eq!(tuning.cursor.dynamic.mode, DynamicCursorMode::Tilt);
        assert!(tuning.cursor.dynamic.shake.enabled);
    }

    #[test]
    fn dynamic_cursor_section_parses_all_keys() {
        let cfg = RuneConfig::from_str(
            r#"
cursor:
  theme "Bibata"
  dynamic:
    enabled true
    mode "rotate"
    threshold 5
    ignore-warps false
    rotate:
      length 32
      offset 10.5
    end
    tilt:
      activation "quadratic"
      limit 2500
      window 150
      full 45
    end
    stretch:
      activation "linear"
      limit 1500
      window 80
    end
    shake:
      enabled false
      threshold 4.5
      base 3.0
      speed 6.0
      influence 1.5
      limit 8.0
      timeout 1500
      effects true
      nearest 2
    end
  end
end
"#,
        )
        .expect("dynamic cursor config should parse");

        let mut out = RuntimeTuning::default();
        load_cursor_section(&cfg, &mut out);

        let d = &out.cursor.dynamic;
        assert!(d.enabled);
        assert_eq!(d.mode, DynamicCursorMode::Rotate);
        assert_eq!(d.threshold_deg, 5.0);
        assert!(!d.ignore_warps);
        assert_eq!(d.rotate.length, 32.0);
        assert_eq!(d.rotate.offset_deg, 10.5);
        assert_eq!(d.tilt.activation, CursorActivation::Quadratic);
        assert_eq!(d.tilt.limit_px_s, 2500.0);
        assert_eq!(d.tilt.window_ms, 150);
        assert_eq!(d.tilt.full_deg, 45.0);
        assert_eq!(d.stretch.activation, CursorActivation::Linear);
        assert_eq!(d.stretch.limit_px_s, 1500.0);
        assert_eq!(d.stretch.window_ms, 80);
        assert!(!d.shake.enabled);
        assert_eq!(d.shake.threshold, 4.5);
        assert_eq!(d.shake.base, 3.0);
        assert_eq!(d.shake.speed, 6.0);
        assert_eq!(d.shake.influence, 1.5);
        assert_eq!(d.shake.limit, 8.0);
        assert_eq!(d.shake.timeout_ms, 1500);
        assert!(d.shake.effects);
        assert_eq!(d.shake.nearest, 2);
        assert_eq!(out.cursor.theme, "Bibata");
    }

    #[test]
    fn dynamic_cursor_ignores_unknown_mode_value() {
        let cfg = RuneConfig::from_str(
            r#"
cursor:
  dynamic:
    enabled true
    mode "wobble"
  end
end
"#,
        )
        .expect("dynamic cursor config should parse");

        let mut out = RuntimeTuning::default();
        load_cursor_section(&cfg, &mut out);
        assert_eq!(out.cursor.dynamic.mode, DynamicCursorMode::Tilt);
    }
}
