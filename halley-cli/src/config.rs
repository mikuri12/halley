use std::fmt::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use halley_api::{Client, ErrorKind};

pub fn edit(explicit: Option<PathBuf>) -> ExitCode {
    let path = match resolve_config_path(explicit) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("halleyctl: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = halley_config::bootstrap_default_config_at(&path) {
        eprintln!(
            "halleyctl: could not prepare config file {}: {error}",
            path.display()
        );
        return ExitCode::from(2);
    }
    let editor = match selected_editor() {
        Ok(editor) => editor,
        Err(error) => {
            eprintln!("halleyctl: {error}");
            return ExitCode::from(2);
        }
    };
    let mut command = Command::new(&editor[0]);
    command.args(&editor[1..]).arg(&path);
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("halleyctl: editor exited with {status}");
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .map(ExitCode::from)
                .unwrap_or(ExitCode::FAILURE)
        }
        Err(error) => {
            eprintln!("halleyctl: could not start editor {:?}: {error}", editor[0]);
            ExitCode::from(2)
        }
    }
}

pub fn verify(explicit: Option<PathBuf>) -> ExitCode {
    let path = match resolve_config_path(explicit) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("halleyctl: {error}");
            return ExitCode::from(2);
        }
    };

    match halley_config::load_runtime_config_diagnostic_at(&path) {
        Ok(_) => {
            println!("Configuration valid");
            println!("  File: {}", path.display());
            ExitCode::SUCCESS
        }
        Err(diagnostic) => {
            eprint!("{}", format_diagnostic(&diagnostic));
            ExitCode::FAILURE
        }
    }
}

