use qol_headless::{Command, PlainTextOutput};

use super::reject_args;

pub(super) fn command() -> Command {
    Command::new("stt")
        .about("Inspect registered speech-recognition providers.")
        .subcommand(providers_command())
}

fn providers_command() -> Command {
    Command::new("providers")
        .about("List provider-neutral STT backends available on this platform.")
        .usage("qol-voice stt providers")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let providers = crate::transcribe::transcriber_descriptors().collect::<Vec<_>>();
            if providers.is_empty() {
                return Ok(PlainTextOutput::text("no STT providers registered"));
            }
            let lines = providers
                .iter()
                .map(|provider| {
                    format!(
                        "{} ({}) [{:?}]",
                        provider.name, provider.id, provider.capabilities.location
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(PlainTextOutput::text(lines))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            let providers = crate::transcribe::transcriber_descriptors().collect::<Vec<_>>();
            Ok(serde_json::to_value(providers)?)
        })
}
