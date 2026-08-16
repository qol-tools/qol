use std::ffi::OsString;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use qol_headless::OutputFormat;
use qol_terminal_sessions::bridge::BridgeCheckpoint;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionFocus, SessionInventory,
    TerminalSessionService, TextInput,
};

mod bridge;
mod capability;
mod close;
mod contract;
mod export;
mod last_send;
mod mcp;
mod spawn;
mod watch;
mod watch_owner;

pub(crate) struct SessionSubcommand {
    pub(crate) name: &'static str,
    run: fn(&[OsString], OutputFormat) -> Result<()>,
}

pub(crate) const SUBCOMMANDS: [SessionSubcommand; 17] = [
    SessionSubcommand {
        name: "list",
        run: |_rest, format| list(format),
    },
    SessionSubcommand {
        name: "capability",
        run: |rest, _format| capability::run(rest),
    },
    SessionSubcommand {
        name: "spawn",
        run: |rest, _format| spawn::run(rest),
    },
    SessionSubcommand {
        name: "submit",
        run: |rest, _format| run_submit(rest),
    },
    SessionSubcommand {
        name: "send",
        run: |rest, _format| send(rest),
    },
    SessionSubcommand {
        name: "bridge",
        run: |rest, _format| run_bridge(rest),
    },
    SessionSubcommand {
        name: "next",
        run: |rest, format| next(rest, format),
    },
    SessionSubcommand {
        name: "resume",
        run: |rest, _format| run_resume(rest),
    },
    SessionSubcommand {
        name: "discard",
        run: |rest, _format| discard(rest),
    },
    SessionSubcommand {
        name: "interrupt",
        run: |rest, _format| interrupt(rest),
    },
    SessionSubcommand {
        name: "close",
        run: |rest, _format| close::run(rest),
    },
    SessionSubcommand {
        name: "read",
        run: |rest, _format| read_screen(rest),
    },
    SessionSubcommand {
        name: "wait",
        run: |rest, _format| wait(rest),
    },
    SessionSubcommand {
        name: "focus",
        run: |rest, _format| focus(rest),
    },
    SessionSubcommand {
        name: "watch",
        run: |rest, _format| watch::run(rest),
    },
    SessionSubcommand {
        name: "mcp",
        run: |rest, _format| mcp::run(rest),
    },
    SessionSubcommand {
        name: "export",
        run: |rest, _format| export::run(rest),
    },
];

fn split_subcommand(args: &[OsString]) -> (&str, &[OsString]) {
    let name = args
        .first()
        .and_then(|argument| argument.to_str())
        .unwrap_or(SUBCOMMANDS[0].name);
    (name, args.get(1..).unwrap_or_default())
}

pub(crate) fn run(args: &[OsString], output_format: OutputFormat) -> Result<()> {
    let (subcommand, rest) = split_subcommand(args);
    if let Some(subcommand) = SUBCOMMANDS.iter().find(|entry| entry.name == subcommand) {
        return (subcommand.run)(rest, output_format);
    }
    match subcommand {
        "help" | "-h" | "--help" => {
            print!("{}", help_text());
            Ok(())
        }
        other => bail!(
            "unknown qol sessions subcommand `{other}`\n\n{}",
            help_text()
        ),
    }
}

pub(crate) fn help_text() -> &'static str {
    r#"qol sessions

Bridge work between independent terminal sessions.

Primary usage:
  qol sessions list [--json]
  qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] [--model MODEL] [--title TITLE] [--task TASK] [--background] [--resume]
  qol sessions submit <session> --task TASK [--acknowledge-marker TEXT]
  qol sessions bridge <session> [<task...>] [--timeout-ms N] [--acknowledge-marker TEXT] [--gate]
  qol sessions next [<session>] [--json]
  qol sessions resume <session> [--timeout-ms N] [--kickstart]
  qol sessions discard <session>
  qol sessions interrupt <session>
  qol sessions close <session>

Diagnostics:
  qol sessions send <session> <text...> [--submit]
  qol sessions read <session>
  qol sessions wait <session> [--timeout-ms N] [--expect TEXT]
  qol sessions focus <session>
  qol sessions watch [TOKEN...]
  qol sessions mcp
  qol sessions export [pi]
  qol sessions help

