use halley_api::{TrailDirection, TrailTarget};

use super::{Action, TrailOutput};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrailCommand {
    Navigate {
        direction: TrailDirection,
        output: Option<String>,
    },
    List {
        output: Option<String>,
    },
    Goto {
        target: TrailTarget,
        output: Option<String>,
    },
}

pub(super) fn parse(args: &[String]) -> Result<Action, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Action::TrailHelp);
    };
    if command == "-h"
        || command == "--help"
        || args[1..].iter().any(|arg| arg == "-h" || arg == "--help")
    {
        return Ok(Action::TrailHelp);
    }
    match command {
        "prev" | "previous" | "next" => {
            let (output, json) = parse_flags(&args[1..])?;
            if json {
                return Err("--json is supported only by trail list".to_string());
            }
            Ok(Action::Trail {
                request: TrailCommand::Navigate {
                    direction: if command == "next" {
                        TrailDirection::Next
                    } else {
                        TrailDirection::Previous
                    },
                    output,
                },
                output: TrailOutput::Ack,
            })
        }
        "list" => {
            let (output, json) = parse_flags(&args[1..])?;
            Ok(Action::Trail {
                request: TrailCommand::List { output },
                output: TrailOutput::List { json },
            })
        }
        "goto" => {
            let raw = args
                .get(1)
                .ok_or_else(|| "trail goto requires an index or node selector".to_string())?;
            if raw.starts_with('-') {
                return Err("trail goto requires its target before options".to_string());
            }
            let target = match raw.parse::<usize>() {
                Ok(index) => TrailTarget::Index(index),
                Err(_) => TrailTarget::Selector(super::node::parse_selector(raw)?),
            };
            let (output, json) = parse_flags(&args[2..])?;
            if json {
                return Err("--json is supported only by trail list".to_string());
            }
            Ok(Action::Trail {
                request: TrailCommand::Goto { target, output },
                output: TrailOutput::Ack,
            })
        }
        other => Err(format!("unknown trail command {other:?}")),
    }
}

fn parse_flags(args: &[String]) -> Result<(Option<String>, bool), String> {
    let mut output = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "-o" | "--output" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "trail output option requires a connector name".to_string())?;
                if output.replace(value.clone()).is_some() {
                    return Err("trail output option was specified more than once".to_string());
                }
            }
            value => return Err(format!("unexpected trail argument {value:?}")),
        }
        index += 1;
    }
    Ok((output, json))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_navigation_list_and_goto() {
        assert_eq!(
            parse(&args(&["prev", "-o", "DP-1"])),
            Ok(Action::Trail {
                request: TrailCommand::Navigate {
                    direction: TrailDirection::Previous,
                    output: Some("DP-1".into()),
                },
                output: TrailOutput::Ack,
            })
        );
        assert_eq!(
            parse(&args(&["list", "--json"])),
            Ok(Action::Trail {
                request: TrailCommand::List { output: None },
                output: TrailOutput::List { json: true },
            })
        );
        assert_eq!(
            parse(&args(&["goto", "3"])),
            Ok(Action::Trail {
                request: TrailCommand::Goto {
                    target: TrailTarget::Index(3),
                    output: None,
                },
                output: TrailOutput::Ack,
            })
        );
        assert!(parse(&args(&["goto", "bogus"])).is_err());
    }
}
