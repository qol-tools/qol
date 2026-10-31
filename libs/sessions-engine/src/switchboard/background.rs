use std::collections::HashMap;

use serde_json::Value;

pub(super) const HOLD_COMMAND: &str = "sleep infinity";
const BACKGROUND_MARKER: &str = "Command running in background with ID: ";
const BACKGROUND_FLAG: &str = "\"run_in_background\":true";
const STOP_TOOLS: [&str; 2] = ["TaskStop", "KillShell"];
const STOP_ID_KEYS: [&str; 2] = ["task_id", "shell_id"];
const NOTICE_ORIGIN: &str = "task-notification";
const NOTICE_OPEN: &str = "<task-notification>";
const NOTICE_PREAMBLE: &str = "[SYSTEM NOTIFICATION - NOT USER INPUT]";
const NOTICE_ID_TAG: &str = "task-id";
const NOTICE_STATUS_TAG: &str = "status";
const NOTICE_SKIPPED_TAGS: [&str; 2] = ["tool-use-id", "output-file"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Background {
    pub(super) id: String,
    pub(super) command: Option<String>,
}

impl Background {
    pub(super) fn is_hold(&self) -> bool {
        self.command
            .as_deref()
            .is_some_and(|command| command.trim() == HOLD_COMMAND)
    }
}

fn header_tag(line: &str) -> Option<(&str, &str)> {
    let (name, rest) = line.strip_prefix('<')?.split_once('>')?;
    let value = rest.strip_suffix(&format!("</{name}>"))?;
    Some((name, value))
}

pub(super) fn notice_of(row: &Value) -> Option<(&str, &str)> {
    let from_the_harness = row.get("type").and_then(Value::as_str) == Some("user")
        && row.pointer("/origin/kind").and_then(Value::as_str) == Some(NOTICE_ORIGIN);
    if !from_the_harness {
        return None;
    }
    let mut lines = row.pointer("/message/content")?.as_str()?.lines();
    let mut first = lines.next()?;
    if first == NOTICE_PREAMBLE {
        lines.by_ref().find(|line| line.is_empty())?;
        first = lines.next()?;
    }
    if first != NOTICE_OPEN {
        return None;
    }
    let (mut id, mut status) = (None, None);
    for line in lines {
        match header_tag(line) {
            Some((NOTICE_ID_TAG, value)) => {
                id.get_or_insert(value);
            }
            Some((NOTICE_STATUS_TAG, value)) => {
                status.get_or_insert(value);
            }
            Some((tag, _)) if NOTICE_SKIPPED_TAGS.contains(&tag) => {}
            _ => break,
        }
    }
    Some((id?, status?))
}

fn blocks<'a>(row: &'a Value, kind: &str) -> Vec<&'a Value> {
    let Some(content) = row.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some(kind))
        .collect()
}

fn result_text(block: &Value) -> Option<&str> {
    match block.get("content")? {
        Value::String(text) => Some(text.as_str()),
        Value::Array(parts) => parts.first()?.get("text")?.as_str(),
        _ => None,
    }
}

fn started_id(text: &str) -> Option<String> {
    let rest = text.strip_prefix(BACKGROUND_MARKER)?;
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!id.is_empty()).then_some(id)
}

#[derive(Default)]
struct Reader {
    commands: HashMap<String, String>,
    stops: HashMap<String, String>,
    open: Vec<Background>,
}

