mod assistant;
mod audio;
mod doctor;
mod listen;
mod session;
mod stt;

use std::process::ExitCode;

use qol_headless::{Command, HeadlessApp, PlainTextOutput};

use crate::PLUGIN_ID;

const BINARY_NAME: &str = "qol-voice";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run provider-neutral speech recognition and conversational turn coordination.")
        .default_command(["session", "status"])
        .command(session::command())
        .command(listen::command())
        .command(audio::command())
        .command(stt::command())
        .command(assistant::command())
        .command(settings_command())
        .doctor_checks(doctor::checks())
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings page.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            crate::platform::open_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn reject_args(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    anyhow::bail!("unexpected arguments: {}", args.join(" "))
}

fn response_payload(action: &str, input: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    Ok(crate::app::send_request(action, input)?.unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::app;

    #[test]
    fn help_exposes_the_headless_mvp() {
        let execution = app().execute(["help".to_owned()]);

        assert_eq!(execution.exit_code, 0);
        assert!(execution.stdout.contains("session"));
        assert!(execution.stdout.contains("listen"));
        assert!(execution.stdout.contains("audio"));
        assert!(execution.stdout.contains("assistant"));
        assert!(execution.stdout.contains("doctor"));
    }
}
