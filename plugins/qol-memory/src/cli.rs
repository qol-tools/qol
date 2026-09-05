use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{Command, CommandContext, Execution, HeadlessApp};
use serde_json::{json, Value};

use crate::ask::{render_text, AskOutput, AskRequest, LogOptions};
use crate::store::Store;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "qol-memory";
const DEFAULT_K: usize = 5;
const DEFAULT_LOG_SOURCE: &str = "ask-cli";

const USAGE_ASK: &str = concat!(
    "usage: qol-memory ask \"<query>\" [--k N] [--exclude-session ID] [--brief] ",
    "[--log-source S] [--log-cwd PATH] [--log-fact FACT] [--no-log] [--store PATH] [--agent-home DIR]"
);
const USAGE_STATUS: &str = "usage: qol-memory status [--store PATH]";
const USAGE_RUN: &str = "usage: qol-memory run";
const USAGE_CAPTURE: &str =
    "usage: qol-memory capture (--unit '<json>' | --text '<fact>' --cwd PATH) [--store PATH] [--agent-home DIR]";
const USAGE_CONTINUE: &str =
    "usage: qol-memory continue --cwd PATH --session ID [--store PATH] [--agent-home DIR]";
const USAGE_REINDEX: &str = "usage: qol-memory reindex [--store PATH]";
const USAGE_DISTILL: &str = "usage: qol-memory distill [--store PATH]";
const USAGE_ROWS: &str = "usage: qol-memory rows \"<query>\" [--store PATH] [--agent-home DIR]";

type PlainHandler = Box<dyn Fn(&CommandContext) -> Result<Execution> + Send + Sync>;
type JsonHandler = Box<dyn Fn(&CommandContext) -> Result<Value> + Send + Sync>;

struct Handlers {
    ask_plain: PlainHandler,
    ask_json: JsonHandler,
    status_plain: PlainHandler,
    status_json: JsonHandler,
    run_plain: PlainHandler,
    capture_plain: PlainHandler,
    capture_json: JsonHandler,
    continue_plain: PlainHandler,
    continue_json: JsonHandler,
    reindex_plain: PlainHandler,
    reindex_json: JsonHandler,
    distill_plain: PlainHandler,
    distill_json: JsonHandler,
    rows_plain: PlainHandler,
    rows_json: JsonHandler,
}

impl Handlers {
    fn live() -> Handlers {
        Handlers {
            ask_plain: Box::new(run_ask_plain),
            ask_json: Box::new(run_ask_json),
            status_plain: Box::new(run_status_plain),
            status_json: Box::new(run_status_json),
            run_plain: Box::new(run_run_plain),
            capture_plain: Box::new(run_capture_plain),
            capture_json: Box::new(run_capture_json),
            continue_plain: Box::new(run_continue_plain),
            continue_json: Box::new(run_continue_json),
            reindex_plain: Box::new(run_reindex_plain),
            reindex_json: Box::new(run_reindex_json),
            distill_plain: Box::new(run_distill_plain),
            distill_json: Box::new(run_distill_json),
            rows_plain: Box::new(run_rows_plain),
            rows_json: Box::new(run_rows_json),
        }
    }
}

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(Handlers::live())
}

fn app_with_handlers(handlers: Handlers) -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Long-context memory: answer questions from your settled agent session history.")
        .default_command(["status"])
        .command(ask_command(handlers.ask_plain, handlers.ask_json))
        .command(status_command(handlers.status_plain, handlers.status_json))
        .command(run_command(handlers.run_plain))
        .command(capture_command(
            handlers.capture_plain,
            handlers.capture_json,
        ))
        .command(continue_command(
            handlers.continue_plain,
            handlers.continue_json,
        ))
        .command(reindex_command(
            handlers.reindex_plain,
            handlers.reindex_json,
        ))
        .command(distill_command(
            handlers.distill_plain,
            handlers.distill_json,
        ))
        .command(rows_command(handlers.rows_plain, handlers.rows_json))
        .doctor_checks(crate::doctor::checks())
}

fn ask_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("ask")
        .about("Answer a question from your agent history memory.")
        .usage(format!(
            "{BINARY_NAME} ask \"<query>\" [--k N] [--exclude-session ID] [--brief] \
             [--log-source S] [--log-cwd PATH] [--log-fact FACT] [--no-log] [--store PATH] \
             [--agent-home DIR]"
        ))
        .detail("The first positional argument is the query; quote it.")
        .detail(format!(
            "Defaults: k={DEFAULT_K}, log-source={DEFAULT_LOG_SOURCE}. Pass --no-log to skip the retrieval log."
        ))
        .output(
            "verdict, reason, answer, recalled, and skills lines in plain text; \
             the full result object with --json.",
        )
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn status_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("status")
        .about("Show the state of the local memory store.")
        .usage(format!("{BINARY_NAME} status [--store PATH]"))
        .output("key: value lines in plain text; the status object with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn run_command(plain: PlainHandler) -> Command {
    Command::new("run")
        .alias("daemon")
        .about("Run the resident memory daemon.")
        .usage(USAGE_RUN)
        .detail("The legacy `daemon` command is an alias.")
        .exit_behavior("Runs until stopped; exits non-zero if daemon startup fails.")
        .run_result(move |context| plain(context))
}

