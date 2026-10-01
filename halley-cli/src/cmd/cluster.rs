use halley_api::{ClusterId, ClusterTarget};

use super::{Action, ClusterOutput};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterCommand {
    List {
        output: Option<String>,
    },
    Inspect {
        target: ClusterTarget,
        output: Option<String>,
    },
    LayoutCycle {
        output: Option<String>,
    },
    Slot {
        slot: u8,
        output: Option<String>,
    },
}

pub(super) fn parse(args: &[String]) -> Result<Action, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Action::ClusterHelp);
    };
    if matches!(command, "-h" | "--help") {
        return Ok(Action::ClusterHelp);
    }
    let (request, output) = match command {
        "list" => {
            let options = options(&args[1..], true)?;
            (
                ClusterCommand::List {
                    output: options.output,
                },
                ClusterOutput::List { json: options.json },
            )
        }
        "info" | "inspect" => {
            let (target, rest) = match args.get(1).map(String::as_str) {
                Some(value) if !value.starts_with('-') => (parse_target(value)?, &args[2..]),
                _ => (ClusterTarget::Current, &args[1..]),
            };
            let options = options(rest, true)?;
            (
                ClusterCommand::Inspect {
                    target,
                    output: options.output,
                },
                ClusterOutput::Info { json: options.json },
            )
        }
        "layout-cycle" | "layout" => {
            let options = options(&args[1..], false)?;
            (
                ClusterCommand::LayoutCycle {
                    output: options.output,
                },
                ClusterOutput::Ack,
            )
        }
        "slot" => {
            let raw = args
                .get(1)
                .ok_or_else(|| "cluster slot requires a number from 1 through 10".to_string())?;
            let slot = raw
                .parse::<u8>()
                .ok()
                .filter(|slot| (1..=10).contains(slot))
                .ok_or_else(|| format!("invalid cluster slot {raw:?}; expected 1 through 10"))?;
            let options = options(&args[2..], false)?;
            (
                ClusterCommand::Slot {
                    slot,
                    output: options.output,
                },
                ClusterOutput::Ack,
            )
        }
        other => return Err(format!("unknown cluster command {other:?}")),
    };
    Ok(Action::Cluster { request, output })
}

fn parse_target(value: &str) -> Result<ClusterTarget, String> {
    if value == "current" {
        return Ok(ClusterTarget::Current);
    }
    let value = value.strip_prefix("id:").unwrap_or(value);
    value
        .parse::<u64>()
        .map(ClusterId::new)
        .map(ClusterTarget::Id)
        .map_err(|_| format!("invalid cluster target {value:?}; expected current, ID, or id:ID"))
}

struct Options {
    output: Option<String>,
    json: bool,
}

fn options(args: &[String], allow_json: bool) -> Result<Options, String> {
    let mut output = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "cluster output option requires a connector name".to_string())?;
                if output.replace(value.clone()).is_some() {
                    return Err("cluster output option was specified more than once".to_string());
                }
            }
            "--json" if allow_json => json = true,
            "--json" => return Err("--json is valid only for cluster list and info".to_string()),
            value => return Err(format!("unexpected cluster argument {value:?}")),
        }
        index += 1;
    }
    Ok(Options { output, json })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_read_and_control_commands() {
        assert_eq!(
            parse(&args(&["list", "-o", "DP-1", "--json"])),
            Ok(Action::Cluster {
                request: ClusterCommand::List {
                    output: Some("DP-1".into())
                },
                output: ClusterOutput::List { json: true },
            })
        );
        assert_eq!(
            parse(&args(&["info", "id:7"])),
            Ok(Action::Cluster {
                request: ClusterCommand::Inspect {
                    target: ClusterTarget::Id(ClusterId::new(7)),
                    output: None,
                },
                output: ClusterOutput::Info { json: false },
            })
        );
        assert_eq!(
            parse(&args(&["slot", "10"])),
            Ok(Action::Cluster {
                request: ClusterCommand::Slot {
                    slot: 10,
                    output: None,
                },
                output: ClusterOutput::Ack,
            })
        );
        assert!(parse(&args(&["slot", "0"])).is_err());
        assert!(parse(&args(&["layout-cycle", "--json"])).is_err());
    }
}
