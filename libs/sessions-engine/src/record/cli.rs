use std::ffi::OsString;
use std::path::Path;

use serde_json::{json, Value};

use super::store::{Error, Record, RecordStore};

pub(crate) const EXIT_USAGE: u8 = 2;
pub(crate) const EXIT_UNKNOWN: u8 = 3;
const EXIT_FAILED: u8 = 1;

pub(crate) const HELP: &str = "qol-cli-sessions sessions record <verb>

Keep one record per session key, so a session's identity lives in one place and its chat is saved under that key to resume or search later. Records live in the CLI Sessions data folder, in records/<key>.json, and each part's chat is archived in records/<key>/part-<n>.jsonl.

Usage:
  qol-cli-sessions sessions record claim --description TEXT --cwd DIR
  qol-cli-sessions sessions record part --key KEY --agent-id ID --transcript FILE
  qol-cli-sessions sessions record archive --key KEY
  qol-cli-sessions sessions record done --key KEY --reason TEXT
  qol-cli-sessions sessions record stopped --key KEY
  qol-cli-sessions sessions record resume --key KEY
  qol-cli-sessions sessions record show (--key KEY | --agent-id ID)
  qol-cli-sessions sessions record list

Details:
  claim creates a new record and never reuses one. A description of the form `<key>: <friendly>`, with a key of lowercase letters, digits and dashes that starts with a letter or digit and is at most 64 characters, keeps that key and friendly description. Any other description becomes the friendly description, and its key is the description in lowercase ASCII, each run of other characters turned into one dash, cut to at most 40 characters at a dash (`session` when nothing is left). A key already taken gets -2, -3 and so on. --cwd is the absolute project folder. The output carries key, friendly and description, the `<key>: <friendly>` text to give a fork (just the key when the friendly description is empty).
  part adds the next numbered part of the record for an agent id, with --transcript the absolute path of that agent's chat file. An agent id the record already holds changes nothing and returns its part number.
  archive copies each part's transcript that exists to the record's archive folder and lists the copies. A part whose transcript is gone keeps the copy made earlier.
  done sets the record's done time and reason; a later call replaces them, so the latest done wins.
  stopped sets the time the session was stopped.
  resume clears done and stopped_ms, for a session that was done or stopped and works again; a record with neither set stays as it is.
  show prints the record for a key, or the record whose parts include the agent id.
  list prints every record, newest created first.

