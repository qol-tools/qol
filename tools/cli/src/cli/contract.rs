use super::CliArgs;
use anyhow::{anyhow, Result};
use qol_headless::{Command, Execution, HeadlessApp};

use crate::commands::{agents, peers, sessions};

pub(super) fn execution(args: &CliArgs) -> Result<Option<Execution>> {
    let has_help = args
        .values
        .iter()
        .take_while(|value| value.to_str() != Some("--"))
        .any(|value| {
            value
                .to_str()
                .is_some_and(|value| matches!(value, "help" | "-h" | "--help"))
        });
    let is_agents = args.values.first().and_then(|value| value.to_str()) == Some("agents");
    let is_peers = args.values.first().and_then(|value| value.to_str()) == Some("peers");
    if !is_agents && !is_peers && !args.values.is_empty() && !has_help && !args.json {
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
        .command(peers::command())
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
        .command(
            command(
                "check",
                "Run affected workspace checks.",
                "qol check [--staged|--lint] [--base REV] [--report PATH] [--format-owned PATH]",
                "--staged checks the exact staged tree instead of the working tree. --base REV pins the comparison base for affected planning. --report PATH publishes the machine report. Repeatable --format-owned PATH formats exactly the named Rust source files before full worktree checks.",
                "Check plan and command progress on stdout; diagnostics on stderr.",
                "Exits non-zero when planning, formatting, or a selected check fails, or when the source changes mid-run.",
            )
            .detail("--lint runs clippy only over uncommitted changes plus dependents, with its own lint cache.")
            .detail("--format-owned validates every named file and the formatter before any write, never follows modules, and records before/after hashes, keeping completed rewrites when a later file fails.")
            .detail("A --report destination is refused when it is a tracked source path, a symlink, a directory, or an existing file that is not a prior qol-check report.")
            .detail("Publication is recorded in the run-directory report and starts as pending until the external rename succeeds, and a failed publication never leaves a verified report behind.")
            .detail("Full checks fingerprint the source before and after, and a mismatch marks the run stale instead of passing."),
        )
        .command(
            command(
                "clean",
                "Clean workspace or named build artifacts, or prune cargo caches no build still uses.",
                crate::commands::clean::USAGE,
                "Use -v to show child command output. --prune keeps every cache a build used in the last 3 days, and what those builds depend on, and removes the rest; --dry-run only reports what it would free. Without dirs it prunes the workspace target dir and its cargo build dir.",
                "Cleanup progress on stdout, or with --prune --json the per-directory sweep reports; diagnostics on stderr.",
                "Exits non-zero when cleanup fails or a pruned cache entry cannot be removed.",
            )
            .detail("A prune skips a profile while cargo builds in it and never touches the qol-check, qol-env, qol-dev, qol-lint, and sherpa-onnx-prebuilt roots.")
            .detail("On a disk that does not record file reads, a prune removes only caches cargo can no longer use: replaced incremental sessions and outputs without a fingerprint.")
            .run_json(|context| crate::commands::clean::prune_json(context.args())),
        )
        .command(command(
            "install",
            "Install built qol applications and plugins. --dev builds only the development tray and installs it beside the production install, in its own directory. It leaves the plugins folder and the workspace build record to the production install.",
            "qol install",
            "Use -v to show child command output.",
            "Installation progress on stdout; diagnostics on stderr.",
            "Exits non-zero when installation fails.",
        ))
        .command(
            command(
                "pr",
                "Run a pull request's CI build in the installed tray.",
                crate::commands::pr::USAGE,
                "Downloads the Linux build the live test workflow uploaded for the pull request's head commit, so a live test needs no local build.",
                "Progress on stdout; diagnostics on stderr.",
                "Exits non-zero when the build is missing, still running, or for another commit, or when a swap fails.",
            )
            .subcommand(command(
                "try",
                "Install a pull request's plugin builds, or put the installed ones back.",
                crate::commands::pr::USAGE,
                "try <number> downloads the live-test-build-linux-x86_64 artifact of the newest successful live test run at the pull request's head, checks that its manifest names that pull request and commit, and that each binary is a production build of its plugin. It backs up each installed plugin binary once, swaps the build in atomically, and stops the plugin's daemon so the tray starts the new one. Plugins not installed here are skipped, and a tray build is left in the download folder. try --off puts every backed-up binary back and restarts those daemons. Only one pull request is installed at a time.",
                "One line per plugin on stdout; diagnostics on stderr.",
                "Exits non-zero when gh fails, the run is missing, unfinished or failed, the manifest does not match, or a swap or restore fails.",
            )),
        )
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
            .detail("Agent assignment binds a dispatch to a named agent_profiles entry with a closed role and explicit image_input or visual_review requirements; configuring any profile enables enforcement unless enforce_agent_profiles is false, and the profile's declared tool and model must match the launch while allowed_models still governs spending.")
            .detail("bridge owns submission, completion signalling, waiting, and result delivery.")
            .detail("next prints the exact command for each open round; resume re-attaches to a pending round and waits without submitting.")
            .detail("read, send, wait, and focus remain human diagnostics.")
            .detail("export renders a per-client agent surface from the shared tool contract.")
            .subcommand(command(
                "list",
                "Discover live terminal sessions.",
                "qol sessions list [--json]",
                "Returns stable session tokens with current directory, display identity, activity hint, and capabilities. A row ends with the session's Remote Control link, or - when Remote Control is off, and the GitHub pull requests it linked in session_send messages, or -.",
                "Session rows on stdout.",
                "Exits non-zero when discovery fails.",
            ))
            .subcommand(command(
                "status",
                "Print one line per keyed session and today's closed forks.",
                "qol sessions status",
                "Prints one tab-separated line per live session with a spawn key, then one per fork recorded today that is no longer live: key, attention state from the CLI Sessions panel (closed for a fork that is not live), remote control on with its claude.ai link or off, the pull requests it linked in session_send messages, and the first 80 characters of the last message it sent with session_send. It reads the sessions data dir and spends no model turn.",
                "Status lines on stdout; diagnostics on stderr.",
                "Exits non-zero when discovery fails or the sessions data dir cannot be read.",
            ))
            .subcommand(command(
                "capability",
                "Report whether a lane can be spawned, optionally at a named tier.",
                "qol sessions capability [--tier TOKEN]",
                "Answers whether spawning a lane is possible right now: lane_spawn is true when a registered tool is installed and a model is resolvable, and --tier TOKEN narrows that to models whose id carries the token, so a caller asks for a tier without naming a vendor or a model. Each tool row reports its launch program, whether that program is installed, the models its catalog lists, and the subset matching the tier. A tool whose harness exposes no model catalog reports no models and answers only the untiered question. The same JSON lists the configured agent_profiles sorted by preference with their tool, model, roles, image_input and visual_review declarations plus spend_allowed against allowed_models, so a caller can choose a profile within budget without any vendor ranking being baked in.",
                "Capability JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when a flag is invalid or the spawn model config cannot be read.",
            ))
            .subcommand(command(
                "spawn",
                "Launch a tagged tool session or reuse its live match.",
                "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] [--model MODEL] [--effort LEVEL] [--title TITLE] [--task TASK] [--no-resume] [--remote-control] [--agent-profile NAME] [--task-role ROLE] [--requires LIST]",
                "Launches a tagged harness for a registered tool in a new tab, or reuses the single live session already carrying the key when its tool matches. The result JSON reports the live session token, tool, key, reused, cwd, surface, model, and title, plus agent_assignment and agent_status when the dispatch is constrained. A key spanning tools conflicts, multiple matches are ambiguous, and the CLI generates a key when --key is omitted. The surface default comes from spawn_surface in ~/.config/qol-tray/sessions.toml, then tab; an explicit --model overrides the spawned session's model; a [spawn_defaults.<harness>] table in the same file, keyed by the harness the spawning session runs, fills the tool, model and effort the spawn leaves out (tool defaults to that same harness, and model and effort apply only when the spawn launches that tool), then spawn_model is the fallback, while a selected agent profile supplies its declared model and refuses a conflicting --model or --tool. The tool_models mapping in the same file binds models to harnesses, and a pair it does not declare is refused before launch. --effort (low, medium, high, xhigh, max) goes to harnesses that take one, claude as --effort and pi as --thinking, and every claude launch skips permission prompts; spawn and fork build these flags from one source. --remote-control starts a claude lane with Remote Control on, named after its key; spawn_remote_control in sessions.toml sets the default, and a tool without Remote Control refuses it. --title names the new tab (the lane key by default); --task delivers the first round at spawn time so the round is already open when the command returns. With --background (which requires --task, and --silent-wake to skip the parent wake) the spawn JSON also reports completion_marker and receipt_path, the absolute path of the receipt.json the watcher writes when the round completes (a bridge call that collects the round first returns the report itself and no receipt is written); grouped lanes (a --lanes set of two or more, or --group) omit it because the set delivers one combined report instead. Resume is automatic when the spawn ledger holds a session id for the key (same tool and cwd); --no-resume opts out, and the spawn JSON reports resume and resume_detail; a constrained resume cannot promote a prior session without a compatible recorded assignment and asks for resume=false instead. --agent-profile names an agent_profiles entry, --task-role is required for every constrained assignment even when --requires is empty, and --requires is a comma-separated list drawn from image_input and visual_review: image_input needs a native declaration and visual_review needs native plus allow. Configuring any profile enables enforcement unless enforce_agent_profiles is false, and a lane set inherits top-level assignment fields while refusing a field set both top-level and on a lane.",
                "Spawn JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on orchestration, identity, capability, readiness, or task delivery failure.",
            ))
            .subcommand(command(
                "fork",
                "Launch a detached architect that never reports back.",
                "qol sessions fork [--tool TOOL] --cwd PATH [--key KEY] [--model MODEL] (--brief TEXT | --brief-file PATH | --blank) [--effort LEVEL] [--title TITLE] [--surface tab|os-window] [--parent SESSION] [--no-chat] [--agent-profile NAME] [--task-role ROLE] [--requires LIST] [--park] [--remote-control] [--dry-run]",
                "Launches a new terminal that owns the brief end to end and reports to the user in its own terminal, not back to the caller. Use it when a second problem surfaces mid-session and chasing it would cost the thread already being held. A fork is the root of a new tree, not a lane: no round is opened on it, no completion signal is embedded in its launch, and bridge refuses it by name. The brief is written to a file under the sessions data dir and the launch points the fork at that path, so a long problem statement survives argv limits and stays readable after the screen scrolls. --parent names the session being forked from; its chat is copied beside the brief and the fork is pointed at that copy, while --no-chat skips the copy. --blank launches a fresh session with no brief, no chat copy and no launch prompt, so the user types its first message. --model is optional: an explicit value wins, then the selected agent profile's declared model, then spawn_model in sessions.toml, and a value that conflicts with the selected profile is refused. --tool is optional and resolves from the selected profile or the model's tool_models entry; a pair the mapping does not declare is refused. --effort is passed to tools that take one (claude: low, medium, high, xhigh, max). A claude fork starts with --dangerously-skip-permissions. A claude launch into a folder Claude Code has not been told to trust (no trust record for it or a parent folder, and always the home folder, whose trust it never remembers) is refused before the terminal opens, naming the folder, because Claude asks about trust before it reads the prompt and that flag does not skip the question. --agent-profile binds the fork to an agent_profiles entry; --task-role and --requires declare the role and capabilities, and the resolved assignment is recorded with the fork. --park lets the fork close its terminal while it is parked on a long wait and reopen in a new tab when the wait ends; a fork with a brief whose tool can resume conversations is also watched from launch, so its tab closes once its turn ends unless the user writes in it or it parks, and its final message is shown in a CLI Sessions notification that reopens the conversation when clicked. Without --park the fork stays open until it sends session_send kind report with done true, which closes its tab when that turn ends. --remote-control starts a claude fork with Remote Control on, named after its key, and the fork JSON carries remote_control_url when the session shows the link within 5 seconds; fork_remote_control in sessions.toml sets the default, and a tool without Remote Control refuses it. --dry-run launches the harness with no prompt and types the prompt into its input without submitting, so a launch can be checked without spending tokens. --key defaults to the brief's first content words plus a short id, or fork-<id> when the brief has none or the fork is blank. A key already held by a live session is refused because a fork always starts fresh.",
                "Fork JSON on stdout; diagnostics on stderr.",
                "Exits non-zero on validation, key conflict, readiness, or launch failure.",
            ))
            .subcommand(command(
                "forks",
                "List the detached forks recorded on this host.",
                "qol sessions forks",
                "Prints one tab-separated row per recorded fork: key, tool, tier, session token, the path to its brief, then its chat copy, its Remote Control link while it is live with Remote Control on, and the GitHub pull requests it linked in session_send messages, each - when absent. Forks are never collected, so this listing is the only link back to a tree that was deliberately cut loose.",
                "Fork rows on stdout; diagnostics on stderr.",
                "Exits non-zero when the fork directory cannot be read.",
            ))
            .subcommand(command(
                "switchboard",
                "Launch the one control chat that starts, continues, lists and closes sessions.",
                "qol sessions switchboard [--here] [--model MODEL] [--effort LEVEL] [--surface tab|os-window]",
                "Opens the one control chat, a claude session keyed and titled switchboard, in its own folder, and the only terminal the user keeps open. New work, a new or empty session, and a closed conversation brought back all run as forks, which are Claude Code subagents inside that one chat, each pinned to a project folder named on the first line of its task; a closed conversation's fork reads its transcript and carries it on. A terminal opens only for another tool, a conversation with no transcript, or when asked for one, and terminal sessions that still run are listed, closed and messaged with the qol-sessions MCP tools. Only one runs at a time: a live session already holding the switchboard key is refused. A new one resumes the newest switchboard conversation started under its current instructions and wakes the forks that were running or waiting when the last one ended, except those the user stopped. A resumed conversation keeps the instructions it started with, so after an update that changed them it starts a new conversation and starts those forks again as new forks, each reading the transcript of the one it replaces. --effort defaults to medium; --model and --surface resolve as they do for a fork. The switchboard is not recorded as a fork and never parks.",
                "Switchboard JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when a switchboard is already running, or on validation, readiness, or launch failure.",
            ))
            .subcommand(command(
                "park",
                "Close this harness and resume it when a long command exits.",
                "qol sessions park [--model MODEL] [--effort LEVEL] [--title TITLE] [--session SESSION] -- <command> [args...]",
                "Forwards to the installed CLI Sessions plugin, which owns parking. It parks the calling harness session on a long wait such as a pull request watcher: a detached CLI Sessions process runs the command, closes the calling terminal once its current turn ends unless its harness reports it must stay open, and when the command exits resumes the same conversation in a new tab with the exit code and the tail of the output. The resumed tab closes once its turn ends, unless it parks again or the user writes in it. If the calling terminal is still open when the command exits, the result is submitted into it instead.",
                "Park JSON on stdout; diagnostics on stderr.",
                "Exits non-zero when CLI Sessions is not installed or its park fails.",
            ))
            .subcommand(command(
                "parked",
                "List parked sessions and what became of them.",
                "qol sessions parked",
                "Forwards to the installed CLI Sessions plugin and prints one row per parked session.",
                "Park rows on stdout; diagnostics on stderr.",
                "Exits non-zero when CLI Sessions is not installed or cannot read its parked sessions.",
            ))
            .subcommand(command(
                "unpark",
                "Stop a parked wait and resume the conversation now.",
                "qol sessions unpark <id>",
                "Forwards to the installed CLI Sessions plugin, which stops the waiting command and resumes the conversation now, or reopens a finished one.",
                "A confirmation line on stdout; diagnostics on stderr.",
                "Exits non-zero when CLI Sessions is not installed or the unpark fails.",
            ))
            .subcommand(command(
                "submit",
                "Deliver one bounded task and return with the round open.",
                "qol sessions submit <session> --task TASK [--acknowledge-marker TEXT] [--agent-profile NAME] [--task-role ROLE] [--requires LIST]",
                "Submits exactly once with a generated completion signal and returns immediately with the round recorded and open, so several lanes can run in parallel before any of them is awaited. Refuses when a round is already pending on that session; pass the reviewed completion_marker as --acknowledge-marker to start the next round. --agent-profile, --task-role and --requires declare a constrained round; omitted, they inherit the assignment recorded against the session, and in every case the recorded profile is revalidated against current policy before the task is dispatched. A constrained submit to a session with no recorded identity is refused instead of relabelling it. Wait for the completion with `qol sessions bridge <session>` (no task) or resume.",
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
                "Reads the durable per-session bridge state: a waiting round prints its resume command; a round whose target already printed its completion signal prints phase=collect with the resume command that collects it; a round whose target went idle without its completion signal prints resume --kickstart; a round whose target's terminal is gone prints discard; a completed round prints a review instruction with the acknowledge-marker bridge template; no rounds prints phase=idle. Every row also carries agent_status and the recorded agent_assignment when one exists, so a reviewer sees what policy was checked for the dispatch.",
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
            .subcommand(command(
                "lane-exec",
                "Start a lane under the Claude account its architect runs under.",
                "qol sessions lane-exec (--secret NAME --env VAR | --login-file PATH) -- PROGRAM [ARGS...]",
                "Spawn, fork, reopen and park put this in front of a claude lane so it runs under the caller's account, and it replaces itself with PROGRAM, so the token never passes through the terminal. Every spawn, fork and park records the caller's login variables (CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN) in a private file per CLAUDE_CONFIG_DIR under the qol-tray data folder, lane-logins/, and removes it when the caller has none; --login-file clears those variables and sets the recorded ones. claude_accounts in sessions.toml overrides that for a config dir: --secret reads the named secret from the OS credential store (the login Keychain on macOS, the Secret Service via secret-tool on Linux, the Credential Manager on Windows) and exports it as VAR. Each entry is `[claude_accounts.\"<config dir>\"]` with `secret = \"<name>\"` and an optional `token_env` that defaults to CLAUDE_CODE_OAUTH_TOKEN.",
                "The lane's own output; diagnostics on stderr.",
                "Exits non-zero when the secret or login file cannot be read or PROGRAM cannot start.",
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
        .command(
            command(
                "agents",
                "Inspect and manage the agent home registry.",
                "qol agents <list|current|add|remove>",
                "One registry file (agents.toml in the qol config directory) declares the agent homes this machine knows; see docs/agent-homes.md.",
                "Home rows, home ids, or confirmation lines on stdout; diagnostics on stderr.",
                "Exits non-zero when a harness name or path is invalid or the registry file cannot be read or written.",
            )
            .run_plain_text(|context| agents::run(context.args()))
            .subcommand(
                command(
                    "list",
                    "List every registered agent home plus unregistered env homes.",
                    "qol agents list [--json]",
                    "One tab-separated row per home: harness, id, shared or -, default or -, then declared, implicit, or unregistered; each harness whose env home is set but not registered adds one extra unregistered row.",
                    "Tab-separated rows on stdout, or the homes JSON object with --json.",
                    "Exits non-zero when the registry cannot be read.",
                )
                .run_plain_text(|_| agents::list_plain())
                .run_json(|_| agents::list_json()),
            )
            .subcommand(
                command(
                    "current",
                    "Print the current agent home for a harness.",
                    "qol agents current <claude|codex|kimi|pi> [--json]",
                    "The harness home env var wins when set; otherwise the harness default home applies. Scripts call this verb.",
                    "The home id on stdout, or the AgentHome JSON with --json.",
                    "Exits non-zero when the harness name is unknown.",
                )
                .run_plain_text(|context| agents::current_plain(context.args()))
                .run_json(|context| agents::current_json(context.args())),
            )
            .subcommand(
                command(
                    "add",
                    "Add or update an agent home in the registry file.",
                    "qol agents add <claude|codex|kimi|pi> <path> [--shared] [--default]",
                    "Appends or updates the [[home]] entry in agents.toml, creating the file; --default clears default on that harness's other entries; prints the resulting row after the confirmation.",
                    "A confirmation line on stdout; diagnostics on stderr.",
                    "Exits non-zero when the harness name or path is invalid or the registry file cannot be written.",
                )
                .run_plain_text(|context| agents::add_plain(context.args())),
            )
            .subcommand(
                command(
                    "remove",
                    "Remove an agent home from the registry file.",
                    "qol agents remove <path>",
                    "Removes every [[home]] entry whose normalized path matches, regardless of harness, and errors when nothing matched; comments and formatting survive.",
                    "A confirmation line naming the removed harnesses on stdout; diagnostics on stderr.",
                    "Exits non-zero when the entry is absent or the registry file cannot be written.",
                )
                .run_plain_text(|context| agents::remove_plain(context.args())),
            ),
        )
        .command(
            command(
                "mcp",
                "Print qol-tray MCP endpoint facts and configure agent harnesses.",
                "qol mcp <url|token|headers|configure>",
                "Covers the stateless streamable HTTP MCP endpoint that qol-tray serves behind its token auth: url prints the endpoint URL, token prints the auth token, headers prints a JSON headers object, and configure writes or replaces the qol MCP entry for claude, codex, pi, or kimi.",
                "URL, token, headers JSON, or configure confirmation on stdout; diagnostics on stderr.",
                "Exits non-zero when the tray token or the harness config file is missing or a harness name is unknown.",
            )
            .subcommand(command(
                "url",
                "Print the MCP endpoint URL.",
                "qol mcp url",
                "Prints the local streamable HTTP endpoint URL that qol-tray serves.",
                "The URL on stdout.",
                "Exits zero.",
            ))
            .subcommand(command(
                "token",
                "Print the tray HTTP auth token.",
                "qol mcp token",
                "Reads the token from the qol config directory; it exists once qol-tray has run.",
                "The token on stdout.",
                "Exits non-zero when the token file is missing.",
            ))
            .subcommand(command(
                "headers",
                "Print a JSON headers object carrying the auth token.",
                "qol mcp headers",
                "Prints a compact JSON object mapping the auth header name to the token, ready to paste into a harness config.",
                "The headers JSON on stdout.",
                "Exits non-zero when the token file is missing.",
            ))
            .subcommand(command(
                "configure",
                "Write or replace the qol MCP entry in a harness's user config.",
                "qol mcp configure <claude|codex|pi|kimi>",
                "Writes or replaces the qol entry under mcpServers for claude, pi, and kimi, or the codex TOML table; claude and pi reference the qol CLI for headers and token while codex and kimi embed the token value directly. Other servers, tables, and comments survive. The config file must already exist.",
                "updated <path> and the written entry on stdout; diagnostics on stderr.",
                "Exits non-zero when the harness name is unknown or the config file does not exist.",
            )),
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