pub fn migrate(explicit: Option<PathBuf>, dry_run: bool) -> ExitCode {
    let path = match resolve_config_path(explicit) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("halleyctl: {error}");
            return ExitCode::from(2);
        }
    };
    match halley_config::migrate_config_at(&path, dry_run) {
        Ok(report) => {
            use halley_config::MigrationStatus;
            match report.status {
                MigrationStatus::UpToDate => {
                    println!("No structural migration needed");
                    println!("  File: {}", path.display());
                }
                MigrationStatus::WouldUpdate | MigrationStatus::Updated => {
                    println!(
                        "Configuration {}",
                        if dry_run {
                            "migration preview"
                        } else {
                            "migrated"
                        }
                    );
                    println!("  File: {}", path.display());
                    if let Some(reason) = &report.reason {
                        println!("  Reason: {reason}");
                    }
                    for item in &report.applied {
                        println!("  Change: {item}");
                    }
                    if let Some(backup) = report.backup {
                        println!("  Backup: {}", backup.display());
                    }
                }
                MigrationStatus::Replaced => {
                    println!("Configuration replaced");
                    println!("  File: {}", path.display());
                    if let Some(reason) = &report.reason {
                        println!("  Reason: {reason}");
                    }
                    for item in &report.applied {
                        println!("  Change: {item}");
                    }
                    if let Some(backup) = report.backup {
                        println!("  Backup: {}", backup.display());
                    }
                }
            }
            for item in &report.skipped {
                println!("  Skip: {item}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("halleyctl: could not migrate {}: {error}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn resolve_config_path(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    match explicit {
        Some(path) => match absolute_path(path) {
            Ok(path) => Ok(path),
            Err(error) => Err(format!("could not resolve config path: {error}")),
        },
        None => discover_config_path(),
    }
}

fn selected_editor() -> Result<Vec<String>, String> {
    for name in ["VISUAL", "EDITOR"] {
        if let Some(value) = std::env::var_os(name) {
            let value = value
                .into_string()
                .map_err(|_| format!("${name} contains non-Unicode text"))?;
            if value.trim().is_empty() {
                continue;
            }
            return parse_editor(name, &value);
        }
    }
    Ok(vec!["vi".to_string()])
}

fn parse_editor(name: &str, value: &str) -> Result<Vec<String>, String> {
    shlex::split(value)
        .filter(|arguments| !arguments.is_empty())
        .ok_or_else(|| format!("${name} is not a valid editor command"))
}

fn absolute_path(path: PathBuf) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn discover_config_path() -> Result<PathBuf, String> {
    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) if error.kind() == ErrorKind::Connection => return default_config_path(),
        Err(error) => return Err(format!("could not connect to the compositor: {error}")),
    };
    match client.config_path().map_err(|error| {
        format!(
            "the running compositor cannot report its config path ({error}); pass `-c PATH` explicitly"
        )
    })? {
        Some(path) => Ok(path),
        None => Err("the running compositor has no selected config path; pass `-c PATH`".to_string()),
    }
}

fn default_config_path() -> Result<PathBuf, String> {
    halley_config::config_path()
        .ok_or_else(|| "HOME and XDG_CONFIG_HOME are unset; pass `-c PATH`".to_string())
        .and_then(|path| {
            absolute_path(path)
                .map_err(|error| format!("could not resolve default config: {error}"))
        })
}

fn format_diagnostic(diagnostic: &halley_config::ConfigDiagnostic) -> String {
    let mut formatted = String::from("Configuration invalid\n");
    if let Some(path) = diagnostic.path.as_deref() {
        writeln!(formatted, "  File: {}", path.display()).unwrap();
    }
    match (diagnostic.line, diagnostic.column) {
        (Some(line), Some(column)) => {
            writeln!(formatted, "  Location: line {line}, column {column}").unwrap();
        }
        (Some(line), None) => {
            writeln!(formatted, "  Location: line {line}").unwrap();
        }
        (None, Some(column)) => {
            writeln!(formatted, "  Location: column {column}").unwrap();
        }
        (None, None) => {}
    }
    writeln!(formatted, "  Error: {}", diagnostic.message).unwrap();
    if let (Some(line), Some(source)) = (diagnostic.line, diagnostic.source_line.as_deref()) {
        let width = line.to_string().len();
        writeln!(formatted, "  {line:>width$} | {source}").unwrap();
        if let Some(column) = diagnostic.column {
            let caret_padding = " ".repeat(column.saturating_sub(1));
            writeln!(formatted, "  {:>width$} | {caret_padding}^", "").unwrap();
        }
    }
    if let Some(hint) = diagnostic.hint.as_deref() {
        writeln!(formatted, "  Hint: {hint}").unwrap();
    }
    if let Some(code) = diagnostic.code {
        writeln!(formatted, "  Code: E{code}").unwrap();
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_diagnostic_omits_unavailable_fields() {
        let diagnostic = halley_config::ConfigDiagnostic {
            path: Some("/tmp/halley.rune".into()),
            line: Some(12),
            column: Some(4),
            message: "bad value".to_string(),
            source_line: Some("  value nope".to_string()),
            hint: Some("use yes".to_string()),
            code: Some(7),
        };
        let formatted = format_diagnostic(&diagnostic);
        assert!(formatted.contains("File: /tmp/halley.rune"));
        assert!(formatted.contains("Location: line 12, column 4"));
        assert!(formatted.contains("12 |   value nope"));
        assert!(formatted.contains("|    ^"));
        assert!(formatted.contains("Hint: use yes"));
        assert!(formatted.contains("Code: E7"));

        let semantic =
            halley_config::ConfigDiagnostic::message(None, "semantic failure".to_string());
        let formatted = format_diagnostic(&semantic);
        assert!(!formatted.contains("File:"));
        assert!(!formatted.contains("Location:"));
    }

    #[test]
    fn editor_command_supports_arguments_and_quotes() {
        assert_eq!(
            parse_editor("EDITOR", "code --wait --name 'Halley config'").unwrap(),
            ["code", "--wait", "--name", "Halley config"]
        );
        assert!(parse_editor("EDITOR", "editor '").is_err());
    }
}
