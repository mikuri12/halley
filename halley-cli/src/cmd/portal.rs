use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use halley_api::Client;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalCommand {
    Status,
    Version,
}

pub fn run(command: PortalCommand, json: bool) -> ExitCode {
    let portal_path = find_in_path("xdg-desktop-portal-halley");
    let portal_version = portal_path
        .as_ref()
        .and_then(|path| run_version(path).ok())
        .unwrap_or_else(|| "(not found)".to_string());
    let compositor_version = Client::connect()
        .map(|client| client.server_info().compositor_version.clone())
        .map_err(|error| error.to_string());
    match command {
        PortalCommand::Version => print_version(json, portal_version, compositor_version),
        PortalCommand::Status => {
            print_status(json, portal_path, portal_version, compositor_version)
        }
    }
    ExitCode::SUCCESS
}

pub(super) fn parse(args: &[String]) -> Result<super::Action, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(super::Action::PortalHelp);
    };
    if matches!(command, "-h" | "--help") {
        return Ok(super::Action::PortalHelp);
    }
    let command = match command {
        "status" => PortalCommand::Status,
        "version" => PortalCommand::Version,
        other => return Err(format!("unknown portal command {other:?}")),
    };
    let mut json = false;
    for argument in &args[1..] {
        match argument.as_str() {
            "--json" if !json => json = true,
            "--json" => return Err("portal --json was specified more than once".to_string()),
            other => return Err(format!("unexpected portal argument {other:?}")),
        }
    }
    Ok(super::Action::Portal { command, json })
}

fn print_version(json: bool, portal: String, compositor: Result<String, String>) {
    let compositor = compositor.unwrap_or_else(|error| format!("unreachable: {error}"));
    if json {
        println!(
            "{}",
            serde_json::json!({
                "portal": portal,
                "halleyctl": env!("CARGO_PKG_VERSION"),
                "compositor": compositor,
            })
        );
    } else {
        println!("portal: {portal}");
        println!("halleyctl: {}", env!("CARGO_PKG_VERSION"));
        println!("halley: {compositor}");
    }
}

fn print_status(
    json: bool,
    portal_path: Option<PathBuf>,
    portal_version: String,
    compositor: Result<String, String>,
) {
    let backend = portal_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(not found in PATH)".to_string());
    let compositor = compositor
        .map(|version| format!("ok ({version})"))
        .unwrap_or_else(|error| format!("unreachable ({error})"));
    if json {
        println!(
            "{}",
            serde_json::json!({
                "backend": backend,
                "portal_version": portal_version,
                "compositor": compositor,
                "sources": ["screen", "window"],
                "cursor_modes": ["hidden", "embedded", "metadata"],
            })
        );
    } else {
        println!("backend: {backend}");
        println!("portal: {portal_version}");
        println!("compositor-ipc: {compositor}");
        println!("sources: screen, window");
        println!("cursor-modes: hidden, embedded, metadata");
    }
}

fn run_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(binary))
            .find(|candidate| candidate.is_file())
    })
}