fn capture_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("capture")
        .about("Append one settled fact or one whole unit to the memory store.")
        .usage(USAGE_CAPTURE)
        .detail("Pass a fact with --text and --cwd, or a whole unit as a JSON object with --unit.")
        .detail("--agent-home names the caller's agent home; it is stamped on the unit.")
        .output("The `appended: <n>` line in plain text; the appended count with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn continue_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("continue")
        .about("Print the units that landed since the per-cwd continuation marker.")
        .usage(USAGE_CONTINUE)
        .detail("--agent-home names the caller's agent home used for visibility.")
        .output(
            "The continuation block in plain text when units were injected; \
             nothing otherwise; the outcome object with --json.",
        )
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn rows_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("rows")
        .about("Print the launcher rows for a question.")
        .usage(USAGE_ROWS)
        .detail("Rows are the answer, the recalled units and the skill hits, in that order.")
        .output("One line per row: title, a tab, then the subtitle; the rows object with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn reindex_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("reindex")
        .about("Drop the persisted BM25 indexes and rebuild them.")
        .usage(USAGE_REINDEX)
        .output("The `reindexed: <layers>` line in plain text; the layer list with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

fn distill_command(plain: PlainHandler, json: JsonHandler) -> Command {
    Command::new("distill")
        .about("Rewrite the notes layer from compaction units, carrying decision notes forward.")
        .usage(USAGE_DISTILL)
        .output("The `distill: ...` result line in plain text; the report object with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(move |context| plain(context))
        .run_json(move |context| json(context))
}

#[derive(Debug)]
struct CliAskInvocation {
    request: AskRequest,
    log_options: LogOptions,
    store: Option<PathBuf>,
}

fn parse_ask_invocation(args: &[String]) -> std::result::Result<CliAskInvocation, String> {
    let mut request = AskRequest {
        query: String::new(),
        k: DEFAULT_K,
        brief: false,
        exclude_session: None,
        agent_home: None,
    };
    let mut log_options = LogOptions {
        source: DEFAULT_LOG_SOURCE.to_string(),
        cwd: None,
        fact: None,
        no_log: false,
    };
    let mut store: Option<PathBuf> = None;
    let mut query: Option<String> = None;

    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--k" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage_error("--k requires a value"))?;
                request.k = value
                    .parse::<usize>()
                    .map_err(|_| usage_error(&format!("--k expects a number, got `{value}`")))?;
                index += 2;
            }
            "--exclude-session" => {
                request.exclude_session =
                    Some(value_flag(args, index, "--exclude-session")?.to_string());
                index += 2;
            }
            "--agent-home" => {
                request.agent_home = Some(value_flag(args, index, "--agent-home")?.to_string());
                index += 2;
            }
            "--log-source" => {
                log_options.source = value_flag(args, index, "--log-source")?.to_string();
                index += 2;
            }
            "--log-cwd" => {
                log_options.cwd = Some(value_flag(args, index, "--log-cwd")?.to_string());
                index += 2;
            }
            "--log-fact" => {
                log_options.fact = Some(value_flag(args, index, "--log-fact")?.to_string());
                index += 2;
            }
            "--store" => {
                store = Some(PathBuf::from(value_flag(args, index, "--store")?));
                index += 2;
            }
            "--brief" => {
                request.brief = true;
                index += 1;
            }
            "--no-log" => {
                log_options.no_log = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(usage_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                if query.is_some() {
                    return Err(usage_error("expected a single quoted query"));
                }
                query = Some(positional.to_string());
                index += 1;
            }
        }
    }

    let query = match query {
        Some(query) => query,
        None => return Err(usage_error("missing query")),
    };
    request.query = query;
    request.agent_home =
        Some(qol_agent_homes::Registry::load().resolve_caller(request.agent_home.as_deref()));
    Ok(CliAskInvocation {
        request,
        log_options,
        store,
    })
}

fn parse_status_invocation(args: &[String]) -> std::result::Result<Option<PathBuf>, String> {
    let mut store = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--store" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| usage_status_error("--store requires a value"))?;
                store = Some(PathBuf::from(value.as_str()));
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_status_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                return Err(usage_status_error(&format!(
                    "unexpected argument `{positional}`"
                )));
            }
        }
    }
    Ok(store)
}

#[derive(Debug)]
struct CaptureInvocation {
    unit: Value,
    store: Option<PathBuf>,
    agent_home: Option<String>,
}

fn parse_capture_invocation(args: &[String]) -> std::result::Result<CaptureInvocation, String> {
    let mut unit: Option<Value> = None;
    let mut text: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut store: Option<PathBuf> = None;
    let mut agent_home: Option<String> = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--unit" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| usage_capture_error("--unit requires a value"))?;
                let parsed: Value = serde_json::from_str(raw)
                    .map_err(|_| usage_capture_error("--unit expects a JSON object"))?;
                if !parsed.is_object() {
                    return Err(usage_capture_error("--unit expects a JSON object"));
                }
                unit = Some(parsed);
                index += 2;
            }
            "--text" => {
                text = Some(value_flag_with(args, index, "--text", USAGE_CAPTURE)?.to_string());
                index += 2;
            }
            "--cwd" => {
                cwd = Some(value_flag_with(args, index, "--cwd", USAGE_CAPTURE)?.to_string());
                index += 2;
            }
            "--store" => {
                store = Some(PathBuf::from(value_flag_with(
                    args,
                    index,
                    "--store",
                    USAGE_CAPTURE,
                )?));
                index += 2;
            }
            "--agent-home" => {
                let value = value_flag_with(args, index, "--agent-home", USAGE_CAPTURE)?;
                agent_home = Some(value.to_string());
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_capture_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                return Err(usage_capture_error(&format!(
                    "unexpected argument `{positional}`"
                )));
            }
        }
    }
    let unit = if let Some(unit) = unit {
        if text.is_some() || cwd.is_some() {
            return Err(usage_capture_error(
                "--unit cannot be combined with --text or --cwd",
            ));
        }
        unit
    } else {
        let (text, cwd) = match (text, cwd) {
            (Some(text), Some(cwd)) => (text, cwd),
            (Some(_), None) => return Err(usage_capture_error("--text requires --cwd")),
            (None, Some(_)) => return Err(usage_capture_error("--cwd requires --text")),
            (None, None) => return Err(usage_capture_error("--unit is required")),
        };
        if text.trim().is_empty() {
            return Err(usage_capture_error("--text must not be empty"));
        }
        if cwd.trim().is_empty() {
            return Err(usage_capture_error("--cwd must not be empty"));
        }
        crate::ingest::capture_unit(text.trim(), cwd.trim(), &crate::text::now_iso())
    };
    let agent_home = Some(qol_agent_homes::Registry::load().resolve_caller(agent_home.as_deref()));
    Ok(CaptureInvocation {
        unit,
        store,
        agent_home,
    })
}

