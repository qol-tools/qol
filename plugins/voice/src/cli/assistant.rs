use anyhow::{anyhow, Result};
use qol_headless::{Command, PlainTextOutput};

use super::response_payload;

pub(super) fn command() -> Command {
    Command::new("assistant")
        .about("Send agent-side conversational control into the live session.")
        .subcommand(request_command())
}

fn request_command() -> Command {
    Command::new("request")
        .about("Finalize active user speech and request an assistant turn.")
        .usage("qol-voice assistant request --response-id ID --utterance-id ID")
        .detail("This is a semantic agent interruption, not an audio-level shortcut.")
        .run_plain_text(|context| {
            let input = parse(context.args())?;
            let effects = response_payload("request_assistant_turn", input)?;
            let count = effects
                .get("effects")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            Ok(PlainTextOutput::text(format!(
                "assistant turn requested; {count} effect(s)"
            )))
        })
        .run_json(|context| response_payload("request_assistant_turn", parse(context.args())?))
}

fn parse(args: &[String]) -> Result<serde_json::Value> {
    let mut response_id = None;
    let mut utterance_id = None;
    let mut index = 0;
    while index < args.len() {
        let target = match args[index].as_str() {
            "--response-id" => &mut response_id,
            "--utterance-id" => &mut utterance_id,
            option => return Err(anyhow!("unknown assistant request option: {option}")),
        };
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| anyhow!("missing value for {}", args[index - 1]))?;
        *target = Some(
            value
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid numeric identifier: {value}"))?,
        );
        index += 1;
    }
    Ok(serde_json::json!({
        "response_id": response_id.ok_or_else(|| anyhow!("missing --response-id"))?,
        "utterance_id": utterance_id.ok_or_else(|| anyhow!("missing --utterance-id"))?,
    }))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn request_requires_both_typed_ids() {
        let input = parse(&[
            "--response-id".to_owned(),
            "10".to_owned(),
            "--utterance-id".to_owned(),
            "20".to_owned(),
        ])
        .unwrap();

        assert_eq!(input["response_id"], 10);
        assert_eq!(input["utterance_id"], 20);
        assert!(parse(&["--response-id".to_owned(), "ten".to_owned()]).is_err());
    }
}
