use super::CliArgs;
use anyhow::{anyhow, Result};
use qol_headless::{Command, Execution, HeadlessApp};

use crate::commands::sessions;

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
    if !has_help
        && args.json
        && args.values.first().and_then(|value| value.to_str()) == Some("sessions")
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
    let agent_tools = sessions::tool_names();
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
            "Install built qol applications and plugins. --dev builds with the dev feature and installs a dev-mode runtime.",
            "qol install",
            "Use -v to show child command output.",
            "Installation progress on stdout; diagnostics on stderr.",
            "Exits non-zero when installation fails.",
        ))
        .command(
            command(
                "sync",
                "Sync the active profile with its configured cloud repository.",
                "qol sync",
                "Pulls the latest profile from the configured git repository, merges changes field-level, and pushes local changes back. Conflicts keep local data and write a backup.",
                "Human summary on stdout; diagnostics on stderr.",
                "Exits non-zero when sync cannot complete or conflicts need review.",
            )
            .run_json(|context| {
                if !context.args().is_empty() {
                    return Err(anyhow!("usage: qol sync"));
                }
                crate::commands::sync::run_json()
            }),
        )
        .command(
            command(
                "sessions",
                "Bridge work between independent terminal sessions.",
                &format!(
                    "qol sessions <{}>",
                    sessions::SUBCOMMANDS
                        .iter()
                        .map(|entry| entry.name)
                        .collect::<Vec<_>>()
                        .join("|")
                ),
                "Use list to discover a stable token, then bridge to submit and await one bounded implementation task.",
                "Session rows or bridge JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when discovery, identity, capability, validation, or delivery fails.",
            )
            .detail(format!("The agent surface is {agent_tools}."))
            .detail("spawn launches a tagged harness for a registered tool or reuses its live match under the same key.")
            .detail("bridge owns submission, completion signalling, waiting, and result delivery.")
            .detail("next prints the exact command for each open round; resume re-attaches to a pending round and waits without submitting.")
            .detail("read, send, wait, and focus remain human diagnostics.")
            .detail("export renders a per-client agent surface from the shared tool contract.")
            .subcommand(command(
                "list",
                "Discover live terminal sessions.",
                "qol sessions list [--json]",
                "Returns stable session tokens with current directory, display identity, activity hint, and capabilities.",
                "Session rows on stdout.",
                "Exits non-zero when discovery fails.",
            ))
            .subcommand(command(
                "capability",
                "Report whether a lane can be spawned, optionally at a named tier.",
                "qol sessions capability [--tier TOKEN]",
                "Answers whether spawning a lane is possible right now: lane_spawn is true when a registered tool is installed and a model is resolvable, and --tier TOKEN narrows that to models whose id carries the token, so a caller asks for a tier without naming a vendor or a model. Each tool row reports its launch program, whether that program is installed, the models its catalog lists, and the subset matching the tier. A tool whose harness exposes no model catalog reports no models and answers only the untiered question.",
                "Capability JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when a flag is invalid or the spawn model config cannot be read.",
            ))
            .subcommand(command(
                "spawn",
                "Launch a tagged tool session or reuse its live match.",
                "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] [--model MODEL] [--title TITLE] [--task TASK] [--no-resume]",
                "Launches a tagged harness for a registered tool in a new tab, or reuses the single live session already carrying the key when its tool matches. The result JSON reports the live session token, tool, key, reused, cwd, surface, model, and title. A key spanning tools conflicts, multiple matches are ambiguous, and the CLI generates a key when --key is omitted. The surface default comes from spawn_surface in ~/.config/qol-tray/sessions.toml, then tab; an explicit --model overrides the spawned session's model, with spawn_model in the same file as the fallback. --title names the new tab (the lane key by default); --task delivers the first round at spawn time so the round is already open when the command returns. Resume is automatic when the spawn ledger holds a session id for the key (same tool and cwd); --no-resume opts out, and the spawn JSON reports resume and resume_detail.",
                "Spawn JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on orchestration, identity, capability, readiness, or task delivery failure.",
            ))
            .subcommand(command(
                "fork",
                "Launch a detached architect that never reports back.",
                "qol sessions fork --tool TOOL --cwd PATH --key KEY --model MODEL (--brief TEXT | --brief-file PATH) [--effort LEVEL] [--title TITLE] [--surface tab|os-window] [--parent SESSION]",
                "Launches a new terminal that owns the brief end to end and reports to the user in its own terminal, not back to the caller. Use it when a second problem surfaces mid-session and chasing it would cost the thread already being held. A fork is the root of a new tree, not a lane: no round is opened on it, no completion signal is embedded in its launch, and bridge refuses it by name. The brief is written to a file under the sessions data dir and the launch points the fork at that path, so a long problem statement survives argv limits and stays readable after the screen scrolls. --model is required so a fork launches at a deliberately chosen tier rather than inheriting the caller's, and --effort is passed to tools that take one (claude: low, medium, high, xhigh, max). A key already held by a live session is refused because a fork always starts fresh.",
                "Fork JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on validation, key conflict, readiness, or launch failure.",
            ))
            .subcommand(command(
                "forks",
                "List the detached forks recorded on this host.",
                "qol sessions forks",
                "Prints one row per recorded fork: key, tool, tier, session token, and the path to its brief. Forks are never collected, so this listing is the only link back to a tree that was deliberately cut loose.",
                "Fork rows on stdout; diagnostics on stderr.",
                "Exits non-zero when the fork directory cannot be read.",
            ))
            .subcommand(command(
                "submit",
                "Deliver one bounded task and return with the round open.",
                "qol sessions submit <session> --task TASK [--acknowledge-marker TEXT]",
                "Submits exactly once with a generated completion signal and returns immediately with the round recorded and open, so several lanes can run in parallel before any of them is awaited. Refuses when a round is already pending on that session; pass the reviewed completion_marker as --acknowledge-marker to start the next round. Wait for the completion with `qol sessions bridge <session>` (no task) or resume.",
                "Submit JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on validation or delivery failure; the round is open only when delivery is observed.",
            ))
            .subcommand(command(
                "bridge",
                "Submit and await one bounded implementation task.",
                "qol sessions bridge <session> [<task...>] [--timeout-ms N]",
                "Submits exactly once, waits for a generated completion signal, and returns completed, session, completion_marker, screen, reads, and elapsed_ms as JSON. Without a task it re-attaches to the pending round and waits for its completion marker. Timeout defaults to 24h.",
                "Bridge JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on validation or delivery failure; a timeout returns completed=false.",
            ))
            .subcommand(command(
                "next",
                "Print the exact next command for each open bridge round.",
                "qol sessions next [<session>] [--json]",
                "Reads the durable per-session bridge state: a waiting round prints its resume command; a round whose target went idle without its completion signal prints resume --kickstart; a round whose target's terminal is gone prints discard; a completed round prints a review instruction with the acknowledge-marker bridge template; no rounds prints phase=idle.",
                "Round phases and commands on stdout.",
                "Exits non-zero when the bridge state cannot be read.",
            ))
            .subcommand(command(
                "discard",
                "Drop the checkpoint of a round whose terminal is gone.",
                "qol sessions discard <session>",
                "Removes the pending-bridge checkpoint of a session that no longer has a live terminal (verified via discovery); it refuses a live session, refuses when no checkpoint exists, and never touches last-send state or spawn locks. The session token comes from `qol sessions next`, which prints phase=gone with the exact discard command for orphaned rounds.",
                "Removal confirmation on stdout; diagnostics on stderr.",
                "Exits non-zero when the session is live, has no checkpoint, or discovery fails.",
            ))
            .subcommand(command(
                "resume",
                "Re-attach to the pending bridge round and await its completion.",
                "qol sessions resume <session> [--timeout-ms N] [--kickstart]",
                "Waits for the recorded completion marker without submitting anything and returns the same JSON as bridge with submitted=false. --kickstart first nudges an interrupted session to continue or emit the signal. An idle target returns stalled=true instead of blocking until timeout. Timeout defaults to 24h.",
                "Bridge JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when no round is pending; a timeout returns completed=false.",
            ))
            .subcommand(command(
                "interrupt",
                "Send the target tool's stop key while a bridge round is open.",
                "qol sessions interrupt <session>",
                "Resolves the per-tool stop gesture (agent TUIs: esc, plain shells: ctrl+c) and delivers it as a key event, never as text. The round and any queued input stay intact; follow with `qol sessions next`.",
                "Confirmation on stdout; diagnostics on stderr.",
                "Exits non-zero when no round is open or delivery fails.",
            ))
            .subcommand(
                command(
                    "mcp",
                    "Serve the session tools over stdio as a Model Context Protocol server.",
                    "qol sessions mcp",
                    &format!("One JSON-RPC 2.0 message per line (protocol 2025-03-26); tools are {agent_tools}. session_spawn takes optional title, model, and task; session_submit delivers a task without waiting. A bridge submits once and waits for the generated completion signal before returning; loop closure records an explicit accepted or paused transition. The round envelope is generated server-side from the target's durable role record (lane marker written at spawn; absent means architect): bridging a non-lane session is an architect-receiver round - the receiver may accept the request into its own loop or decline with a reason, and returns the completion fragments either way. The caller never chooses the receiver's role."),
                    "Protocol responses on stdout.",
                    "Exits zero on EOF.",
                )
                .run_plain_text(|context| {
                    let mut args = Vec::with_capacity(context.args().len() + 1);
                    args.push(std::ffi::OsString::from("mcp"));
                    args.extend(context.args().iter().map(std::ffi::OsString::from));
                    crate::commands::sessions::run(
                        &args,
                        qol_headless::OutputFormat::PlainText,
                    )?;
                    Ok(qol_headless::PlainTextOutput::empty())
                }),
            ),
        )
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