Details:
  list discovers stable live-session tokens.
  spawn launches a tagged harness for a registered tool in a new terminal tab
  (default surface), or reuses the single live session already carrying the
  key when its tool matches; a key used by a different tool conflicts and
  multiple matches are ambiguous. The result JSON reports session, tool, key,
  reused, cwd, surface, and model from the live session facts. The CLI
  generates a key when --key is omitted; the MCP session_spawn tool requires
  one so retries are idempotent. The surface default comes from the
  spawn_surface setting in ~/.config/qol-tray/sessions.toml, then tab. An
  explicit --model override names the spawned session's model (appended to
  the harness launch as --model); the spawn_model setting in the same file is
  the fallback. --title names the new tab (the lane key by default), and
  --task delivers the first round at spawn time so the round is already open
  when the command returns; the outcome JSON then reports task_submitted,
  completion_marker, and next_command. Resume is automatic when the spawn
  ledger holds a session id for the key (same tool and cwd), so a respawned
  lane continues the prior session; --no-resume opts out; the spawn JSON
  reports resume and resume_detail.
  submit delivers one bounded task and returns immediately with the round
  recorded and open, so several lanes can run in parallel before any of them
  is awaited; it refuses when a round is already pending on that session.
  Submitted rounds close their lane terminal after the watcher confirms
  completion; sessions without a spawn identity are never closed.
  bridge submits one bounded task, supplies a generated completion signal,
  waits in the same call, and prints JSON with completed, submitted, session,
  completion_marker, screen, reads, and elapsed_ms. Without a task it
  re-attaches to the pending round and waits for its completion marker.
  Before submitting, it
  resumes any unfinished bridge for that session and returns its latest
  response with submitted=false. Pass its reviewed completion_marker through
  --acknowledge-marker to submit the following round. Its timeout is clamped
  to 1s..24h (default 24h). With --gate the local quality gate (cargo fmt
  --check, clippy with -D warnings, and the qol and qol-terminal-sessions
  test suites) runs in the current working directory once the round
  completes, and its per-step results and GREEN/RED verdict are appended to
  screen; a missing Cargo.toml skips the gate with a note, and --no-gate
  (the default) leaves it off.
  Use -- before task text that contains --timeout-ms.
  next reads the durable per-session bridge state and prints the exact next
  command for each open round: resume while waiting, resume --kickstart when
  the target went idle without emitting its completion signal, discard when
  the target's terminal is gone, then a review instruction with the
  acknowledge-marker bridge template once complete.
  resume re-attaches to the one pending round and waits for its completion
  marker without submitting anything; its timeout defaults to 24h. With
  --kickstart it first nudges the interrupted session to continue or emit
  the signal. A wait that detects an idle target returns stalled=true
  instead of blocking until timeout; rerun next when that happens.
  discard removes the pending-bridge checkpoint of a session whose terminal
  is gone (verified via live discovery); it refuses a live session, refuses
  when no checkpoint exists, and never touches last-send state or spawn
  locks.
  interrupt sends the target tool's stop key (agent TUIs: esc, plain
  shells: ctrl+c) while a round is open, leaving the round and its
  queued input intact. Every bridge JSON result carries next_command;
  run it instead of repeating the previous command.
  close terminates a spawned implementation session's terminal after its
  feature loop is closed. It refuses the calling terminal, sessions without
  a spawn identity, and sessions with an open loop.
  watch is long-running infrastructure for event-driven lane wakeup: it
  polls each watched round's screen for its completion marker, prints one
  JSON line per event (completed, gone, stalled), and exits 0 when no
  watched rounds remain pending. Each event is delivered into the round's
  initiator terminal (the checkpoint driver) as a submitted wake message
  before the line is printed; an autoclose round closes its lane tab only
  after that delivery is confirmed, and an undeliverable wake leaves a
  wake-failed-<session>.json trace plus delivered=false on the event line.
  A lane whose window closed right after showing the marker completes with
  its last screen instead of going gone, so the report survives. With no
  tokens it watches every pending round in the checkpoint store and takes a
  spawn lock so two watchers do not double-poll; explicit tokens need no
  lock. It is not an agent tool.
  The MCP and generated agent surfaces expose sessions_list, session_spawn,
  session_bridge, session_loop_close, and session_close. The remaining
  commands are human diagnostics.

Exit:
  Exits non-zero on discovery, identity, capability, validation, or delivery
  failures. A bridge timeout is a successful JSON result with completed=false.
"#
}

fn service() -> Result<TerminalSessionService> {
    Ok(TerminalSessionService::system())
}

fn list(output_format: OutputFormat) -> Result<()> {
    let facts = service()?.discover().context("session discovery failed")?;
    let interpreter = CliSessionInterpreter::system();
    let mut rows = facts
        .iter()
        .filter_map(|session| {
            let binding = session.binding().ok()?;
            let descriptor = interpreter.describe(session);
            Some(contract::session_row(session, &binding, &descriptor))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.session.cmp(&right.session));
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&rows).context("failed to serialize sessions")?
        ),
        OutputFormat::PlainText => {
            for row in rows {
                let caps = row.capabilities.join(",");
                let activity = row
                    .activity
                    .map(|active| if active { "active" } else { "idle" })
                    .unwrap_or("-");
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    row.session,
                    row.tool,
                    row.title,
                    row.cwd,
                    if caps.is_empty() { "-" } else { &caps },
                    activity,
                );
            }
        }
    }
    Ok(())
}

fn send(args: &[OsString]) -> Result<()> {
    let (binding_token, text, submit) = parse_send_args(args)?;
    let binding = SessionBinding::from_str(&binding_token)
        .map_err(|error| anyhow!("invalid session token `{binding_token}`: {error}"))?;
    let mode = if submit {
        DeliveryMode::Submit
    } else {
        DeliveryMode::Insert
    };
    service()?
        .send_text(&binding, &text, mode)
        .context("text delivery failed")?;
    if let Some(store) = last_send::LastSendStore::system() {
        store.record(&binding, &text);
    }
    println!("delivered {} to {}", mode_label(mode), binding);
    if mode == DeliveryMode::Insert {
        println!("WARNING: the text sits unsubmitted in the target's input; nothing runs until Enter is pressed there. Pass --submit to deliver and submit in one step.");
    }
    Ok(())
}

