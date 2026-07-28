use super::CliArgs;
use anyhow::{anyhow, Result};
use qol_headless::{Command, Execution, HeadlessApp};

pub(super) fn execution(args: &CliArgs) -> Result<Option<Execution>> {
    let has_help = args.values.iter().any(|value| {
        value
            .to_str()
            .is_some_and(|value| matches!(value, "help" | "-h" | "--help"))
    });
    if !args.values.is_empty() && !has_help && !args.json {
        return Ok(None);
    }
    if !has_help
        && args.json
        && args.values.first().and_then(|value| value.to_str()) == Some("doctor")
    {
        return Ok(None);
    }

    let mut values = args
        .values
        .iter()
        .map(|value| {
            value
                .to_str()
                .map(ToString::to_string)
                .ok_or_else(|| anyhow!("help and JSON command paths must be valid UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        values.push("help".to_string());
    }
    for value in &mut values {
        if value == "-h" {
            *value = "--help".to_string();
        }
    }
    if args.json {
        values.insert(0, "--json".to_string());
    }

    let general_help = general_help_request(&values);
    let doctor_help = contextual_doctor_help(&values);
    let mut execution = app().execute(values);
    if general_help && execution.exit_code == qol_headless::EXIT_SUCCESS {
        execution.stdout = render_general_help();
    } else if doctor_help && execution.exit_code == qol_headless::EXIT_SUCCESS {
        execution.stdout = crate::commands::doctor::help_text().to_string();
    }
    Ok(Some(execution))
}

pub(super) fn general_help() -> String {
    render_general_help()
}

fn general_help_request(values: &[String]) -> bool {
    matches!(values, [value] if matches!(value.as_str(), "help" | "--help"))
}

fn contextual_doctor_help(values: &[String]) -> bool {
    let values = values
        .iter()
        .filter(|value| value.as_str() != "--json")
        .map(String::as_str)
        .collect::<Vec<_>>();
    matches!(values.as_slice(), ["help", "doctor"] | ["doctor", "help"])
}

fn render_general_help() -> String {
    app()
        .execute(["help".to_string()])
        .stdout
        .replace(
            "Global flags:\n  --json  Request structured JSON output from commands that support it.\n",
            "Global flags:\n  -v, --verbose     Show child command output.\n  -n, --no-plugins  qol dev: skip plugin rebuilds.\n  --json             Request structured JSON from commands that support it.\n  --                 Stop global option parsing.\n",
        )
}

fn app() -> HeadlessApp {
    HeadlessApp::new("qol", "qol")
        .about("Build, inspect, diagnose, and run the qol-tools workspace.")
        .command(command(
            "setup",
            "Build and install local qol development tooling.",
            "qol setup",
            "Updates local development binaries and repository-owned integration.",
            "Progress on stdout; diagnostics on stderr.",
            "Exits non-zero when setup cannot complete.",
        ))
        .command(command(
            "dev",
            "Run the development dashboard and tray.",
            "qol dev [worktree|--base] [--no-plugins]",
            "Use -v for child output and --no-plugins to skip plugin rebuilds.",
            "Interactive dashboard output.",
            "Exits non-zero when the development session cannot start.",
        ))
        .command(command(
            "env",
            "Manage disposable development environments.",
            "qol env <list|doctor|up|image|cancel|runs|down|shot|exec|drag>",
            "Environment subcommands own their operation-specific flags.",
            "Human-readable environment state and progress.",
            "Exits non-zero when discovery, validation, or an operation fails.",
        ))
        .command(command(
            "flow",
            "Run and inspect disposable environment workflows.",
            "qol flow <run|runs>",
            "Run accepts workflow, environment, repeat, job, and resource options.",
            "Human-readable workflow progress and reports.",
            "Exits non-zero when a workflow cannot be prepared or completed.",
        ))
        .command(command(
            "emu",
            "Discover, prepare, run, and control local emulators.",
            "qol emu <list|add|open|doctor|desktop|up|run|check|shot|key|insert|pull|snap|sh|exec|drag|down>",
            "Launch and control subcommands own their operation-specific flags.",
            "Human-readable emulator state, progress, and report paths.",
            "Exits non-zero when discovery, validation, launch, or control fails.",
        ))
        .command(command(
            "cat",
            "Render a source file or stdin with deterministic line numbers.",
            "qol cat [--no-less] [--plain|--color=auto|always|never] <path|->",
            "Paging and color default to terminal-aware automatic behavior.",
            "Rendered source on stdout; diagnostics on stderr.",
            "Exits non-zero when input cannot be read or rendered.",
        ))
        .command(command(
            "build",
            "Build the workspace or a named target.",
            "qol build [name]",
            "Use -v to show child command output.",
            "Build progress on stdout; diagnostics on stderr.",
            "Exits non-zero when the selected build fails.",
        ))
        .command(command(
            "check",
            "Run affected workspace checks.",
            "qol check [--staged]",
            "--staged checks the exact staged tree instead of the working tree.",
            "Check plan and command progress on stdout; diagnostics on stderr.",
            "Exits non-zero when planning or a selected check fails.",
        ))
        .command(command(
            "clean",
            "Clean workspace or named build artifacts.",
            "qol clean [name]",
            "Use -v to show child command output.",
            "Cleanup progress on stdout; diagnostics on stderr.",
            "Exits non-zero when cleanup fails.",
        ))
        .command(command(
            "install",
            "Install built qol applications and plugins.",
            "qol install",
            "Use -v to show child command output.",
            "Installation progress on stdout; diagnostics on stderr.",
            "Exits non-zero when installation fails.",
        ))
        .command(command(
            "sync",
            "Report the current source synchronization route.",
            "qol sync",
            "This command currently directs callers to make sync.",
            "Guidance on stderr.",
            "Exits non-zero because direct qol sync is not implemented.",
        ))
        .command(command(
            "trace",
            "Inspect a named runtime trace target.",
            "qol trace [name]",
            "Without a target, shows available trace guidance.",
            "Trace output on stdout; diagnostics on stderr.",
            "Exits non-zero when the trace target cannot run.",
        ))
        .command(command(
            "trace-rs",
            "Inspect the Rust runtime trace stream.",
            "qol trace-rs [options]",
            "Supports replay, filtering, detail, and marker options.",
            "Formatted trace events on stdout; diagnostics on stderr.",
            "Exits non-zero when the trace log cannot be read.",
        ))
        .doctor_provider(|| Ok(Vec::new()))
}

fn command(
    name: &str,
    about: &str,
    usage: &str,
    detail: &str,
    output: &str,
    exit_behavior: &str,
) -> Command {
    Command::new(name)
        .about(about)
        .usage(usage)
        .detail(detail)
        .output(output)
        .exit_behavior(exit_behavior)
}
