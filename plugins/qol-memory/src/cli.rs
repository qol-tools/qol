use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{Command, CommandContext, Execution, HeadlessApp};

use crate::ask::{render_text, AskRequest, LogOptions};
use crate::store::Store;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "qol-memory";
const DEFAULT_K: usize = 5;
const DEFAULT_LOG_SOURCE: &str = "ask-cli";

const USAGE_ASK: &str = concat!(
    "usage: qol-memory ask \"<query>\" [--k N] [--exclude-session ID] [--brief] ",
    "[--log-source S] [--log-cwd PATH] [--log-fact FACT] [--no-log] [--store PATH]"
);
const USAGE_STATUS: &str = "usage: qol-memory status [--store PATH]";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(
        run_ask_plain,
        run_ask_json,
        run_status_plain,
        run_status_json,
    )
}

fn app_with_handlers<AP, AJ, SP, SJ>(
    ask_plain: AP,
    ask_json: AJ,
    status_plain: SP,
    status_json: SJ,
) -> HeadlessApp
where
    AP: Fn(&CommandContext) -> Result<Execution> + Send + Sync + 'static,
    AJ: Fn(&CommandContext) -> Result<serde_json::Value> + Send + Sync + 'static,
    SP: Fn(&CommandContext) -> Result<Execution> + Send + Sync + 'static,
    SJ: Fn(&CommandContext) -> Result<serde_json::Value> + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Long-context memory: answer questions from your settled agent session history.")
        .default_command(["status"])
        .command(ask_command(ask_plain, ask_json))
        .command(status_command(status_plain, status_json))
        .doctor_checks(crate::doctor::checks())
}

fn ask_command<AP, AJ>(plain: AP, json: AJ) -> Command
where
    AP: Fn(&CommandContext) -> Result<Execution> + Send + Sync + 'static,
    AJ: Fn(&CommandContext) -> Result<serde_json::Value> + Send + Sync + 'static,
{
    Command::new("ask")
        .about("Answer a question from your agent history memory.")
        .usage(format!(
            "{BINARY_NAME} ask \"<query>\" [--k N] [--exclude-session ID] [--brief] \
             [--log-source S] [--log-cwd PATH] [--log-fact FACT] [--no-log] [--store PATH]"
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
        .run_result(plain)
        .run_json(json)
}

fn status_command<SP, SJ>(plain: SP, json: SJ) -> Command
where
    SP: Fn(&CommandContext) -> Result<Execution> + Send + Sync + 'static,
    SJ: Fn(&CommandContext) -> Result<serde_json::Value> + Send + Sync + 'static,
{
    Command::new("status")
        .about("Show the state of the local memory store.")
        .usage(format!("{BINARY_NAME} status [--store PATH]"))
        .output("key: value lines in plain text; the status object with --json.")
        .exit_behavior("Usage errors exit 64; failures exit 1.")
        .run_result(plain)
        .run_json(json)
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

fn value_flag<'a>(
    args: &'a [String],
    index: usize,
    name: &str,
) -> std::result::Result<&'a str, String> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| usage_error(&format!("{name} requires a value")))?;
    Ok(value.as_str())
}

fn usage_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_ASK}")
}

fn usage_status_error(detail: &str) -> String {
    format!("{detail}\n{USAGE_STATUS}")
}

fn run_ask_plain(context: &CommandContext) -> Result<Execution> {
    let invocation = match parse_ask_invocation(context.args()) {
        Ok(invocation) => invocation,
        Err(message) => return Ok(Execution::usage(message)),
    };
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

fn run_ask_json(context: &CommandContext) -> Result<serde_json::Value> {
    let invocation = parse_ask_invocation(context.args()).map_err(anyhow::Error::msg)?;
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

fn run_status_plain(context: &CommandContext) -> Result<Execution> {
    let store_path = match parse_status_invocation(context.args()) {
        Ok(store_path) => store_path,
        Err(message) => return Ok(Execution::usage(message)),
    };
    let store =
        Store::resolve(store_path.as_deref()).context("failed to resolve the qol-memory store")?;
    let value = crate::ask::status(&store)?;
    Ok(Execution::success(newline_terminated(flatten_status(
        &value,
    ))))
}

fn run_status_json(context: &CommandContext) -> Result<serde_json::Value> {
    let store_path = parse_status_invocation(context.args()).map_err(anyhow::Error::msg)?;
    let store =
        Store::resolve(store_path.as_deref()).context("failed to resolve the qol-memory store")?;
    crate::ask::status(&store)
}

fn flatten_status(value: &serde_json::Value) -> String {
    let mut lines = Vec::new();
    push_status_lines("", value, &mut lines);
    lines.join("\n")
}

fn push_status_lines(key: &str, value: &serde_json::Value, lines: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
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

    use qol_headless::{DoctorReport, EXIT_RUNTIME_ERROR, EXIT_SUCCESS, EXIT_USAGE};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        ask: AtomicUsize,
        status: AtomicUsize,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let ask_calls = Arc::clone(&calls);
        let ask_json_calls = Arc::clone(&calls);
        let status_calls = Arc::clone(&calls);
        let status_json_calls = Arc::clone(&calls);
        app_with_handlers(
            move |_| {
                ask_calls.ask.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel ask"))
            },
            move |_: &CommandContext| {
                ask_json_calls.ask.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "ask" }))
            },
            move |_| {
                status_calls.status.fetch_add(1, Ordering::SeqCst);
                Ok(Execution::success("sentinel status"))
            },
            move |_: &CommandContext| {
                status_json_calls.status.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "sentinel": "status" }))
            },
        )
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
    fn empty_store_status_and_ask_exit_one_with_the_store_error() {
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
        assert_eq!(status_run.exit_code, EXIT_RUNTIME_ERROR);
        assert!(
            status_run.stderr.contains("no runs under"),
            "stderr: {}",
            status_run.stderr
        );
        assert!(status_run.stdout.is_empty());

        let ask_run = app().execute([
            "ask".to_string(),
            "--store".to_string(),
            store_dir.display().to_string(),
            "--no-log".to_string(),
            "x".to_string(),
        ]);
        assert_eq!(ask_run.exit_code, EXIT_RUNTIME_ERROR);
        assert!(ask_run.stdout.is_empty());

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
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert_eq!(calls.ask.load(Ordering::SeqCst), 0, "args: {args:?}");
            assert_eq!(calls.status.load(Ordering::SeqCst), 0, "args: {args:?}");
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
        assert_eq!(report.checks.len(), 8);
        let ids = report
            .checks
            .iter()
            .map(|check| check.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "platform_supported",
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
    fn global_json_plus_help_is_rejected_as_usage() {
        let execution = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["--json".to_string(), "help".to_string()]);

        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }
}