fn run_submit(args: &[OsString]) -> Result<()> {
    let (binding_token, task, acknowledge_marker) = parse_submit_args(args)?;
    let binding = SessionBinding::from_str(&binding_token)
        .map_err(|error| anyhow!("invalid session token `{binding_token}`: {error}"))?;
    let terminals = service()?;
    let outcome = bridge::submit(
        &terminals,
        &CliSessionInterpreter::system(),
        &binding,
        &task,
        &bridge::PendingBridgeStore::system()?,
        acknowledge_marker.as_deref(),
        false,
    )?;
    println!(
        "{}",
        serde_json::to_string(&outcome).context("failed to serialize submit outcome")?
    );
    Ok(())
}

fn parse_submit_args(args: &[OsString]) -> Result<(String, String, Option<String>)> {
    let usage = "qol sessions submit <session> --task TASK [--acknowledge-marker TEXT]";
    let binding = args
        .first()
        .and_then(|argument| argument.to_str())
        .ok_or_else(|| anyhow!("usage: {usage}"))?
        .to_owned();
    let mut task = None;
    let mut acknowledge_marker = None;
    let mut index = 1;
    while index < args.len() {
        let argument = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("submit arguments must be valid UTF-8"))?;
        match argument {
            "--task" => {
                task = Some(
                    args.get(index + 1)
                        .and_then(|value| value.to_str())
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| anyhow!("usage: {usage}"))?
                        .to_owned(),
                );
                index += 2;
            }
            "--acknowledge-marker" => {
                acknowledge_marker = Some(
                    args.get(index + 1)
                        .and_then(|value| value.to_str())
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| anyhow!("usage: {usage}"))?
                        .to_owned(),
                );
                index += 2;
            }
            other => bail!("unknown submit flag `{other}`\nusage: {usage}"),
        }
    }
    let task = task.ok_or_else(|| anyhow!("usage: {usage}"))?;
    Ok((binding, task, acknowledge_marker))
}

fn run_bridge(args: &[OsString]) -> Result<()> {
    let parsed = parse_bridge_args(args)?;
    let binding = SessionBinding::from_str(&parsed.binding)
        .map_err(|error| anyhow!("invalid session token `{}`: {error}", parsed.binding))?;
    let timeout_ms = parsed
        .timeout_ms
        .unwrap_or(bridge::TIMEOUT_DEFAULT_MS)
        .clamp(bridge::TIMEOUT_MIN_MS, bridge::TIMEOUT_MAX_MS);
    let terminals = service()?;
    let pending = bridge::PendingBridgeStore::system()?;
    let ledger = spawn::SpawnLedger::system()?;
    let locks = spawn::SpawnLocks::system()?;
    let mut outcome = match parsed.task.as_deref() {
        Some(task) => bridge::execute(
            &terminals,
            &CliSessionInterpreter::system(),
            &binding,
            task,
            std::time::Duration::from_millis(timeout_ms),
            &pending,
            &ledger,
            &locks,
            parsed.acknowledge_marker.as_deref(),
        )?,
        None => {
            if parsed.acknowledge_marker.is_some() {
                bail!(
                    "bridge without a task takes no --acknowledge-marker; acknowledge the reviewed round on the next submit or the loop close"
                );
            }
            bridge::resume(
                &terminals,
                &CliSessionInterpreter::system(),
                &binding,
                std::time::Duration::from_millis(timeout_ms),
                &pending,
                &ledger,
                &locks,
                false,
            )?
        }
    };
    if parsed.gate && outcome.completed {
        outcome.screen = bridge::run_quality_gate(&outcome.screen, &std::env::current_dir()?);
    }
    println!(
        "{}",
        serde_json::to_string(&outcome).context("failed to serialize bridge outcome")?
    );
    Ok(())
}

struct BridgeArgs {
    binding: String,
    task: Option<String>,
    timeout_ms: Option<u64>,
    acknowledge_marker: Option<String>,
    gate: bool,
}

fn parse_bridge_args(args: &[OsString]) -> Result<BridgeArgs> {
    let usage =
        "qol sessions bridge <session> <task...> [--timeout-ms N] [--acknowledge-marker TEXT] [--gate]";
    let binding = args
        .first()
        .and_then(|argument| argument.to_str())
        .ok_or_else(|| anyhow!("usage: {usage}"))?
        .to_owned();
    let mut task_parts = Vec::new();
    let mut timeout_ms = None;
    let mut acknowledge_marker = None;
    let mut gate = false;
    let mut literal = false;
    let mut index = 1;
    while index < args.len() {
        let argument = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("session arguments must be valid UTF-8"))?;
        if literal {
            task_parts.push(argument.to_owned());
            index += 1;
            continue;
        }
        match argument {
            "--" => {
                literal = true;
                index += 1;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| anyhow!("usage: {usage}"))?;
                timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| anyhow!("invalid --timeout-ms value `{value}`"))?,
                );
                index += 2;
            }
            "--acknowledge-marker" => {
                let value = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| anyhow!("usage: {usage}"))?;
                acknowledge_marker = Some(value.to_owned());
                index += 2;
            }
            "--gate" => {
                gate = true;
                index += 1;
            }
            "--no-gate" => {
                gate = false;
                index += 1;
            }
            other => {
                task_parts.push(other.to_owned());
                index += 1;
            }
        }
    }
    let task = task_parts.join(" ");
    Ok(BridgeArgs {
        binding,
        task: (!task.is_empty()).then_some(task),
        timeout_ms,
        acknowledge_marker,
        gate,
    })
}

