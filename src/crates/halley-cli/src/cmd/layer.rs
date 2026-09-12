use halley_api::{LayerRequest, Request};

use crate::help::HelpTopic;
use crate::parse::{ParseOutcome, UsageError, contains_help_flag, parse_output_option};

pub(crate) fn parse_layer_request(args: &[String]) -> Result<ParseOutcome, UsageError> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => Ok(ParseOutcome::Help(HelpTopic::Layer)),
        Some("list") => parse_layer_list(&args[1..]),
        Some("promote") => parse_layer_promote(&args[1..]),
        Some("demote") => parse_layer_demote(&args[1..]),
        Some(other) => Err(UsageError::new(
            format!("unknown layer command: {other}"),
            HelpTopic::Layer,
        )),
    }
}

fn parse_layer_list(args: &[String]) -> Result<ParseOutcome, UsageError> {
    if contains_help_flag(args) {
        return Ok(ParseOutcome::Help(HelpTopic::LayerList));
    }
    let output = parse_output_option(args, HelpTopic::LayerList)?;
    Ok(ParseOutcome::Request(Request::Layer(LayerRequest::List {
        output,
    })))
}

fn parse_layer_promote(args: &[String]) -> Result<ParseOutcome, UsageError> {
    if contains_help_flag(args) {
        return Ok(ParseOutcome::Help(HelpTopic::LayerPromote));
    }
    let handle = parse_layer_handle(args)?;
    Ok(ParseOutcome::Request(Request::Layer(LayerRequest::Promote {
        handle,
    })))
}

fn parse_layer_demote(args: &[String]) -> Result<ParseOutcome, UsageError> {
    if contains_help_flag(args) {
        return Ok(ParseOutcome::Help(HelpTopic::LayerDemote));
    }
    let handle = parse_layer_handle(args)?;
    Ok(ParseOutcome::Request(Request::Layer(LayerRequest::Demote {
        handle,
    })))
}

fn parse_layer_handle(args: &[String]) -> Result<u64, UsageError> {
    let Some(raw) = args.first() else {
        return Err(UsageError::new(
            "missing HANDLE argument".to_string(),
            HelpTopic::Layer,
        ));
    };
    raw.parse::<u64>().map_err(|_| {
        UsageError::new(
            format!("invalid HANDLE: {raw} (expected a number from `layer list`)"),
            HelpTopic::Layer,
        )
    })
}
