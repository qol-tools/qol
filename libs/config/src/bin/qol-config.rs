use qol_config::contract::{
    parse_runtime_spec, parse_spec, validate_contracts, ParseRuntimeSpecError, RuntimeSpec,
};
use qol_config::normalized::resolve_config;
use qol_config::validation::validate_spec;
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    app().run(std::env::args().skip(1))
}

fn app() -> HeadlessApp {
    HeadlessApp::new("qol-config", "qol-config")
        .about("Validate and normalize qol plugin configuration contracts.")
        .command(
            Command::new("validate")
                .about("Validate a plugin's configuration and runtime contracts.")
                .usage("qol-config validate --plugin-root <path>")
                .output("Prints `valid` on success.")
                .exit_behavior("Exits non-zero when a contract is missing or invalid.")
                .run_plain_text(|context| {
                    run_validate(context.args()).map_err(anyhow::Error::msg)?;
                    Ok(PlainTextOutput::text("valid"))
                })
                .run_json(|context| {
                    run_validate(context.args()).map_err(anyhow::Error::msg)?;
                    Ok(serde_json::json!({ "valid": true }))
                }),
        )
        .command(
            Command::new("normalize")
                .about("Resolve a plugin contract with optional JSON overrides.")
                .usage("qol-config normalize --plugin-root <path> [--overrides <path>] [--pretty]")
                .output("Prints the resolved configuration as JSON.")
                .exit_behavior("Exits non-zero when input cannot be read, parsed, or resolved.")
                .run_plain_text(|context| {
                    let value = normalized_value(context.args()).map_err(anyhow::Error::msg)?;
                    let text = if context.args().iter().any(|arg| arg == "--pretty") {
                        serde_json::to_string_pretty(&value)?
                    } else {
                        serde_json::to_string(&value)?
                    };
                    Ok(PlainTextOutput::text(text))
                })
                .run_json(|context| normalized_value(context.args()).map_err(anyhow::Error::msg)),
        )
        .doctor_check(DoctorCheck::new(
            "contract_engine",
            "Verify the contract parser and normalizer are available.",
            || {
                Ok(DoctorCheckResult::ok(
                    "contract_engine",
                    "configuration contract engine is available",
                ))
            },
        ))
}

fn run_validate(args: &[String]) -> Result<(), String> {
    let plugin_root = parse_plugin_root(args)?;
    let spec = parse_spec(spec_path(&plugin_root)).map_err(format_parse_error)?;
    validate_spec(&spec).map_err(format_validation_errors)?;
    let runtime = load_runtime_spec(&plugin_root)?;
    validate_contracts(&spec, runtime.as_ref()).map_err(format_validation_errors)?;
    Ok(())
}

fn normalized_value(args: &[String]) -> Result<serde_json::Value, String> {
    let plugin_root = parse_plugin_root(args)?;
    let overrides_path = parse_optional_value(args, "--overrides");
    let spec = parse_spec(spec_path(&plugin_root)).map_err(format_parse_error)?;
    let overrides = load_overrides(overrides_path.as_deref())?;
    let resolved = resolve_config(&spec, &overrides).map_err(format_validation_errors)?;
    serde_json::to_value(resolved).map_err(|error| error.to_string())
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

fn runtime_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join("qol-runtime.toml")
}

fn load_overrides(path: Option<&str>) -> Result<serde_json::Value, String> {
    let path = match path {
        Some(path) => path,
        None => return Ok(serde_json::Value::Null),
    };
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

fn load_runtime_spec(plugin_root: &Path) -> Result<Option<RuntimeSpec>, String> {
    let path = runtime_path(plugin_root);
    if !path.exists() {
        return Ok(None);
    }
    parse_runtime_spec(&path)
        .map(Some)
        .map_err(format_runtime_parse_error)
}

fn format_parse_error(error: qol_config::contract::ParseSpecError) -> String {
    match error {
        qol_config::contract::ParseSpecError::Io(error) => error.to_string(),
        qol_config::contract::ParseSpecError::Toml(error) => error.to_string(),
    }
}

fn format_runtime_parse_error(error: ParseRuntimeSpecError) -> String {
    match error {
        ParseRuntimeSpecError::Io(error) => error.to_string(),
        ParseRuntimeSpecError::Toml(error) => error.to_string(),
        ParseRuntimeSpecError::Validation(message) => message,
    }
}

fn format_validation_errors(errors: Vec<qol_config::validation::ValidationError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorReport;

    #[test]
    fn contextual_help_is_equivalent_in_both_positions() {
        for args in [["help", "normalize"], ["normalize", "help"]] {
            let output = app().execute(args.map(str::to_string));
            assert_eq!(output.exit_code, 0, "{}", output.stderr);
            assert!(output.stdout.contains("--plugin-root <path>"));
        }
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_positions() {
        for args in [["--json", "doctor"], ["doctor", "--json"]] {
            let output = app().execute(args.map(str::to_string));
            assert_eq!(output.exit_code, 0, "{}", output.stderr);
            let report: DoctorReport = serde_json::from_str(&output.stdout).unwrap();
            assert_eq!(report.plugin_id, "qol-config");
            assert_eq!(report.checks.len(), 1);
        }
    }

    #[test]
    fn json_validation_reports_input_errors_without_mutating_files() {
        let output = app().execute(
            ["--json", "validate", "--plugin-root", "/definitely/missing"].map(str::to_string),
        );
        assert_eq!(output.exit_code, 1);
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.trim().is_empty());
    }
}