fn run_resume(args: &[OsString]) -> Result<()> {
    let usage = "qol sessions resume <session> [--timeout-ms N] [--kickstart]";
    let mut kickstart = false;
    let mut rest = Vec::new();
    for argument in args {
        if argument.to_str() == Some("--kickstart") {
            kickstart = true;
        } else {
            rest.push(argument.clone());
        }
    }
    let (mut positionals, timeout_ms, expect) = parse_wait_args(&rest)?;
    if expect.is_some() || positionals.len() != 1 {
        bail!("usage: {usage}");
    }
    let binding: SessionBinding = positionals
        .pop()
        .unwrap()
        .parse()
        .map_err(|_| anyhow!("invalid session token"))?;
    let timeout_ms = timeout_ms
        .unwrap_or(bridge::TIMEOUT_MAX_MS)
        .clamp(bridge::TIMEOUT_MIN_MS, bridge::TIMEOUT_MAX_MS);
    let outcome = bridge::resume(
        &service()?,
        &CliSessionInterpreter::system(),
        &binding,
        std::time::Duration::from_millis(timeout_ms),
        &bridge::PendingBridgeStore::system()?,
        &spawn::SpawnLedger::system()?,
        &spawn::SpawnLocks::system()?,
        kickstart,
    )?;
    println!(
        "{}",
        serde_json::to_string(&outcome).context("failed to serialize bridge outcome")?
    );
    Ok(())
}

fn interrupt(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions interrupt <session>")?;
    let pending = bridge::PendingBridgeStore::system()?;
    let round = pending.pending_round(&binding)?.ok_or_else(|| {
        anyhow!("no open bridge round for `{binding}`; interrupt only steers an active round")
    })?;
    if round.completed {
        bail!("the round is already complete; review it via `qol sessions next {binding}`");
    }
    let terminals = service()?;
    let target = terminals
        .discover()
        .context("session discovery failed")?
        .into_iter()
        .find(|session| session.id == *binding.session_id())
        .ok_or_else(|| anyhow!("interrupt target `{binding}` is no longer present"))?;
    let key = CliSessionInterpreter::system().interrupt_key(&target);
    terminals
        .send_key(&binding, key)
        .context("interrupt delivery failed")?;
    println!(
        "sent {key} to {binding}; the round stays open - continue via `qol sessions next {binding}`"
    );
    Ok(())
}

fn next(args: &[OsString], output_format: OutputFormat) -> Result<()> {
    let pending = bridge::PendingBridgeStore::system()?;
    let rounds = if args.is_empty() {
        pending.pending_rounds()?
    } else {
        let binding = single_binding(args, "qol sessions next [<session>]")?;
        pending.pending_round(&binding)?.into_iter().collect()
    };
    let rows = next_rows(
        &service()?,
        &CliSessionInterpreter::system(),
        &pending,
        &rounds,
    )?;
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&rows).context("failed to serialize next rounds")?
        ),
        OutputFormat::PlainText => {
            if rows.is_empty() {
                println!("phase=idle");
                println!("No pending round. Discover targets with `qol sessions list`, then submit one bounded round: `qol sessions bridge <session> -- <task>`.");
                return Ok(());
            }
            for row in &rows {
                println!(
                    "phase={} session={}",
                    row["phase"].as_str().unwrap_or_default(),
                    row["session"].as_str().unwrap_or_default(),
                );
                println!("{}", row["instruction"].as_str().unwrap_or_default());
                let command = row["command"].as_str().unwrap_or_default();
                if !command.is_empty() {
                    println!("run: {command}");
                }
            }
        }
    }
    Ok(())
}

fn wake_failed(session: &str) -> bool {
    let Some(dir) = qol_config::data_subdir("sessions") else {
        return false;
    };
    let key = session.replace([':', '.'], "_");
    dir.join(format!("wake-failed-{key}.json")).exists()
}

