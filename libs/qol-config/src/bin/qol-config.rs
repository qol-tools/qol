use qol_config::contract::parse_spec;
use qol_config::normalized::resolve_config;
use qol_config::validation::validate_spec;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match args.first().map(String::as_str) {
        Some(command) => command,
        None => return Err(usage()),
    };
    if command == "validate" {
        return run_validate(&args[1..]);
    }
    if command == "normalize" {
        return run_normalize(&args[1..]);
    }
    Err(usage())
}

fn run_validate(args: &[String]) -> Result<(), String> {
    let plugin_root = parse_plugin_root(args)?;
    let spec = parse_spec(spec_path(&plugin_root)).map_err(format_parse_error)?;
    validate_spec(&spec).map_err(format_validation_errors)?;
    println!("valid");
    Ok(())
}

fn run_normalize(args: &[String]) -> Result<(), String> {
    let plugin_root = parse_plugin_root(args)?;
    let overrides_path = parse_optional_value(args, "--overrides");
    let pretty = args.iter().any(|arg| arg == "--pretty");
    let spec = parse_spec(spec_path(&plugin_root)).map_err(format_parse_error)?;
    let overrides = load_overrides(overrides_path.as_deref())?;
    let resolved = resolve_config(&spec, &overrides).map_err(format_validation_errors)?;
    if pretty {
        println!(
            "{}",
            serde_json::to_string_pretty(&resolved).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!(
        "{}",
        serde_json::to_string(&resolved).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn parse_plugin_root(args: &[String]) -> Result<PathBuf, String> {
    let value = match parse_optional_value(args, "--plugin-root") {
        Some(value) => value,
        None => return Err("missing --plugin-root".to_string()),
    };
    Ok(PathBuf::from(value))
}

fn parse_optional_value(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

fn spec_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join("qol-config.toml")
}

fn load_overrides(path: Option<&str>) -> Result<serde_json::Value, String> {
    let path = match path {
        Some(path) => path,
        None => return Ok(serde_json::Value::Null),
    };
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

fn format_parse_error(error: qol_config::contract::ParseSpecError) -> String {
    match error {
        qol_config::contract::ParseSpecError::Io(error) => error.to_string(),
        qol_config::contract::ParseSpecError::Toml(error) => error.to_string(),
    }
}

fn format_validation_errors(errors: Vec<qol_config::validation::ValidationError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn usage() -> String {
    "usage: qol-config <validate|normalize> --plugin-root <path> [--overrides <path>] [--pretty]"
        .to_string()
}
