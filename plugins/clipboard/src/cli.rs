use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};
use serde_json::{json, Value};

use crate::app::rows;
use crate::capture::{CaptureError, Support};
use crate::history::{now_ms, Clip, History, Retention, Store};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const LIST_LIMIT: usize = 50;
const OPEN_COMMAND: &str = "open";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Keep a searchable history of everything copied, and copy it again.")
        .default_command(["list"])
        .command(
            Command::new("list")
                .about("List the newest clips, optionally only those matching a search.")
                .usage(format!("{PLUGIN_ID} list [search words]"))
                .output("One clip per line: id, kind, age and its first line. Secrets show masked.")
                .exit_behavior("Exits non-zero if the history cannot be read.")
                .run_plain_text(|context| list_text(&search_words(context)))
                .run_json(|context| list_json(&search_words(context))),
        )
        .command(
            Command::new("get")
                .about("Print one clip in full.")
                .usage(format!("{PLUGIN_ID} get <id>"))
                .output("The clip's text, or the path of its image.")
                .exit_behavior("Exits non-zero if no clip has that id.")
                .run_plain_text(|context| get_text(one_id(context)?))
                .run_json(|context| get_json(one_id(context)?)),
        )
        .command(
            Command::new("copy")
                .about("Put a clip back on the clipboard through the running daemon.")
                .usage(format!("{PLUGIN_ID} copy <id>"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if qol-tray or capture is not running.")
                .run_plain_text(|context| {
                    copy_via_tray(one_id(context)?)?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .command(
            Command::new("pin")
                .about("Pin a clip so it is kept until unpinned, or unpin a pinned clip.")
                .usage(format!("{PLUGIN_ID} pin <id>"))
                .output("Says whether the clip is now pinned.")
                .exit_behavior("Exits non-zero if no clip has that id or it is a secret.")
                .run_plain_text(|context| pin_text(one_id(context)?)),
        )
        .command(
            Command::new("forget")
                .about("Remove a clip from the history.")
                .usage(format!("{PLUGIN_ID} forget <id>"))
                .output("Confirms the removed id.")
                .exit_behavior("Exits non-zero if no clip has that id.")
                .run_plain_text(|context| {
                    let id = one_id(context)?;
                    open_history()?.forget(id)?;
                    Ok(PlainTextOutput::text(format!("Forgot {id}")))
                }),
        )
        .command(
            Command::new("clear")
                .about("Remove every unpinned clip; --all removes pinned clips too.")
                .usage(format!("{PLUGIN_ID} clear [--all]"))
                .output("How many clips were removed.")
                .exit_behavior("Exits non-zero if the history cannot be written.")
                .run_plain_text(|context| clear_text(context.args())),
        )
        .command(
            Command::new(OPEN_COMMAND)
                .about("Open the launcher straight into the clipboard flow.")
                .usage(format!("{PLUGIN_ID} {OPEN_COMMAND}"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the launcher cannot be started.")
                .run_plain_text(|_| {
                    crate::app::host::open_in_launcher()?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .command(
            Command::new("settings")
                .about("Open the clipboard settings.")
                .usage(format!("{PLUGIN_ID} settings"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the settings cannot be opened.")
                .run_plain_text(|_| {
                    crate::app::host::open_settings()?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .doctor_checks(doctor_checks())
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "capture",
            "Verify clipboard changes can be captured in this session.",
            || Ok(capture_result(crate::capture::probe())),
        ),
        DoctorCheck::new(
            "history_readable",
            "Verify the clipboard history can be read.",
            history_check,
        ),
    ]
}

fn capture_result(support: Support) -> DoctorCheckResult {
    match support {
        Support::Ready { backend } => {
            DoctorCheckResult::ok("capture", format!("Clipboard capture runs on {backend}."))
        }
        Support::Unavailable(CaptureError::Unsupported { platform }) => DoctorCheckResult::warn(
            "capture",
            format!(
                "Clipboard capture is not supported on {platform} yet; the history stays as it is."
            ),
        ),
        Support::Unavailable(error) => DoctorCheckResult::warn("capture", error.to_string())
            .with_fix("Run qol-tray inside an X11 session."),
    }
}

fn history_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve() {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::fail(
                "history_readable",
                format!("{error:#}"),
            ))
        }
    };
    Ok(match store.load() {
        Ok(clips) => DoctorCheckResult::ok(
            "history_readable",
            format!(
                "{} clips in {}.",
                clips.len(),
                store.history_path().display()
            ),
        ),
        Err(error) => {
            DoctorCheckResult::fail("history_readable", format!("{error:#}")).with_fix(format!(
                "Move {} aside; a new history starts empty.",
                store.history_path().display()
            ))
        }
    })
}

fn open_history() -> Result<History> {
    Ok(History::new(Store::resolve()?))
}

fn retention() -> Retention {
    crate::config::load().retention()
}

fn search_words(context: &CommandContext) -> String {
    context.args().join(" ")
}

fn one_id(context: &CommandContext) -> Result<&str> {
    match context.args() {
        [id] => Ok(id.as_str()),
        [] => bail!("missing clip id; run `{PLUGIN_ID} list` to see ids"),
        _ => bail!("expected one clip id"),
    }
}

fn listed(search: &str) -> Result<Vec<Clip>> {
    open_history()?.search(search, LIST_LIMIT, now_ms(), retention())
}

fn list_text(search: &str) -> Result<PlainTextOutput> {
    let now = now_ms();
    let lines: Vec<String> = listed(search)?
        .iter()
        .map(|clip| list_line(clip, now))
        .collect();
    if lines.is_empty() {
        return Ok(PlainTextOutput::text("No clips."));
    }
    Ok(PlainTextOutput::text(lines.join("\n")))
}

fn list_line(clip: &Clip, now: u64) -> String {
    let pinned = if clip.pinned { "  pinned" } else { "" };
    format!(
        "{}  {:<7}  {:>6}  {}{pinned}",
        clip.id,
        clip.kind.id(),
        rows::age(now.saturating_sub(clip.copied_at_ms)),
        rows::title(clip)
    )
}

fn list_json(search: &str) -> Result<Value> {
    Ok(Value::Array(listed(search)?.iter().map(summary).collect()))
}

fn summary(clip: &Clip) -> Value {
    json!({
        "id": clip.id,
        "kind": clip.kind,
        "title": rows::title(clip),
        "app": clip.app,
        "bytes": clip.bytes,
        "size": rows::size(clip),
        "copied_at_ms": clip.copied_at_ms,
        "pinned": clip.pinned,
        "masked": clip.is_secret(),
    })
}

fn found(id: &str) -> Result<(History, Clip)> {
    let history = open_history()?;
    let clip = history
        .get(id, now_ms(), retention())?
        .with_context(|| format!("no clip with id {id}"))?;
    Ok((history, clip))
}

fn get_text(id: &str) -> Result<PlainTextOutput> {
    let (history, clip) = found(id)?;
    if let Some(image) = &clip.image {
        let path = history.store().blob_path(&image.blob);
        return Ok(PlainTextOutput::text(path.display().to_string()));
    }
    Ok(PlainTextOutput::text(clip.text.unwrap_or_default()))
}

fn get_json(id: &str) -> Result<Value> {
    let (history, clip) = found(id)?;
    let mut value = serde_json::to_value(&clip).context("failed to encode the clip")?;
    if let (Some(image), Some(fields)) = (&clip.image, value.as_object_mut()) {
        let path = history.store().blob_path(&image.blob);
        fields.insert("path".to_string(), json!(path));
    }
    Ok(value)
}

fn pin_text(id: &str) -> Result<PlainTextOutput> {
    let pinned = open_history()?.toggle_pin(id, now_ms(), retention())?;
    let verb = if pinned { "Pinned" } else { "Unpinned" };
    Ok(PlainTextOutput::text(format!("{verb} {id}")))
}

fn clear_text(args: &[String]) -> Result<PlainTextOutput> {
    let include_pinned = match args {
        [] => false,
        [flag] if flag == "--all" => true,
        _ => bail!("clear takes only --all"),
    };
    let removed = open_history()?.clear(include_pinned)?;
    let noun = if removed == 1 { "clip" } else { "clips" };
    Ok(PlainTextOutput::text(format!("Cleared {removed} {noun}")))
}

fn copy_via_tray(id: &str) -> Result<()> {
    let route = qol_conventions::api_routes::plugin_action(PLUGIN_ID, "copy");
    let body = json!({ "id": id }).to_string();
    let (status, response) = qol_plugin_api::host_exec::post_to_daemon(&route, &body)
        .context("qol-tray is not running; copy needs the clipboard daemon")?;
    if !(200..300).contains(&status) {
        bail!("qol-tray answered {status}: {response}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qol_headless::{DoctorReport, DoctorStatus, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;

    fn run(args: &[&str]) -> qol_headless::Execution {
        app().execute(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        for command in [
            "list",
            "get",
            "copy",
            "pin",
            "forget",
            "clear",
            OPEN_COMMAND,
        ] {
            let first = run(&["help", command]);
            let last = run(&[command, "help"]);
            assert_eq!(first.exit_code, EXIT_SUCCESS, "command: {command}");
            assert_eq!(first.stdout, last.stdout, "command: {command}");
        }
    }

    #[test]
    fn doctor_json_lists_capture_and_history_checks() {
        let before = run(&["--json", "doctor"]);
        let after = run(&["doctor", "--json"]);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport = serde_json::from_str(&before.stdout).expect("doctor JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        let ids: Vec<&str> = report
            .checks
            .iter()
            .map(|check| check.id.as_str())
            .collect();
        assert_eq!(ids, vec!["capture", "history_readable"]);
    }

    #[test]
    fn capture_that_cannot_run_is_a_warning_never_ok() {
        let cases = [
            (
                Support::Ready {
                    backend: "x11-xfixes",
                },
                DoctorStatus::Ok,
            ),
            (
                Support::Unavailable(CaptureError::Unsupported { platform: "macOS" }),
                DoctorStatus::Warn,
            ),
            (
                Support::Unavailable(CaptureError::Unavailable("no X11 display".to_string())),
                DoctorStatus::Warn,
            ),
            (
                Support::Unavailable(CaptureError::Failed("lost".to_string())),
                DoctorStatus::Warn,
            ),
        ];
        for (support, status) in cases {
            let label = format!("{support:?}");
            assert_eq!(capture_result(support).status, status, "{label}");
        }
    }

    #[test]
    fn json_is_rejected_for_commands_without_a_json_contract() {
        for command in ["pin", "forget", "clear", "copy", OPEN_COMMAND] {
            let execution = run(&[command, "--json"]);
            assert_eq!(execution.exit_code, EXIT_USAGE, "command: {command}");
        }
    }

    #[test]
    fn id_commands_reject_missing_or_extra_ids() {
        for command in ["get", "copy", "pin", "forget"] {
            let missing = run(&[command]);
            assert_ne!(missing.exit_code, EXIT_SUCCESS, "command: {command}");
            assert!(
                missing.stderr.contains("missing clip id"),
                "{}",
                missing.stderr
            );
            let extra = run(&[command, "a", "b"]);
            assert_ne!(extra.exit_code, EXIT_SUCCESS, "command: {command}");
        }
        assert_ne!(run(&["clear", "--everything"]).exit_code, EXIT_SUCCESS);
    }

    #[test]
    fn list_lines_mask_secrets_and_mark_pins() {
        let mut clip = Clip {
            id: "c_0123456789abcdef".to_string(),
            kind: crate::history::ClipKind::Secret,
            text: Some("hunter2".to_string()),
            image: None,
            bytes: 7,
            app: None,
            copied_at_ms: 1_000,
            pinned: false,
        };
        let line = list_line(&clip, 61_000);
        assert!(!line.contains("hunter2"), "{line}");
        assert!(line.contains("1 min"), "{line}");
        assert_eq!(summary(&clip)["masked"], true);

        clip.kind = crate::history::ClipKind::Text;
        clip.pinned = true;
        assert!(list_line(&clip, 61_000).ends_with("hunter2  pinned"));
    }
}
