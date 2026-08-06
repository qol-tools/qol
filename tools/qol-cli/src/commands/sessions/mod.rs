use std::ffi::OsString;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use qol_headless::OutputFormat;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionFocus, SessionInventory,
    TerminalSessionService, TextInput,
};

mod contract;
mod export;
mod last_send;
mod mcp;

pub(crate) struct SessionSubcommand {
    pub(crate) name: &'static str,
    run: fn(&[OsString], OutputFormat) -> Result<()>,
}

pub(crate) const SUBCOMMANDS: [SessionSubcommand; 7] = [
    SessionSubcommand {
        name: "list",
        run: |_rest, format| list(format),
    },
    SessionSubcommand {
        name: "send",
        run: |rest, _format| send(rest),
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
    "qol sessions\n\nDiscover live terminal sessions and deliver text into them.\n\nUsage:\n  qol sessions\n  qol sessions list [--json]\n  qol sessions send <session> <text...> [--submit]\n  qol sessions read <session>\n  qol sessions wait <session> [--timeout-ms N] [--expect TEXT]\n  qol sessions focus <session>\n  qol sessions mcp\n  qol sessions export [pi]\n  qol sessions help\n  qol help sessions\n\nDetails:\n  Sessions come from the shared terminal-sessions backends (kitty remote control).\n  A session is a stable token like v1:kitty:1:42; list prints them with the\n  interpreted tool and activity hint from the shared CLI interpreter.\n  send delivers text to the session's CLI; --submit appends Enter.\n  read prints the current screen text of the session.\n  wait blocks until the screen settles (changed then stable) or shows the\n  expected text outside the echo of the last send and settles again, then\n  prints JSON with settled, screen, polls, and elapsed_ms;\n  settled=false means the timeout elapsed (clamped 1s..600s, default 30s).\n  focus raises the session's window.\n  mcp runs a Model Context Protocol server over stdio exposing these tools\n  (sessions_list, session_read_screen, session_send_text,\n  session_wait_output, session_focus); qol sessions mcp --help prints its\n  usage. Delivery is synchronous: send returns after the text is delivered,\n  wait_output blocks until the screen settles or an expected substring\n  appears outside the echo of the last send and the screen settles again.\n  export renders a per-client agent surface from the shared tool contract\n  in sessions/contract.rs; qol sessions export pi prints the pi extension\n  source, qol sessions export with no client lists the available clients.\n\nOutput:\n  Plain text on stdout by default; list --json emits structured rows.\n\nExit:\n  Exits non-zero when discovery, identity, capability, or delivery fails.\n  mcp exits zero on EOF.\n"
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
    Ok(())
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
    use qol_terminal_sessions::SessionCapabilities;

    #[test]
    fn capability_names_reflect_flags() {
        let mut caps = SessionCapabilities::NONE;
        assert!(contract::capability_names(&caps).is_empty());
        caps = SessionCapabilities::SCREEN_READING | SessionCapabilities::TEXT_INPUT;
        assert_eq!(contract::capability_names(&caps), ["read", "input"]);
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
}
