use halley_api::CaptureMode;

use super::Action;

pub(super) fn parse(args: &[String]) -> Result<Action, String> {
    let Some(mode) = args.first().map(String::as_str) else {
        return Ok(Action::CaptureHelp);
    };
    if matches!(mode, "-h" | "--help") {
        return Ok(Action::CaptureHelp);
    }
    let mode = match mode {
        "menu" => CaptureMode::Menu,
        "region" => CaptureMode::Region,
        "screen" => CaptureMode::Screen,
        "window" => CaptureMode::Window,
        other => return Err(format!("unknown capture mode {other:?}")),
    };
    let output = super::parse_output_option(&args[1..], "capture")?;
    Ok(Action::Capture { mode, output })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_capture_modes() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            parse(&args(&["region", "-o", "DP-1"])),
            Ok(Action::Capture {
                mode: CaptureMode::Region,
                output: Some("DP-1".into()),
            })
        );
        assert_eq!(
            parse(&args(&["menu"])),
            Ok(Action::Capture {
                mode: CaptureMode::Menu,
                output: None,
            })
        );
        assert!(parse(&args(&["pixels"])).is_err());
    }
}