fn next_rows(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    pending: &bridge::PendingBridgeStore,
    rounds: &[bridge::PendingRound],
) -> Result<Vec<serde_json::Value>> {
    let live = terminals.discover().ok();
    let mut rows = Vec::with_capacity(rounds.len());
    for round in rounds {
        let binding = round.session.parse::<SessionBinding>().ok();
        let attached = binding
            .as_ref()
            .and_then(|binding| pending.owner_pid(binding));
        if let Some(pid) = attached {
            rows.push(serde_json::json!({
                "phase": "attached",
                "session": round.session,
                "command": "",
                "instruction": format!("Another bridge process (pid {pid}) is already attached to this session and is waiting for its completion signal. Do not start a bridge, resume, or new task for this session; let that process return."),
            }));
            continue;
        }
        let gone = binding.as_ref().is_some_and(|binding| {
            live.as_ref().is_some_and(|facts| {
                !facts
                    .iter()
                    .any(|session| session.id == *binding.session_id())
            })
        });
        if gone {
            rows.push(serde_json::json!({
                "phase": "gone",
                "session": round.session,
                "command": format!("qol sessions discard {}", round.session),
                "instruction": "The implementation terminal is gone, so this round cannot resume or be reviewed here. Run the command to drop the orphaned checkpoint, then start a fresh round on a live session.",
            }));
            continue;
        }
        if round.completed {
            let wake_note = if wake_failed(round.session.as_str()) {
                " The completion wake could not be delivered to the initiator terminal (see wake-failed-*.json in the sessions data dir); the lane stayed open as the report surface."
            } else {
                ""
            };
            rows.push(serde_json::json!({
                "phase": "review",
                "session": round.session,
                "completion_marker": round.completion_marker,
                "command": format!(
                    "qol sessions bridge {} --acknowledge-marker {} -- <next bounded correction task>",
                    round.session, round.completion_marker
                ),
                "instruction": format!("The round is complete. Personally review the implementation against the acceptance criteria first. Then either run the command with the next bounded correction task, or, when the entire feature is accepted, call session_loop_close with this session and completion_marker.{wake_note}"),
            }));
            continue;
        }
        let stalled = binding.as_ref().is_some_and(|binding| {
            bridge::session_liveness(terminals, interpreter, binding)() == Some(false)
        });
        if stalled {
            rows.push(serde_json::json!({
                "phase": "stalled",
                "session": round.session,
                "command": format!(
                    "qol sessions resume {} --kickstart --timeout-ms {}",
                    round.session,
                    bridge::TIMEOUT_MAX_MS
                ),
                "instruction": "The implementation session went idle without emitting its completion signal; it was likely interrupted. Run the command: it nudges the session to continue or emit the signal, then waits in the foreground. If the session is instead visibly hung mid-action, run `qol sessions interrupt <session>` first to send its tool-appropriate stop key.",
            }));
        } else {
            rows.push(serde_json::json!({
                "phase": "waiting",
                "session": round.session,
                "command": format!(
                    "qol sessions resume {} --timeout-ms {}",
                    round.session,
                    bridge::TIMEOUT_MAX_MS
                ),
                "instruction": "Implementation is still running. Run the command in the foreground and do nothing else until it returns.",
            }));
        }
    }
    Ok(rows)
}

fn discard(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions discard <session>")?;
    let removed = discard_checkpoint(
        &service()?,
        &bridge::PendingBridgeStore::system()?,
        &binding,
    )?;
    println!(
        "removed pending bridge checkpoint for {} (completion_marker={}, completed={})",
        binding, removed.completion_marker, removed.completed
    );
    Ok(())
}

fn discard_checkpoint(
    terminals: &TerminalSessionService,
    pending: &bridge::PendingBridgeStore,
    binding: &SessionBinding,
) -> Result<BridgeCheckpoint> {
    if terminals
        .discover()
        .context("session discovery failed")?
        .iter()
        .any(|session| session.id == *binding.session_id())
    {
        bail!(
            "session `{binding}` still has a live terminal; `qol sessions discard` only removes the checkpoint of a session whose terminal is gone"
        );
    }
    pending.discard(binding)
}

fn parse_send_args(args: &[OsString]) -> Result<(String, String, bool)> {
    let mut submit = false;
    let mut binding_arg: Option<String> = None;
    let mut text_parts: Vec<String> = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        let value = argument
            .to_str()
            .ok_or_else(|| anyhow!("session arguments must be valid UTF-8"))?;
        let is_last = index + 1 == args.len();
        match value {
            "--submit" if is_last => submit = true,
            "--insert" if is_last => submit = false,
            _ if binding_arg.is_none() => binding_arg = Some(value.to_owned()),
            _ => text_parts.push(value.to_owned()),
        }
    }
    let binding = binding_arg
        .ok_or_else(|| anyhow!("usage: qol sessions send <session> <text...> [--submit]"))?;
    let text = text_parts.join(" ");
    if text.is_empty() {
        bail!("usage: qol sessions send <session> <text...> [--submit]");
    }
    Ok((binding, text, submit))
}

fn read_screen(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions read <session>")?;
    let screen = service()?
        .read_screen(&binding)
        .context("screen read failed")?;
    print!("{screen}");
    Ok(())
}

fn focus(args: &[OsString]) -> Result<()> {
    let binding = single_binding(args, "qol sessions focus <session>")?;
    service()?.focus(&binding).context("focus failed")?;
    println!("focused {}", binding);
    Ok(())
}