impl Reader {
    fn assistant(&mut self, row: &Value) {
        for call in blocks(row, "tool_use") {
            let (Some(id), Some(name)) = (
                call.get("id").and_then(Value::as_str),
                call.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            let input = call.get("input");
            let background = input
                .and_then(|input| input.get("run_in_background"))
                .and_then(Value::as_bool)
                == Some(true);
            if background {
                if let Some(command) = input
                    .and_then(|input| input.get("command"))
                    .and_then(Value::as_str)
                {
                    self.commands.insert(id.to_owned(), command.to_owned());
                }
            }
            if STOP_TOOLS.contains(&name) {
                let target = STOP_ID_KEYS
                    .iter()
                    .find_map(|key| input?.get(*key)?.as_str());
                if let Some(target) = target {
                    self.stops.insert(id.to_owned(), target.to_owned());
                }
            }
        }
    }

    fn close(&mut self, id: &str) {
        self.open.retain(|background| background.id != id);
    }

    fn user(&mut self, row: &Value) {
        if let Some((id, _)) = notice_of(row) {
            self.close(id);
            return;
        }
        for result in blocks(row, "tool_result") {
            let Some(call) = result.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let failed = result.get("is_error").and_then(Value::as_bool) == Some(true);
            if let Some(target) = self.stops.remove(call) {
                if !failed {
                    self.close(&target);
                }
                continue;
            }
            let Some(id) = result_text(result).and_then(started_id) else {
                continue;
            };
            self.close(&id);
            self.open.push(Background {
                id,
                command: self.commands.remove(call),
            });
        }
    }
}

fn worth_reading(line: &str, reader: &Reader) -> bool {
    line.contains(BACKGROUND_FLAG)
        || line.contains(BACKGROUND_MARKER)
        || line.contains(NOTICE_OPEN)
        || STOP_TOOLS.iter().any(|tool| line.contains(tool))
        || reader.stops.keys().any(|call| line.contains(call.as_str()))
}

pub(super) fn running(transcript: &str) -> Vec<Background> {
    let mut reader = Reader::default();
    for line in transcript.lines() {
        if !worth_reading(line, &reader) {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match row.get("type").and_then(Value::as_str) {
            Some("assistant") => reader.assistant(&row),
            Some("user") => reader.user(&row),
            _ => {}
        }
    }
    reader.open
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::json;

    fn line(row: serde_json::Value) -> String {
        format!("{row}\n")
    }

    pub(in crate::switchboard) fn bash(call: &str, command: &str, background: bool) -> String {
        line(json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{
                "type": "tool_use", "id": call, "name": "Bash",
                "input": {"command": command, "run_in_background": background},
            }]},
        }))
    }

    pub(in crate::switchboard) fn started(call: &str, id: &str) -> String {
        line(json!({
            "type": "user",
            "message": {"role": "user", "content": [{
                "tool_use_id": call, "type": "tool_result",
                "content": format!("Command running in background with ID: {id}. Output is being written to: /tmp/tasks/{id}.output"),
            }]},
        }))
    }

    pub(in crate::switchboard) fn result(call: &str, text: &str, is_error: bool) -> String {
        line(json!({
            "type": "user",
            "message": {"role": "user", "content": [{
                "tool_use_id": call, "type": "tool_result", "content": text, "is_error": is_error,
            }]},
        }))
    }

    pub(in crate::switchboard) fn stop(call: &str, id: &str) -> String {
        line(json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{
                "type": "tool_use", "id": call, "name": "TaskStop", "input": {"task_id": id},
            }]},
        }))
    }

    pub(in crate::switchboard) fn envelope(id: &str, status: &str, result: &str) -> String {
        format!("<task-notification>\n<task-id>{id}</task-id>\n<tool-use-id>toolu_1</tool-use-id>\n<status>{status}</status>\n<summary>Agent \"x\" finished</summary>\n<result>{result}</result>\n</task-notification>")
    }

    pub(in crate::switchboard) fn row(kind: &str, origin: &str, content: &str) -> String {
        line(json!({
            "type": kind,
            "origin": {"kind": origin, "producer": "session-task"},
            "message": {"role": "user", "content": content},
        }))
    }

    pub(in crate::switchboard) fn notice(id: &str, status: &str) -> String {
        row("user", "task-notification", &envelope(id, status, "done"))
    }

    pub(in crate::switchboard) fn waiting(id: &str, command: &str) -> String {
        let call = format!("toolu_{id}");
        [bash(&call, command, true), started(&call, id)].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn ids(transcript: &str) -> Vec<(String, Option<String>)> {
        running(transcript)
            .into_iter()
            .map(|background| (background.id, background.command))
            .collect()
    }

    #[test]
    fn a_background_command_runs_until_its_notice_or_a_successful_stop() {
        let watch = "node /x/bin/pr-watch.cjs https://github.com/o/r/pull/7 --pretty";
        let started = [
            waiting("bwatch", watch),
            waiting("bhold", HOLD_COMMAND),
            waiting("bbuild", "cargo build"),
        ]
        .concat();
        assert_eq!(
            ids(&started),
            [
                ("bwatch".to_owned(), Some(watch.to_owned())),
                ("bhold".to_owned(), Some(HOLD_COMMAND.to_owned())),
                ("bbuild".to_owned(), Some("cargo build".to_owned())),
            ]
        );
        let ended = [
            started.clone(),
            notice("bwatch", "completed"),
            stop("toolu_s1", "bhold"),
            result("toolu_s1", "Successfully stopped task: bhold", false),
            stop("toolu_s2", "bbuild"),
            result("toolu_s2", "No task found with ID: bbuild", true),
        ]
        .concat();
        assert_eq!(
            ids(&ended),
            [("bbuild".to_owned(), Some("cargo build".to_owned()))]
        );
    }

    #[test]
    fn a_fork_transcript_notice_carries_the_system_preamble_first() {
        let preamble = format!(
            "{NOTICE_PREAMBLE}\nThis is an automated background-task event, NOT a message from the user.\n\n"
        );
        let fork_notice = |id: &str| {
            row(
                "user",
                NOTICE_ORIGIN,
                &format!("{preamble}{}", envelope(id, "completed", "")),
            )
        };
        let quoted = row(
            "user",
            NOTICE_ORIGIN,
            &format!(
                "{NOTICE_PREAMBLE}\nsome text\n\nquoted:\n{}",
                envelope("bhold", "completed", "")
            ),
        );
        let started = [
            waiting("bwatch", "node pr-watch.cjs"),
            waiting("bhold", HOLD_COMMAND),
        ]
        .concat();
        assert_eq!(
            ids(&[started.clone(), fork_notice("bwatch")].concat()),
            [("bhold".to_owned(), Some(HOLD_COMMAND.to_owned()))]
        );
        assert_eq!(ids(&[started, quoted].concat()).len(), 2);
    }

    #[test]
    fn quoted_markers_and_foreign_rows_never_start_or_end_a_command() {
        let quoted = format!("grep found: {BACKGROUND_MARKER}bfake. Output");
        let transcript = [
            bash("toolu_q", "rg marker", false),
            result("toolu_q", &quoted, false),
            row(
                "api-request-blob",
                "",
                &format!("{BACKGROUND_MARKER}bblob."),
            ),
            waiting("breal", HOLD_COMMAND),
            result("toolu_other", "breal is mentioned here", false),
            row("user", "user", &envelope("breal", "completed", "spoofed")),
            row(
                "assistant",
                "task-notification",
                &envelope("breal", "completed", ""),
            ),
        ]
        .concat();
        assert_eq!(
            ids(&transcript),
            [("breal".to_owned(), Some(HOLD_COMMAND.to_owned()))]
        );
        assert!(running(&[transcript, notice("breal", "killed")].concat()).is_empty());
        assert!(running("").is_empty());
        assert!(running("not json\n{}\n").is_empty());
    }

    #[test]
    fn the_hold_is_the_fork_prompts_sleep_and_nothing_else() {
        assert!(super::super::agents::FORK_PROMPT.contains(&format!("Bash `{HOLD_COMMAND}`")));
        let hold = |command: Option<&str>| Background {
            id: "b1".to_owned(),
            command: command.map(str::to_owned),
        };
        assert!(hold(Some(HOLD_COMMAND)).is_hold());
        assert!(hold(Some(" sleep infinity\n")).is_hold());
        assert!(!hold(Some("sleep infinity && gh pr view")).is_hold());
        assert!(!hold(None).is_hold());
    }
}