struct ContinueInvocation {
    cwd: String,
    session: String,
    store: Option<PathBuf>,
    agent_home: Option<String>,
}

fn parse_continue_invocation(args: &[String]) -> std::result::Result<ContinueInvocation, String> {
    let mut cwd: Option<String> = None;
    let mut session: Option<String> = None;
    let mut store: Option<PathBuf> = None;
    let mut agent_home: Option<String> = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--cwd" => {
                cwd = Some(value_flag_with(args, index, "--cwd", USAGE_CONTINUE)?.to_string());
                index += 2;
            }
            "--session" => {
                session =
                    Some(value_flag_with(args, index, "--session", USAGE_CONTINUE)?.to_string());
                index += 2;
            }
            "--store" => {
                store = Some(PathBuf::from(value_flag_with(
                    args,
                    index,
                    "--store",
                    USAGE_CONTINUE,
                )?));
                index += 2;
            }
            "--agent-home" => {
                let value = value_flag_with(args, index, "--agent-home", USAGE_CONTINUE)?;
                agent_home = Some(value.to_string());
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_continue_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                return Err(usage_continue_error(&format!(
                    "unexpected argument `{positional}`"
                )));
            }
        }
    }
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => return Err(usage_continue_error("--cwd is required")),
    };
    let session = match session {
        Some(session) => session,
        None => return Err(usage_continue_error("--session is required")),
    };
    let agent_home = Some(qol_agent_homes::Registry::load().resolve_caller(agent_home.as_deref()));
    Ok(ContinueInvocation {
        cwd,
        session,
        store,
        agent_home,
    })
}

struct RowsInvocation {
    query: String,
    store: Option<PathBuf>,
    agent_home: Option<String>,
}

fn parse_rows_invocation(args: &[String]) -> std::result::Result<RowsInvocation, String> {
    let mut query: Option<String> = None;
    let mut store: Option<PathBuf> = None;
    let mut agent_home: Option<String> = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--store" => {
                store = Some(PathBuf::from(value_flag_with(
                    args, index, "--store", USAGE_ROWS,
                )?));
                index += 2;
            }
            "--agent-home" => {
                let value = value_flag_with(args, index, "--agent-home", USAGE_ROWS)?;
                agent_home = Some(value.to_string());
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_rows_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                if query.is_some() {
                    return Err(usage_rows_error("expected a single quoted query"));
                }
                query = Some(positional.to_string());
                index += 1;
            }
        }
    }
    let query = match query {
        Some(query) => query,
        None => return Err(usage_rows_error("missing query")),
    };
    let agent_home = Some(qol_agent_homes::Registry::load().resolve_caller(agent_home.as_deref()));
    Ok(RowsInvocation {
        query,
        store,
        agent_home,
    })
}

struct ReindexInvocation {
    store: Option<PathBuf>,
}

fn parse_reindex_invocation(args: &[String]) -> std::result::Result<ReindexInvocation, String> {
    let mut store: Option<PathBuf> = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--store" => {
                store = Some(PathBuf::from(value_flag_with(
                    args,
                    index,
                    "--store",
                    USAGE_REINDEX,
                )?));
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_reindex_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                return Err(usage_reindex_error(&format!(
                    "unexpected argument `{positional}`"
                )));
            }
        }
    }
    Ok(ReindexInvocation { store })
}

struct DistillInvocation {
    store: Option<PathBuf>,
}

fn parse_distill_invocation(args: &[String]) -> std::result::Result<DistillInvocation, String> {
    let mut store: Option<PathBuf> = None;
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        match token {
            "--store" => {
                store = Some(PathBuf::from(value_flag_with(
                    args,
                    index,
                    "--store",
                    USAGE_DISTILL,
                )?));
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(usage_distill_error(&format!("unknown flag `{other}`")));
            }
            positional => {
                return Err(usage_distill_error(&format!(
                    "unexpected argument `{positional}`"
                )));
            }
        }
    }
    Ok(DistillInvocation { store })
}

fn value_flag<'a>(
    args: &'a [String],
    index: usize,
    name: &str,
) -> std::result::Result<&'a str, String> {
    value_flag_with(args, index, name, USAGE_ASK)
}

fn value_flag_with<'a>(
    args: &'a [String],
    index: usize,
    name: &str,
    usage: &str,
) -> std::result::Result<&'a str, String> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{name} requires a value\n{usage}"))?;
    Ok(value.as_str())
}

fn usage_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_ASK}")
}

fn usage_status_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_STATUS}")
}

fn usage_run_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_RUN}")
}

fn usage_capture_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_CAPTURE}")
}

fn usage_continue_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_CONTINUE}")
}

fn usage_reindex_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_REINDEX}")
}

fn usage_distill_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_DISTILL}")
}

fn usage_rows_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_ROWS}")
}

fn run_ask_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_ask_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    if let Some(output) = ask_via_socket(&invocation)? {
        return Ok(Execution::success(newline_terminated(render_text(&output))));
    }
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    let aliases = crate::aliases::embedded();
    let output = crate::ask::run_and_log(
        &store,
        &aliases,
        &invocation.request,
        &invocation.log_options,
    )?;
    Ok(Execution::success(newline_terminated(render_text(&output))))
}

fn run_ask_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_ask_invocation(context.args()).map_err(anyhow::Error::msg)?;
    if let Some(output) = ask_via_socket(&invocation)? {
        return serde_json::to_value(&output).context("failed to serialize the ask result");
    }
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    let aliases = crate::aliases::embedded();
    let output = crate::ask::run_and_log(
        &store,
        &aliases,
        &invocation.request,
        &invocation.log_options,
    )?;
    serde_json::to_value(&output).context("failed to serialize the ask result")
}