Output:
  Every verb prints one JSON object on stdout: claim {key, friendly, description}, part {key, part}, archive {key, archived}, list {records}, and the record itself for done, stopped, resume and show. A record is {key, friendly, cwd, created_ms, parts: [{n, agent_id, transcript, started_ms}], done: {at_ms, reason}, stopped_ms}, where done and stopped_ms are absent until set. An error prints {\"error\": text}. Times are milliseconds since the Unix epoch. The session_records tool searches these records and their archived chats.

Exit:
  0 on success, 1 when the records folder cannot be read or written, 2 for a usage error (a missing, repeated or unknown flag, a relative path, or an unknown verb), 3 when the key or agent id has no record.";

pub(crate) struct Outcome {
    pub(crate) stdout: String,
    pub(crate) code: u8,
}

impl Outcome {
    fn ok(value: Value) -> Self {
        Self {
            stdout: value.to_string(),
            code: 0,
        }
    }

    fn failed(code: u8, message: impl std::fmt::Display) -> Self {
        Self {
            stdout: json!({ "error": message.to_string() }).to_string(),
            code,
        }
    }

    fn usage(message: impl std::fmt::Display) -> Self {
        Self::failed(EXIT_USAGE, format!("{message}\n\n{HELP}"))
    }
}

pub(crate) fn execute(args: &[OsString], store: &RecordStore, now_ms: u64) -> Outcome {
    let Some((verb, rest)) = args.split_first() else {
        return Outcome::usage("a record verb is required");
    };
    let Some(verb) = verb.to_str() else {
        return Outcome::usage("the record verb must be valid UTF-8");
    };
    if matches!(verb, "help" | "-h" | "--help") {
        return Outcome {
            stdout: HELP.to_owned(),
            code: 0,
        };
    }
    let flags = match Flags::parse(rest) {
        Ok(flags) => flags,
        Err(message) => return Outcome::usage(message),
    };
    match run_verb(verb, &flags, store, now_ms) {
        Ok(outcome) => outcome,
        Err(Failure::Usage(message)) => Outcome::usage(message),
        Err(Failure::Store(Error::Unknown(message))) => Outcome::failed(EXIT_UNKNOWN, message),
        Err(Failure::Store(error)) => Outcome::failed(EXIT_FAILED, error),
    }
}

enum Failure {
    Usage(String),
    Store(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Store(error)
    }
}

fn run_verb(
    verb: &str,
    flags: &Flags,
    store: &RecordStore,
    now_ms: u64,
) -> Result<Outcome, Failure> {
    match verb {
        "claim" => {
            flags.only(&["description", "cwd"])?;
            let cwd = flags.absolute("cwd")?;
            let description = flags
                .get("description")
                .ok_or_else(|| Failure::Usage("--description is required".to_owned()))?;
            let record = store.claim(description, cwd, now_ms)?;
            Ok(Outcome::ok(json!({
                "key": record.key,
                "friendly": record.friendly,
                "description": record.description(),
            })))
        }
        "part" => {
            flags.only(&["key", "agent-id", "transcript"])?;
            let transcript = flags.absolute("transcript")?;
            let key = flags.required("key")?;
            let part = store.add_part(key, flags.required("agent-id")?, transcript, now_ms)?;
            Ok(Outcome::ok(json!({ "key": key, "part": part })))
        }
        "archive" => {
            flags.only(&["key"])?;
            let key = flags.required("key")?;
            let archived = store
                .archive(key)?
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            Ok(Outcome::ok(json!({ "key": key, "archived": archived })))
        }
        "done" => {
            flags.only(&["key", "reason"])?;
            let reason = flags.required("reason")?;
            Ok(record_outcome(store.done(
                flags.required("key")?,
                reason,
                now_ms,
            )?))
        }
        "stopped" => {
            flags.only(&["key"])?;
            Ok(record_outcome(
                store.stopped(flags.required("key")?, now_ms)?,
            ))
        }
        "resume" => {
            flags.only(&["key"])?;
            Ok(record_outcome(store.resume(flags.required("key")?)?))
        }
        "show" => {
            flags.only(&["key", "agent-id"])?;
            let record = match (flags.get("key"), flags.get("agent-id")) {
                (Some(key), None) => store.show(key)?,
                (None, Some(agent_id)) => store.find_by_agent(agent_id)?,
                _ => {
                    return Err(Failure::Usage(
                        "show takes exactly one of --key and --agent-id".to_owned(),
                    ))
                }
            };
            Ok(record_outcome(record))
        }
        "list" => {
            flags.only(&[])?;
            Ok(Outcome::ok(json!({ "records": store.list()? })))
        }
        other => Err(Failure::Usage(format!("unknown record verb `{other}`"))),
    }
}

fn record_outcome(record: Record) -> Outcome {
    Outcome::ok(json!(record))
}

struct Flags {
    values: Vec<(String, String)>,
}

impl Flags {
    fn parse(args: &[OsString]) -> Result<Self, String> {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let name = flag
                .to_str()
                .and_then(|flag| flag.strip_prefix("--"))
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("unexpected argument `{}`", flag.to_string_lossy()))?;
            let value = args
                .next()
                .ok_or_else(|| format!("--{name} needs a value"))?
                .to_str()
                .ok_or_else(|| format!("--{name} must be valid UTF-8"))?;
            if values.iter().any(|(seen, _)| seen == name) {
                return Err(format!("--{name} was given twice"));
            }
            values.push((name.to_owned(), value.to_owned()));
        }
        Ok(Self { values })
    }

    fn only(&self, allowed: &[&str]) -> Result<(), Failure> {
        match self
            .values
            .iter()
            .find(|(name, _)| !allowed.contains(&name.as_str()))
        {
            Some((name, _)) => Err(Failure::Usage(format!("unknown flag --{name}"))),
            None => Ok(()),
        }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(flag, _)| flag == name)
            .map(|(_, value)| value.as_str())
    }

    fn required(&self, name: &str) -> Result<&str, Failure> {
        self.get(name)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| Failure::Usage(format!("--{name} is required")))
    }

    fn absolute(&self, name: &str) -> Result<&str, Failure> {
        let value = self.required(name)?;
        if Path::new(value).is_absolute() {
            Ok(value)
        } else {
            Err(Failure::Usage(format!("--{name} must be an absolute path")))
        }
    }
}