fn wait(args: &[OsString]) -> Result<()> {
    let (mut positionals, timeout_ms, expect) = parse_wait_args(args)?;
    if positionals.len() != 1 {
        bail!("usage: qol sessions wait <session> [--timeout-ms N] [--expect TEXT]");
    }
    let binding: SessionBinding = positionals
        .pop()
        .unwrap()
        .parse()
        .map_err(|_| anyhow!("invalid session token"))?;
    let timeout_ms = timeout_ms
        .unwrap_or(mcp::WAIT_TIMEOUT_DEFAULT_MS)
        .clamp(mcp::WAIT_TIMEOUT_MIN_MS, mcp::WAIT_TIMEOUT_MAX_MS);
    let expect = expect.filter(|pattern| !pattern.is_empty());
    let last_sent = last_send::LastSendStore::system().and_then(|store| store.last_sent(&binding));
    let (settled, screen, polls, started) = mcp::poll_until_settled(
        &service()?,
        &binding,
        std::time::Duration::from_millis(timeout_ms),
        expect.as_deref(),
        last_sent.as_deref(),
    )
    .map_err(|error| anyhow!(error))?;
    let elapsed_ms = started.elapsed().as_millis();
    println!(
        "{}",
        serde_json::json!({
            "settled": settled,
            "screen": screen,
            "polls": polls,
            "elapsed_ms": elapsed_ms,
        })
    );
    Ok(())
}

fn parse_wait_args(args: &[OsString]) -> Result<(Vec<String>, Option<u64>, Option<String>)> {
    let mut positionals = Vec::new();
    let mut timeout_ms = None;
    let mut expect = None;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("arguments must be valid UTF-8"))?;
        match argument {
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| {
                        anyhow!(
                            "usage: qol sessions wait <session> [--timeout-ms N] [--expect TEXT]"
                        )
                    })?;
                timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| anyhow!("invalid --timeout-ms value `{value}`"))?,
                );
                index += 2;
            }
            "--expect" => {
                let value = args
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| {
                        anyhow!(
                            "usage: qol sessions wait <session> [--timeout-ms N] [--expect TEXT]"
                        )
                    })?;
                expect = Some(value.to_owned());
                index += 2;
            }
            other => {
                positionals.push(other.to_owned());
                index += 1;
            }
        }
    }
    Ok((positionals, timeout_ms, expect))
}

fn single_binding(args: &[OsString], usage: &str) -> Result<SessionBinding> {
    if args.len() != 1 {
        bail!("usage: {usage}");
    }
    let value = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("session token must be valid UTF-8"))?;
    SessionBinding::from_str(value)
        .map_err(|error| anyhow!("invalid session token `{value}`: {error}"))
}