fn ask_via_socket(invocation: &CliAskInvocation) -> Result<Option<AskOutput>> {
    if invocation.store.is_some() {
        return Ok(None);
    }
    let input = json!({
        "query": invocation.request.query,
        "k": invocation.request.k,
        "brief": invocation.request.brief,
        "exclude_session": invocation.request.exclude_session,
        "agent_home": invocation.request.agent_home,
        "log_source": invocation.log_options.source,
        "log_cwd": invocation.log_options.cwd,
        "log_fact": invocation.log_options.fact,
        "no_log": invocation.log_options.no_log,
    });
    match crate::app::send_request("ask", input) {
        Ok(Some(value)) => Ok(Some(
            serde_json::from_value(value).context("unexpected qol-memory daemon ask payload")?,
        )),
        Ok(None) => anyhow::bail!("qol-memory daemon returned no ask payload"),
        Err(error) if !crate::app::daemon_unreachable(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

fn run_status_plain(context: &CommandContext) -> Result<Execution> {
    let store_path = match parse_status_invocation(context.args()) {
        Ok(store_path) => store_path,
        Err(message) => return Ok(Execution::usage(message)),
    };
    if let Some(value) = status_via_socket(store_path.as_deref())? {
        return Ok(Execution::success(newline_terminated(flatten_status(
            &value,
        ))));
    }
    let store =
        Store::resolve(store_path.as_deref()).context("failed to resolve the qol-memory store")?;
    let value = crate::ask::status(&store)?;
    Ok(Execution::success(newline_terminated(flatten_status(
        &value,
    ))))
}

fn run_status_json(context: &CommandContext) -> Result<Value> {
    let store_path = parse_status_invocation(context.args()).map_err(anyhow::Error::msg)?;
    if let Some(value) = status_via_socket(store_path.as_deref())? {
        return Ok(value);
    }
    let store =
        Store::resolve(store_path.as_deref()).context("failed to resolve the qol-memory store")?;
    crate::ask::status(&store)
}

fn status_via_socket(store: Option<&Path>) -> Result<Option<Value>> {
    if store.is_some() {
        return Ok(None);
    }
    match crate::app::send_request("status", json!({})) {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => anyhow::bail!("qol-memory daemon returned no status payload"),
        Err(error) if !crate::app::daemon_unreachable(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

fn run_run_plain(context: &CommandContext) -> Result<Execution> {
    if let Err(message) = parse_run_invocation(context.args()) {
        return Ok(Execution::usage(message));
    }
    match crate::app::run_daemon() {
        Ok(()) => Ok(Execution::success("")),
        Err(error) => Ok(Execution::runtime_error(format!("{PLUGIN_ID}: {error:#}"))),
    }
}

fn parse_run_invocation(args: &[String]) -> std::result::Result<(), String> {
    match args.first() {
        Some(token) if token.starts_with("--") => {
            Err(usage_run_error(&format!("unknown flag `{token}`")))
        }
        Some(token) => Err(usage_run_error(&format!("unexpected argument `{token}`"))),
        None => Ok(()),
    }
}

fn run_capture_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_capture_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let appended = capture_appended(&invocation)?;
    Ok(Execution::success(newline_terminated(format!(
        "appended: {appended}"
    ))))
}

fn run_capture_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_capture_invocation(context.args()).map_err(anyhow::Error::msg)?;
    let appended = capture_appended(&invocation)?;
    Ok(json!({ "appended": appended }))
}

fn capture_appended(invocation: &CaptureInvocation) -> Result<usize> {
    if invocation.store.is_none() {
        let input = json!({
            "unit": invocation.unit.clone(),
            "agent_home": invocation.agent_home,
        });
        match crate::app::send_request("capture", input) {
            Ok(Some(value)) => {
                let appended = value
                    .get("appended")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        anyhow::anyhow!("qol-memory daemon returned no appended count")
                    })?;
                return Ok(appended as usize);
            }
            Ok(None) => anyhow::bail!("qol-memory daemon returned no capture payload"),
            Err(error) if !crate::app::daemon_unreachable(&error) => return Err(error),
            Err(_) => {}
        }
    }
    in_process_capture(invocation)
}

fn in_process_capture(invocation: &CaptureInvocation) -> Result<usize> {
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    let mut unit = invocation.unit.clone();
    if let Some(fields) = unit.as_object_mut() {
        if let Some(agent_home) = invocation.agent_home.as_deref() {
            fields.insert("agent_home".to_string(), json!(agent_home));
        }
    }
    let mut keys = crate::ingest::KeySet::load(&store)?;
    crate::ingest::append_units(&store, std::slice::from_ref(&unit), &mut keys)
}

fn run_continue_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_continue_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let payload = continue_payload(&invocation)?;
    let stdout = if payload.get("stage").and_then(Value::as_str) == Some("injected") {
        payload
            .get("block")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        String::new()
    };
    Ok(Execution::success(newline_terminated(stdout)))
}

fn run_continue_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_continue_invocation(context.args()).map_err(anyhow::Error::msg)?;
    continue_payload(&invocation)
}

fn continue_payload(invocation: &ContinueInvocation) -> Result<Value> {
    if invocation.store.is_none() {
        let input = json!({
            "cwd": invocation.cwd,
            "session": invocation.session,
            "agent_home": invocation.agent_home,
        });
        match crate::app::send_request("continue", input) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => anyhow::bail!("qol-memory daemon returned no continue payload"),
            Err(error) if !crate::app::daemon_unreachable(&error) => return Err(error),
            Err(_) => {}
        }
    }
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    let request = crate::continue_recall::ContinueRequest {
        cwd: invocation.cwd.clone(),
        session: invocation.session.clone(),
        agent_home: invocation.agent_home.clone(),
    };
    let outcome = crate::continue_recall::run(&store, &request)?;
    serde_json::to_value(outcome).context("failed to serialize the continue outcome")
}

fn run_rows_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_rows_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let payload = rows_payload(&invocation)?;
    Ok(Execution::success(rows_plain_lines(&payload)))
}

fn run_rows_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_rows_invocation(context.args()).map_err(anyhow::Error::msg)?;
    rows_payload(&invocation)
}

