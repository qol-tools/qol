use std::ffi::OsString;

use serde_json::json;

use super::store::{Placement, Record, RecordStore};
use crate::verb::{Failure, Flags, Outcome};

pub(crate) const HELP: &str = include_str!("help.txt");

pub(crate) fn execute(args: &[OsString], store: &RecordStore, now_ms: u64) -> Outcome {
    crate::verb::execute(args, "record", HELP, |verb, flags| {
        run_verb(verb, flags, store, now_ms)
    })
}

fn run_verb(
    verb: &str,
    flags: &Flags,
    store: &RecordStore,
    now_ms: u64,
) -> Result<Outcome, Failure> {
    match verb {
        "claim" | "reclaim" => {
            let placement = if verb == "claim" {
                flags.only(&["description", "cwd", "parent"])?;
                Placement::Under(flags.get("parent"))
            } else {
                flags.only(&["description", "cwd"])?;
                Placement::Existing
            };
            let cwd = flags.absolute("cwd")?;
            let description = flags
                .get("description")
                .ok_or_else(|| Failure::Usage("--description is required".to_owned()))?;
            let record = store.claim(description, placement, cwd, now_ms)?;
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
        "mail" => {
            flags.only(&[])?;
            let mail = match (
                crate::knocks::KnockStore::system(),
                crate::mailbox::MailboxStore::system(),
                crate::terminal_keys(),
            ) {
                (Some(knocks), Ok(mail), Some(terminal_keys)) => {
                    super::mail::take(store, &knocks, &mail, &terminal_keys)?
                }
                _ => Vec::new(),
            };
            Ok(Outcome::ok(json!({ "mail": mail })))
        }
        other => Err(Failure::Usage(format!("unknown record verb `{other}`"))),
    }
}

fn record_outcome(record: Record) -> Outcome {
    Outcome::ok(json!(record))
}
