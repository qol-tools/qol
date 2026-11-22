use serde::Serialize;

use super::store::{RecordStore, Result};
use crate::knocks::KnockStore;
use crate::mailbox::MailboxStore;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct ForkMail {
    pub(super) key: String,
    pub(super) friendly: String,
    pub(super) cwd: String,
    pub(super) agent_id: String,
    pub(super) text: String,
}

pub(super) fn take(
    records: &RecordStore,
    knocks: &KnockStore,
    mail: &MailboxStore,
    terminal_keys: &[String],
) -> Result<Vec<ForkMail>> {
    let waiting = knocks.pending_knocks();
    Ok(records
        .open(terminal_keys)?
        .into_iter()
        .filter(|record| {
            waiting
                .iter()
                .any(|knock| knock.recipient_key == record.key)
        })
        .filter_map(|record| {
            let agent_id = record.parts.last()?.agent_id.clone();
            let text = knocks.take(mail, &record.key).ok().flatten()?;
            Some(ForkMail {
                key: record.key,
                friendly: record.friendly,
                cwd: record.cwd,
                agent_id,
                text,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_is_taken_once_for_each_open_record_that_has_an_agent() {
        let stores = crate::mailbox::tests::Stores::new();
        let root = tempfile::TempDir::new().unwrap();
        let records = RecordStore::with_dir(root.path().join("records"));
        for description in [
            "alpha: Alpha fork",
            "closed: Closed fork",
            "bare: No agent yet",
        ] {
            records
                .claim(
                    description,
                    crate::record::Placement::Under(None),
                    "/work",
                    1,
                )
                .unwrap();
        }
        records
            .add_part("alpha", "agent-old", "/old.jsonl", 2)
            .unwrap();
        records
            .add_part("alpha", "agent-new", "/new.jsonl", 3)
            .unwrap();
        records
            .add_part("closed", "agent-closed", "/closed.jsonl", 2)
            .unwrap();
        records.done("closed", "merged", 4).unwrap();
        records
            .claim(
                "terminal: Open in a terminal too",
                crate::record::Placement::Under(None),
                "/work",
                1,
            )
            .unwrap();
        records
            .add_part("terminal", "agent-terminal", "/terminal.jsonl", 2)
            .unwrap();
        for key in ["alpha", "closed", "bare", "terminal"] {
            stores.knocks.add_board(key, "t1", true).unwrap();
        }
        let terminal_keys = ["terminal".to_owned()];

        let taken = take(&records, &stores.knocks, &stores.mail, &terminal_keys).unwrap();
        assert_eq!(
            taken,
            [ForkMail {
                key: "alpha".to_owned(),
                friendly: "Alpha fork".to_owned(),
                cwd: "/work".to_owned(),
                agent_id: "agent-new".to_owned(),
                text: "[qol session] board mention in thread t1. Read it with session_board."
                    .to_owned(),
            }]
        );
        let left = stores
            .knocks
            .pending_knocks()
            .into_iter()
            .map(|knock| knock.recipient_key)
            .collect::<Vec<_>>();
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(!left.contains(&"alpha".to_owned()));
        assert!(take(&records, &stores.knocks, &stores.mail, &terminal_keys)
            .unwrap()
            .is_empty());
    }
}