fn rows_payload(invocation: &RowsInvocation) -> Result<Value> {
    if invocation.store.is_none() {
        match crate::app::send_request(
            "rows",
            json!({ "query": invocation.query, "agent_home": invocation.agent_home }),
        ) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => anyhow::bail!("qol-memory daemon returned no rows payload"),
            Err(error) if !crate::app::daemon_unreachable(&error) => return Err(error),
            Err(_) => {}
        }
    }
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    let aliases = crate::aliases::embedded();
    let request = AskRequest {
        query: invocation.query.clone(),
        k: DEFAULT_K,
        brief: false,
        exclude_session: None,
        agent_home: invocation.agent_home.clone(),
    };
    let output = crate::ask::run_with_layers(&store, &aliases, &request, &units, &notes)?;
    let flow_rows = crate::ask::rows::from_output(&output, &units, &notes);
    Ok(json!({
        "verdict": output.verdict,
        "confidence": output.confidence,
        "rows": flow_rows,
    }))
}

fn rows_plain_lines(payload: &Value) -> String {
    let mut lines = Vec::new();
    for row in payload
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = row.get("title").and_then(Value::as_str).unwrap_or_default();
        let subtitle = row
            .get("subtitle")
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("{title}\t{subtitle}"));
    }
    newline_terminated(lines.join("\n"))
}

fn run_reindex_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_reindex_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let layers = reindex_layers(&invocation)?;
    Ok(Execution::success(newline_terminated(format!(
        "reindexed: {}",
        layers.join(", ")
    ))))
}

fn run_reindex_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_reindex_invocation(context.args()).map_err(anyhow::Error::msg)?;
    let layers = reindex_layers(&invocation)?;
    Ok(json!({ "layers": layers }))
}

fn reindex_layers(invocation: &ReindexInvocation) -> Result<Vec<String>> {
    if invocation.store.is_none() {
        match crate::app::send_request("reindex", json!({})) {
            Ok(Some(value)) => {
                let layers = value
                    .get("layers")
                    .and_then(Value::as_array)
                    .map(|array| {
                        array
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .ok_or_else(|| anyhow::anyhow!("qol-memory daemon returned no layers"))?;
                return Ok(layers);
            }
            Ok(None) => anyhow::bail!("qol-memory daemon returned no reindex payload"),
            Err(error) if !crate::app::daemon_unreachable(&error) => return Err(error),
            Err(_) => {}
        }
    }
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    crate::app::warm::reindex(&store)
}

fn run_distill_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_distill_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let report = distill_report(&invocation)?;
    let line = if report.unchanged {
        format!(
            "distill: unchanged ({})",
            report.run.as_deref().unwrap_or_default()
        )
    } else {
        format!(
            "distill: run {} added {} carried {} dropped {}",
            report.run.as_deref().unwrap_or_default(),
            report.added,
            report.carried,
            report.dropped
        )
    };
    Ok(Execution::success(newline_terminated(line)))
}

fn run_distill_json(context: &CommandContext) -> Result<Value> {
    let invocation = parse_distill_invocation(context.args()).map_err(anyhow::Error::msg)?;
    let report = distill_report(&invocation)?;
    serde_json::to_value(&report).context("failed to serialize the distill report")
}

fn distill_report(invocation: &DistillInvocation) -> Result<crate::distill::DistillReport> {
    let store = Store::resolve(invocation.store.as_deref())
        .context("failed to resolve the qol-memory store")?;
    crate::distill::run(&store)
}

fn flatten_status(value: &Value) -> String {
    let mut lines = Vec::new();
    push_status_lines("", value, &mut lines);
    lines.join("\n")
}

fn push_status_lines(key: &str, value: &Value, lines: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map {
                let child_path = if key.is_empty() {
                    child_key.clone()
                } else {
                    format!("{key}.{child_key}")
                };
                push_status_lines(&child_path, child_value, lines);
            }
        }
        scalar => {
            let rendered = match scalar.as_str() {
                Some(text) => text.to_string(),
                None => scalar.to_string(),
            };
            if key.is_empty() {
                lines.push(rendered);
            } else {
                lines.push(format!("{key}: {rendered}"));
            }
        }
    }
}