fn mode_label(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Insert => "inserted",
        DeliveryMode::Submit => "submitted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::{
        BackendId, SessionCapabilities, SessionId, TerminalBackend, TerminalError, TerminalSnapshot,
    };
    use std::sync::Arc;

    #[derive(Clone)]
    struct FakeSessionBackend {
        id: BackendId,
        sessions: Vec<qol_terminal_sessions::SessionFacts>,
    }

    impl SessionInventory for FakeSessionBackend {
        fn discover(&self) -> Result<Vec<qol_terminal_sessions::SessionFacts>, TerminalError> {
            Ok(self.sessions.clone())
        }
    }

    impl ScreenReader for FakeSessionBackend {
        fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }
    }

    impl SessionFocus for FakeSessionBackend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for FakeSessionBackend {
        fn send_text(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TerminalBackend for FakeSessionBackend {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }

        fn id(&self) -> &BackendId {
            &self.id
        }
    }

    fn fake_terminals(
        sessions: Vec<qol_terminal_sessions::SessionFacts>,
    ) -> (TerminalSessionService, Arc<FakeSessionBackend>) {
        let backend = Arc::new(FakeSessionBackend {
            id: BackendId::new("fake").unwrap(),
            sessions,
        });
        let terminals = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        (terminals, backend)
    }

    fn fake_facts(native: &str, root_pid: i32) -> qol_terminal_sessions::SessionFacts {
        qol_terminal_sessions::SessionFacts {
            id: SessionId::new(BackendId::new("fake").unwrap(), native).unwrap(),
            root_pid,
            cwd: "/work".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    fn test_store(root: &tempfile::TempDir) -> bridge::PendingBridgeStore {
        bridge::PendingBridgeStore::with_dir(root.path().to_path_buf())
    }

    #[test]
    fn discard_refuses_a_session_with_a_live_terminal() {
        let root = tempfile::TempDir::new().unwrap();
        let store = test_store(&root);
        let binding = SessionBinding::from_str("v1:fake:7:123").unwrap();
        store
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let (terminals, _) = fake_terminals(vec![fake_facts("7", 123)]);

        let error = discard_checkpoint(&terminals, &store, &binding)
            .unwrap_err()
            .to_string();
        assert!(error.contains("still has a live terminal"), "{error}");
        assert!(store.pending_round(&binding).unwrap().is_some());
    }

    #[test]
    fn discard_removes_an_orphaned_checkpoint() {
        let root = tempfile::TempDir::new().unwrap();
        let store = test_store(&root);
        let binding = SessionBinding::from_str("v1:fake:7:123").unwrap();
        store
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800", false)
            .unwrap();
        let (terminals, _) = fake_terminals(Vec::new());

        let removed = discard_checkpoint(&terminals, &store, &binding).unwrap();
        assert_eq!(removed.completion_marker, "QOL_BRIDGE_DONE_round");
        assert!(store.pending_round(&binding).unwrap().is_none());
    }

    #[test]
    fn discard_refuses_when_no_checkpoint_exists() {
        let root = tempfile::TempDir::new().unwrap();
        let store = test_store(&root);
        let binding = SessionBinding::from_str("v1:fake:7:123").unwrap();
        let (terminals, _) = fake_terminals(Vec::new());

        let error = discard_checkpoint(&terminals, &store, &binding)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no pending bridge checkpoint"), "{error}");
    }

    #[test]
    fn next_reports_gone_for_orphaned_rounds_and_waiting_for_live_ones() {
        let root = tempfile::TempDir::new().unwrap();
        let store = test_store(&root);
        let orphan = SessionBinding::from_str("v1:fake:1:100").unwrap();
        store
            .start(&orphan, "QOL_BRIDGE_DONE_orphan", "v1:fake:8:800", false)
            .unwrap();
        let live = SessionBinding::from_str("v1:fake:2:200").unwrap();
        store
            .start(&live, "QOL_BRIDGE_DONE_live", "v1:fake:8:800", false)
            .unwrap();
        let (terminals, _) = fake_terminals(vec![fake_facts("2", 200)]);

        let rows = next_rows(
            &terminals,
            &CliSessionInterpreter::system(),
            &store,
            &store.pending_rounds().unwrap(),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        let by_session = |token: &str| {
            rows.iter()
                .find(|row| row["session"] == token)
                .unwrap_or_else(|| panic!("missing row for {token}"))
        };
        let orphan_row = by_session("v1:fake:1:100");
        assert_eq!(orphan_row["phase"], "gone");
        assert_eq!(orphan_row["command"], "qol sessions discard v1:fake:1:100");
        let live_row = by_session("v1:fake:2:200");
        assert_eq!(live_row["phase"], "waiting");
        assert!(live_row["command"]
            .as_str()
            .unwrap()
            .starts_with("qol sessions resume"));
    }

    #[test]
    fn capability_names_reflect_flags() {
        let mut caps = SessionCapabilities::NONE;
        assert!(contract::capability_names(&caps).is_empty());
        caps = SessionCapabilities::SCREEN_READING | SessionCapabilities::TEXT_INPUT;
        assert_eq!(contract::capability_names(&caps), ["read", "input"]);
    }

    #[test]
    fn sessions_help_advertises_spawn_and_the_24h_bridge_timeout() {
        let help = help_text();
        assert!(help.contains("default 24h"));
        assert!(!help.contains("default 1h"));
        assert!(help.contains(
            "qol sessions spawn --tool TOOL --cwd PATH [--key KEY] [--surface tab|os-window] [--model MODEL]"
        ));
        assert!(help.contains("sessions_list, session_spawn"));
        assert!(help.contains("~/.config/qol-tray/sessions.toml"));
    }

    #[test]
    fn bare_invocation_defaults_to_the_first_subcommand_without_slicing_past_the_end() {
        let cases: [(&[&str], &str, &[&str]); 4] = [
            (&[], "list", &[]),
            (&["list"], "list", &[]),
            (
                &["send", "v1:kitty:1:42", "hello"],
                "send",
                &["v1:kitty:1:42", "hello"],
            ),
            (&["nonsense"], "nonsense", &[]),
        ];
        for (input, expected_name, expected_rest) in cases {
            let args: Vec<OsString> = input.iter().map(OsString::from).collect();
            let (name, rest) = split_subcommand(&args);
            assert_eq!(name, expected_name, "input: {input:?}");
            assert_eq!(rest, expected_rest, "input: {input:?}");
        }
    }

    #[test]
    fn wait_args_parse_flags_and_clamp_timeout() {
        let args = [
            std::ffi::OsString::from("v1:kitty:1:42"),
            std::ffi::OsString::from("--timeout-ms"),
            std::ffi::OsString::from("5000"),
            std::ffi::OsString::from("--expect"),
            std::ffi::OsString::from("relay-ok"),
        ];
        let (positionals, timeout_ms, expect) = parse_wait_args(&args).unwrap();
        assert_eq!(positionals, ["v1:kitty:1:42"]);
        assert_eq!(timeout_ms, Some(5000));
        assert_eq!(expect.as_deref(), Some("relay-ok"));

        let (_, timeout_ms, _) = parse_wait_args(&[std::ffi::OsString::from("t")]).unwrap();
        let clamped = timeout_ms
            .unwrap_or(mcp::WAIT_TIMEOUT_DEFAULT_MS)
            .clamp(mcp::WAIT_TIMEOUT_MIN_MS, mcp::WAIT_TIMEOUT_MAX_MS);
        assert_eq!(clamped, mcp::WAIT_TIMEOUT_DEFAULT_MS);

        let (_, timeout_ms, _) = parse_wait_args(&[
            std::ffi::OsString::from("t"),
            std::ffi::OsString::from("--timeout-ms"),
            std::ffi::OsString::from("999999999"),
        ])
        .unwrap();
        assert_eq!(
            timeout_ms
                .unwrap()
                .clamp(mcp::WAIT_TIMEOUT_MIN_MS, mcp::WAIT_TIMEOUT_MAX_MS),
            mcp::WAIT_TIMEOUT_MAX_MS
        );
    }

    #[test]
    fn send_parses_binding_text_and_mode() {
        let args: [OsString; 4] = [
            "v1:kitty:7:123".into(),
            "echo".into(),
            "hi".into(),
            "--submit".into(),
        ];
        let (binding, text, submit) = parse_send_args(&args).unwrap();
        assert_eq!(binding, "v1:kitty:7:123");
        assert_eq!(text, "echo hi");
        assert!(submit);
    }

    #[test]
    fn send_flags_only_count_when_last() {
        let (_, text, submit) =
            parse_send_args(&["t".into(), "run".into(), "--submit".into(), "now".into()]).unwrap();
        assert_eq!(text, "run --submit now");
        assert!(!submit);

        let (_, text, submit) =
            parse_send_args(&["t".into(), "run".into(), "--submit".into()]).unwrap();
        assert_eq!(text, "run");
        assert!(submit);
    }

    #[test]
    fn bridge_args_keep_the_surface_small_and_support_literal_flags() {
        let parsed = parse_bridge_args(&[
            "v1:kitty:7:123".into(),
            "implement".into(),
            "the fix".into(),
            "--timeout-ms".into(),
            "5000".into(),
            "--acknowledge-marker".into(),
            "QOL_BRIDGE_DONE_previous".into(),
        ])
        .unwrap();
        assert_eq!(parsed.binding, "v1:kitty:7:123");
        assert_eq!(parsed.task.as_deref(), Some("implement the fix"));
        assert_eq!(parsed.timeout_ms, Some(5000));
        assert_eq!(
            parsed.acknowledge_marker.as_deref(),
            Some("QOL_BRIDGE_DONE_previous")
        );
        assert!(!parsed.gate);

        let parsed = parse_bridge_args(&[
            "v1:kitty:7:123".into(),
            "--gate".into(),
            "implement".into(),
            "the fix".into(),
        ])
        .unwrap();
        assert_eq!(parsed.task.as_deref(), Some("implement the fix"));
        assert!(parsed.gate);

        let parsed = parse_bridge_args(&[
            "v1:kitty:7:123".into(),
            "--gate".into(),
            "--no-gate".into(),
            "implement".into(),
        ])
        .unwrap();
        assert!(!parsed.gate, "--no-gate must win over --gate");

        let parsed = parse_bridge_args(&[
            "v1:kitty:7:123".into(),
            "--".into(),
            "explain".into(),
            "--timeout-ms".into(),
        ])
        .unwrap();
        assert_eq!(parsed.task.as_deref(), Some("explain --timeout-ms"));
        assert_eq!(parsed.timeout_ms, None);
        assert_eq!(parsed.acknowledge_marker, None);

        let parsed =
            parse_bridge_args(&["v1:kitty:7:123".into(), "--".into(), "--gate".into()]).unwrap();
        assert_eq!(parsed.task.as_deref(), Some("--gate"));
        assert!(!parsed.gate, "--gate after -- is task text");

        let parsed = parse_bridge_args(&["v1:kitty:7:123".into()]).unwrap();
        assert_eq!(parsed.binding, "v1:kitty:7:123");
        assert_eq!(parsed.task, None);
        assert_eq!(parsed.timeout_ms, None);
        assert_eq!(parsed.acknowledge_marker, None);
        assert!(!parsed.gate);
    }

    #[test]
    fn submit_args_require_the_task_and_support_the_acknowledge_marker() {
        let parsed = parse_submit_args(&[
            "v1:kitty:7:123".into(),
            "--task".into(),
            "implement the fix".into(),
            "--acknowledge-marker".into(),
            "QOL_BRIDGE_DONE_previous".into(),
        ])
        .unwrap();
        assert_eq!(parsed.0, "v1:kitty:7:123");
        assert_eq!(parsed.1, "implement the fix");
        assert_eq!(parsed.2.as_deref(), Some("QOL_BRIDGE_DONE_previous"));

        let unknown = parse_submit_args(&[
            "v1:kitty:7:123".into(),
            "--task".into(),
            "implement the fix".into(),
            "--no-auto-close".into(),
        ])
        .unwrap_err()
        .to_string();
        assert!(
            unknown.contains("unknown submit flag"),
            "the removed autoclose knob must be rejected: {unknown}"
        );
        assert!(unknown.contains("--no-auto-close"), "{unknown}");

        assert!(parse_submit_args(&["v1:kitty:7:123".into()]).is_err());
        assert!(parse_submit_args(&["v1:kitty:7:123".into(), "--task".into()]).is_err());
        assert!(parse_submit_args(&["v1:kitty:7:123".into(), "--bogus".into()]).is_err());
    }
}