fn newline_terminated(text: String) -> String {
    if text.is_empty() || text.ends_with('\n') {
        return text;
    }
    format!("{text}\n")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use qol_headless::{DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        ask: AtomicUsize,
        status: AtomicUsize,
        run: AtomicUsize,
        capture: AtomicUsize,
        continue_cmd: AtomicUsize,
        reindex: AtomicUsize,
        distill: AtomicUsize,
        rows: AtomicUsize,
    }

    impl OperationCalls {
        fn all_zero(&self) -> bool {
            self.ask.load(Ordering::SeqCst) == 0
                && self.status.load(Ordering::SeqCst) == 0
                && self.run.load(Ordering::SeqCst) == 0
                && self.capture.load(Ordering::SeqCst) == 0
                && self.continue_cmd.load(Ordering::SeqCst) == 0
                && self.reindex.load(Ordering::SeqCst) == 0
                && self.distill.load(Ordering::SeqCst) == 0
                && self.rows.load(Ordering::SeqCst) == 0
        }
    }

    fn sentinel_handlers(calls: &Arc<OperationCalls>) -> Handlers {
        let ask_calls = Arc::clone(calls);
        let ask_json_calls = Arc::clone(calls);
        let status_calls = Arc::clone(calls);
        let status_json_calls = Arc::clone(calls);
        let run_calls = Arc::clone(calls);
        let capture_calls = Arc::clone(calls);
        let capture_json_calls = Arc::clone(calls);
        let continue_calls = Arc::clone(calls);
        let continue_json_calls = Arc::clone(calls);
        let reindex_calls = Arc::clone(calls);
        let reindex_json_calls = Arc::clone(calls);
        let distill_calls = Arc::clone(calls);
        let distill_json_calls = Arc::clone(calls);
        let rows_calls = Arc::clone(calls);
        let rows_json_calls = Arc::clone(calls);
        Handlers {
            ask_plain: Box::new(move |_| {
                ask_calls.ask.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel ask"))
            }),
            ask_json: Box::new(move |_: &CommandContext| {
                ask_json_calls.ask.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "ask" }))
            }),
            status_plain: Box::new(move |_| {
                status_calls.status.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel status"))
            }),
            status_json: Box::new(move |_: &CommandContext| {
                status_json_calls.status.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "status" }))
            }),
            run_plain: Box::new(move |_| {
                run_calls.run.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel run"))
            }),
            capture_plain: Box::new(move |_| {
                capture_calls.capture.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel capture"))
            }),
            capture_json: Box::new(move |_: &CommandContext| {
                capture_json_calls.capture.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "capture" }))
            }),
            continue_plain: Box::new(move |_| {
                continue_calls.continue_cmd.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel continue"))
            }),
            continue_json: Box::new(move |_: &CommandContext| {
                continue_json_calls
                    .continue_cmd
                    .fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "continue" }))
            }),
            reindex_plain: Box::new(move |_| {
                reindex_calls.reindex.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel reindex"))
            }),
            reindex_json: Box::new(move |_: &CommandContext| {
                reindex_json_calls.reindex.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "reindex" }))
            }),
            distill_plain: Box::new(move |_| {
                distill_calls.distill.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel distill"))
            }),
            distill_json: Box::new(move |_: &CommandContext| {
                distill_json_calls.distill.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "distill" }))
            }),
            rows_plain: Box::new(move |_| {
                rows_calls.rows.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel rows"))
            }),
            rows_json: Box::new(move |_: &CommandContext| {
                rows_json_calls.rows.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "rows" }))
            }),
        }
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        app_with_handlers(sentinel_handlers(&calls))
    }

    fn parse_args(args: &[&str]) -> std::result::Result<CliAskInvocation, String> {
        let owned = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        parse_ask_invocation(&owned)
    }

    #[test]
    fn ask_args_parse_flags_in_any_order() {
        let first = parse_args(&["--brief", "--k", "3", "why did the build fail?", "--no-log"])
            .expect("flags before and around the query must parse");
        assert_eq!(first.request.query, "why did the build fail?");
        assert_eq!(first.request.k, 3);
        assert!(first.request.brief);
        assert!(first.log_options.no_log);
        assert_eq!(first.log_options.source, "ask-cli");
        assert_eq!(first.request.exclude_session, None);
        assert_eq!(first.log_options.cwd, None);
        assert_eq!(first.log_options.fact, None);
        assert_eq!(first.store, None);

        let second = parse_args(&["--no-log", "retry the daemon restart?", "--brief"])
            .expect("reversed flag order must parse");
        assert_eq!(second.request.query, "retry the daemon restart?");
        assert_eq!(second.request.k, DEFAULT_K);
        assert!(second.request.brief);
        assert!(second.log_options.no_log);
    }

    #[test]
    fn missing_query_is_a_usage_error() {
        for args in [Vec::<&str>::new(), vec!["--brief"], vec!["--k", "3"]] {
            let error = parse_args(&args).expect_err("missing query must be rejected");
            assert!(error.contains("missing query"), "args: {args:?}");
            assert!(error.contains(USAGE_ASK), "args: {args:?}");
        }
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let error = parse_args(&["find bugs", "--wat"]).expect_err("unknown flags are rejected");
        assert!(error.contains("unknown flag `--wat`"));
        assert!(error.contains(USAGE_ASK));
    }

    #[test]
    fn k_parses_from_its_value_token() {
        let invocation = parse_args(&["--k", "7", "what broke"]).expect("--k parses");
        assert_eq!(invocation.request.k, 7);

        let invalid =
            parse_args(&["--k", "many", "what broke"]).expect_err("non-numeric k is rejected");
        assert!(invalid.contains("--k expects a number"));
    }

    #[test]
    fn agent_home_flag_defaults_to_an_explicit_caller() {
        let ask = parse_args(&["query"]).expect("ask parses");
        assert!(!ask
            .request
            .agent_home
            .as_deref()
            .expect("explicit agent home")
            .is_empty());
        let flagged_ask =
            parse_args(&["--agent-home", "/tmp/h1", "query"]).expect("ask flag parses");
        assert_eq!(flagged_ask.request.agent_home.as_deref(), Some("/tmp/h1"));

        let capture =
            parse_capture_args(&["--text", "fact", "--cwd", "/p"]).expect("capture parses");
        assert!(!capture
            .agent_home
            .as_deref()
            .expect("explicit home")
            .is_empty());
        let flagged_capture =
            parse_capture_args(&["--text", "fact", "--cwd", "/p", "--agent-home", "/tmp/h2"])
                .expect("capture flag parses");
        assert_eq!(flagged_capture.agent_home.as_deref(), Some("/tmp/h2"));

        let continue_args: Vec<String> = ["--cwd", "/repo", "--session", "s"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let continue_invocation =
            parse_continue_invocation(&continue_args).expect("continue parses");
        assert!(!continue_invocation
            .agent_home
            .as_deref()
            .expect("explicit home")
            .is_empty());
        let flagged_continue: Vec<String> = [
            "--cwd",
            "/repo",
            "--session",
            "s",
            "--agent-home",
            "/tmp/h3",
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
        let flagged_continue_invocation =
            parse_continue_invocation(&flagged_continue).expect("continue flag parses");
        assert_eq!(
            flagged_continue_invocation.agent_home.as_deref(),
            Some("/tmp/h3")
        );

        let rows_args: Vec<String> = ["query"].iter().map(|arg| (*arg).to_string()).collect();
        let rows_invocation = parse_rows_invocation(&rows_args).expect("rows parses");
        assert!(!rows_invocation
            .agent_home
            .as_deref()
            .expect("explicit home")
            .is_empty());
        let flagged_rows: Vec<String> = ["--agent-home", "/tmp/h4", "query"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let flagged_rows_invocation =
            parse_rows_invocation(&flagged_rows).expect("rows flag parses");
        assert_eq!(
            flagged_rows_invocation.agent_home.as_deref(),
            Some("/tmp/h4")
        );

        let home = std::path::PathBuf::from(std::env::var("HOME").expect("HOME is set"));
        let expected = home.join("h").to_string_lossy().into_owned();
        let tilde_ask = parse_args(&["--agent-home", "~/h", "query"]).expect("ask tilde parses");
        assert_eq!(
            tilde_ask.request.agent_home.as_deref(),
            Some(expected.as_str())
        );
        let tilde_capture =
            parse_capture_args(&["--text", "fact", "--cwd", "/p", "--agent-home", "~/h/"])
                .expect("capture tilde parses");
        assert_eq!(tilde_capture.agent_home.as_deref(), Some(expected.as_str()));
        let tilde_continue: Vec<String> =
            ["--cwd", "/repo", "--session", "s", "--agent-home", "~/h"]
                .iter()
                .map(|arg| (*arg).to_string())
                .collect();
        let tilde_continue_invocation =
            parse_continue_invocation(&tilde_continue).expect("continue tilde parses");
        assert_eq!(
            tilde_continue_invocation.agent_home.as_deref(),
            Some(expected.as_str())
        );
        let tilde_rows: Vec<String> = ["--agent-home", "~/h/", "query"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let tilde_rows_invocation = parse_rows_invocation(&tilde_rows).expect("rows tilde parses");
        assert_eq!(
            tilde_rows_invocation.agent_home.as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn empty_store_status_and_ask_report_no_memories() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let store_dir = std::env::temp_dir().join(format!("qol-memory-cli-empty-{nanos}"));
        std::fs::create_dir_all(&store_dir).expect("create temp store");

        let status_run = app().execute(vec![
            "status".to_string(),
            "--store".to_string(),
            store_dir.display().to_string(),
        ]);
        assert_eq!(status_run.exit_code, 0);
        assert!(status_run.stderr.is_empty());

        let ask_run = app().execute([
            "ask".to_string(),
            "--store".to_string(),
            store_dir.display().to_string(),
            "--no-log".to_string(),
            "x".to_string(),
        ]);
        assert_eq!(ask_run.exit_code, 0);
        assert!(ask_run.stderr.is_empty());
        assert!(ask_run.stdout.contains("no-memory"));

        std::fs::remove_dir_all(&store_dir).ok();
    }

    #[test]
    fn exclude_session_log_options_flow_through_the_invocation() {
        let invocation = parse_args(&[
            "--exclude-session",
            "sess-live-aaa1",
            "--log-source",
            "tool",
            "--log-cwd",
            "/repo",
            "--log-fact",
            "fix shipped",
            "query text",
        ])
        .expect("value flags parse");
        assert_eq!(
            invocation.request.exclude_session.as_deref(),
            Some("sess-live-aaa1")
        );
        assert_eq!(invocation.log_options.source, "tool");
        assert_eq!(invocation.log_options.cwd.as_deref(), Some("/repo"));
        assert_eq!(invocation.log_options.fact.as_deref(), Some("fix shipped"));
    }

    #[test]
    fn doctor_and_help_never_invoke_operational_handlers() {
        let cases = [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help"],
            vec!["help", "ask"],
            vec!["ask", "help"],
            vec!["help", "status"],
            vec!["help", "doctor"],
            vec!["help", "run"],
            vec!["help", "capture"],
            vec!["help", "continue"],
            vec!["help", "reindex"],
            vec!["help", "distill"],
            vec!["help", "rows"],
            vec!["rows", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert!(calls.all_zero(), "args: {args:?}");
        }
    }

    #[test]
    fn run_help_lists_the_daemon_alias() {
        let calls = Arc::new(OperationCalls::default());
        let execution =
            sentinel_app(Arc::clone(&calls)).execute(["help".to_string(), "run".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(
            execution.stdout.contains("`daemon`"),
            "stdout: {}",
            execution.stdout
        );
        assert!(calls.all_zero());
    }

    #[test]
    fn distill_usage_errors_and_store_flag_parse() {
        let with_store: Vec<String> = ["--store", "/tmp/s"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let parsed = parse_distill_invocation(&with_store).expect("distill parses");
        assert_eq!(parsed.store, Some(PathBuf::from("/tmp/s")));

        let empty: Vec<String> = Vec::new();
        let bare = parse_distill_invocation(&empty).expect("bare distill parses");
        assert_eq!(bare.store, None);

        let unknown_flag = app().execute(["distill".to_string(), "--wat".to_string()]);
        assert_eq!(unknown_flag.exit_code, EXIT_USAGE);
        assert!(unknown_flag.stderr.contains(USAGE_DISTILL));

        let positional = app().execute(["distill".to_string(), "now".to_string()]);
        assert_eq!(positional.exit_code, EXIT_USAGE);
        assert!(positional.stderr.contains(USAGE_DISTILL));
    }

    #[test]
    fn rows_requires_a_query() {
        let missing = app().execute(["rows".to_string()]);
        assert_eq!(missing.exit_code, EXIT_USAGE);
        assert!(missing.stderr.contains(USAGE_ROWS));
        assert!(missing.stderr.contains("missing query"));

        let extra = app().execute([
            "rows".to_string(),
            "first".to_string(),
            "second".to_string(),
        ]);
        assert_eq!(extra.exit_code, EXIT_USAGE);
        assert!(extra.stderr.contains(USAGE_ROWS));

        let unknown_flag =
            app().execute(["rows".to_string(), "--wat".to_string(), "query".to_string()]);
        assert_eq!(unknown_flag.exit_code, EXIT_USAGE);
        assert!(unknown_flag.stderr.contains(USAGE_ROWS));
    }

    #[test]
    fn rows_help_is_listed() {
        let calls = Arc::new(OperationCalls::default());
        let execution =
            sentinel_app(Arc::clone(&calls)).execute(["help".to_string(), "rows".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(
            execution
                .stdout
                .contains("Print the launcher rows for a question."),
            "stdout: {}",
            execution.stdout
        );
        assert!(calls.all_zero());
    }

    fn parse_capture_args(args: &[&str]) -> std::result::Result<CaptureInvocation, String> {
        let owned = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        parse_capture_invocation(&owned)
    }

    #[test]
    fn capture_requires_a_json_object() {
        let cases = [
            vec!["capture"],
            vec!["capture", "--unit", "not json"],
            vec!["capture", "--unit", "\"a plain string\""],
            vec!["capture", "--unit", "[1, 2]"],
            vec!["capture", "--unit"],
        ];

        for args in cases {
            let execution = app().execute(args.iter().map(|arg| (*arg).to_string()));
            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(execution.stderr.contains(USAGE_CAPTURE), "args: {args:?}");
        }
    }

    #[test]
    fn capture_text_flags_validate() {
        let combined = parse_capture_args(&["--unit", "{}", "--text", "fact", "--cwd", "/p"])
            .expect_err("--unit excludes --text and --cwd");
        assert!(combined.contains("--unit cannot be combined with --text or --cwd"));
        assert!(combined.contains(USAGE_CAPTURE));

        let text_only = parse_capture_args(&["--text", "fact"]).expect_err("--text requires --cwd");
        assert!(text_only.contains("--text requires --cwd"));
        assert!(text_only.contains(USAGE_CAPTURE));

        let cwd_only = parse_capture_args(&["--cwd", "/p"]).expect_err("--cwd requires --text");
        assert!(cwd_only.contains("--cwd requires --text"));
        assert!(cwd_only.contains(USAGE_CAPTURE));

        let blank_text = parse_capture_args(&["--text", "   ", "--cwd", "/p"])
            .expect_err("blank --text is rejected");
        assert!(blank_text.contains("--text must not be empty"));
        assert!(blank_text.contains(USAGE_CAPTURE));

        let blank_cwd = parse_capture_args(&["--text", "fact", "--cwd", "  "])
            .expect_err("blank --cwd is rejected");
        assert!(blank_cwd.contains("--cwd must not be empty"));
        assert!(blank_cwd.contains(USAGE_CAPTURE));
    }

    #[test]
    fn capture_text_builds_a_capture_unit() {
        let invocation =
            parse_capture_args(&["--text", " fact ", "--cwd", "/p"]).expect("text flags parse");
        let unit = invocation.unit;
        assert_eq!(unit["kind"], "capture");
        assert_eq!(unit["cwd"], "/p");
        assert_eq!(unit["text"], "fact");
        assert_eq!(
            unit["key"],
            crate::ingest::capture_unit("fact", "/p", "x")["key"]
        );
    }

    #[test]
    fn continue_requires_cwd_and_session() {
        let cases = [
            vec!["continue"],
            vec!["continue", "--cwd", "/repo"],
            vec!["continue", "--session", "sess-live-aaa1"],
        ];

        for args in cases {
            let execution = app().execute(args.iter().map(|arg| (*arg).to_string()));
            assert_eq!(execution.exit_code, EXIT_USAGE, "args: {args:?}");
            assert!(execution.stderr.contains(USAGE_CONTINUE), "args: {args:?}");
        }
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        let first = app().execute(["help".to_string(), "ask".to_string()]);
        let final_token = app().execute(["ask".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Supports --json."));
    }

    #[test]
    fn doctor_help_first_and_final_are_equivalent() {
        let first = app().execute(["help".to_string(), "doctor".to_string()]);
        let final_token = app().execute(["doctor".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Run read-only health checks."));
        assert!(first.stdout.contains("platform_supported"));
        assert!(first.stdout.contains("store_dir"));
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_flag_positions() {
        let before = app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = app().execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 9);
        let ids = report
            .checks
            .iter()
            .map(|check| check.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "platform_supported",
                "agent_homes",
                "store_dir",
                "units_layer",
                "notes_layer",
                "index_cache",
                "skills_index",
                "retrieval_log",
                "aliases_valid"
            ]
        );
        assert!(report
            .checks
            .iter()
            .all(|check| !check.id.is_empty() && !check.message.is_empty()));
    }

    #[test]
    fn ask_usage_errors_exit_64_with_the_usage_line() {
        let missing = app().execute(["ask".to_string()]);
        assert_eq!(missing.exit_code, EXIT_USAGE);
        assert!(missing.stderr.contains(USAGE_ASK));

        let extra_positional =
            app().execute(["ask".to_string(), "first".to_string(), "second".to_string()]);
        assert_eq!(extra_positional.exit_code, EXIT_USAGE);

        let unknown_flag =
            app().execute(["ask".to_string(), "--wat".to_string(), "query".to_string()]);
        assert_eq!(unknown_flag.exit_code, EXIT_USAGE);
    }

    #[test]
    fn status_usage_errors_exit_64_with_the_usage_line() {
        let unexpected = app().execute(["status".to_string(), "surprise".to_string()]);
        assert_eq!(unexpected.exit_code, EXIT_USAGE);
        assert!(unexpected.stderr.contains(USAGE_STATUS));

        let unknown_flag = app().execute(["status".to_string(), "--wat".to_string()]);
        assert_eq!(unknown_flag.exit_code, EXIT_USAGE);
        assert!(unknown_flag.stderr.contains(USAGE_STATUS));
    }

    #[test]
    fn capture_and_continue_and_reindex_usage_errors_exit_64() {
        let capture_extra = app().execute([
            "capture".to_string(),
            "--unit".to_string(),
            "{}".to_string(),
            "extra".to_string(),
        ]);
        assert_eq!(capture_extra.exit_code, EXIT_USAGE);

        let continue_unknown = app().execute([
            "continue".to_string(),
            "--cwd".to_string(),
            "/repo".to_string(),
            "--session".to_string(),
            "s".to_string(),
            "--wat".to_string(),
        ]);
        assert_eq!(continue_unknown.exit_code, EXIT_USAGE);
        assert!(continue_unknown.stderr.contains(USAGE_CONTINUE));

        let reindex_unknown = app().execute(["reindex".to_string(), "--wat".to_string()]);
        assert_eq!(reindex_unknown.exit_code, EXIT_USAGE);
        assert!(reindex_unknown.stderr.contains(USAGE_REINDEX));
    }

    #[test]
    fn global_json_plus_help_is_rejected_as_usage() {
        let execution = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["--json".to_string(), "help".to_string()]);

        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }
}
