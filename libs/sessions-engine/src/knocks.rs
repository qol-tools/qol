use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const KNOCK_HEADER: &str = "[qol session]";
pub(super) const MESSAGE_LEAD: &str = "Message from ";
const ANSWER_LEAD: &str = "(Answer with session_send";

pub(super) fn poses_as_sender(text: &str) -> bool {
    let leads = [MESSAGE_LEAD.to_lowercase(), ANSWER_LEAD.to_lowercase()];
    text.lines().any(|line| {
        let line = line
            .chars()
            .filter(|character| !is_invisible(*character))
            .collect::<String>()
            .trim_start()
            .to_lowercase();
        leads.iter().any(|lead| line.starts_with(lead.as_str()))
    })
}

pub(super) fn quoted_body(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| match index {
            0 => line.to_owned(),
            _ => format!("> {line}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn has_line_separator(text: &str) -> bool {
    text.contains(['\u{2028}', '\u{2029}', '\r'])
}

fn is_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingKnock {
    pub recipient_key: String,
    pub text: String,
    pub since_ms: u64,
}

pub fn pending() -> Vec<PendingKnock> {
    KnockStore::system()
        .map(|store| store.pending_knocks())
        .unwrap_or_default()
}

pub struct ClaimedKnock(KnockRecord);

impl ClaimedKnock {
    pub fn text(&self) -> String {
        self.0.text()
    }
}

pub fn claim(recipient_key: &str) -> io::Result<Option<ClaimedKnock>> {
    match KnockStore::system() {
        Some(store) => store.claim(recipient_key),
        None => Ok(None),
    }
}

pub fn restore(claimed: &ClaimedKnock) -> io::Result<()> {
    match KnockStore::system() {
        Some(store) => store.restore(claimed),
        None => Ok(()),
    }
}

pub fn delivered(claimed: &ClaimedKnock) -> io::Result<()> {
    let Some(store) = KnockStore::system() else {
        return Ok(());
    };
    let mail = super::mailbox::MailboxStore::system().map_err(io::Error::other)?;
    store.delivered(&mail, claimed)
}

pub fn lock_delivery(recipient_key: &str) -> io::Result<Option<File>> {
    KnockStore::system()
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no sessions data directory"))?
        .lock_delivery(recipient_key)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct KnockRecord {
    recipient_key: String,
    since_ms: u64,
    #[serde(default)]
    notes: Vec<KnockNote>,
    #[serde(default)]
    board: Vec<BoardKnock>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct KnockNote {
    pub(super) id: String,
    pub(super) from: String,
    pub(super) text: String,
    #[serde(default)]
    pub(super) ask: bool,
}

impl KnockNote {
    fn line(&self) -> String {
        let from = match self.from == super::mailbox::USER_SENDER {
            true => "the user",
            false => self.from.as_str(),
        };
        match self.ask {
            true => format!(
                "{MESSAGE_LEAD}{from}: {}\n{ANSWER_LEAD} kind reply, reply_to {}.)",
                quoted_body(&self.text),
                self.id
            ),
            false => format!("{MESSAGE_LEAD}{from}: {}", quoted_body(&self.text)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct BoardKnock {
    thread: String,
    mention: bool,
}

impl KnockRecord {
    fn fresh(key: &str) -> Self {
        Self {
            recipient_key: key.to_owned(),
            since_ms: super::mailbox::now_ms(),
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.board.is_empty()
    }

    fn text(&self) -> String {
        let mut parts = self.notes.iter().map(KnockNote::line).collect::<Vec<_>>();
        if !self.board.is_empty() {
            let threads = self
                .board
                .iter()
                .map(|knock| {
                    let verb = if knock.mention { "mention" } else { "reply" };
                    format!("board {verb} in thread {}", knock.thread)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let pronoun = if self.board.len() == 1 { "it" } else { "them" };
            parts.push(format!("{threads}. Read {pronoun} with session_board."));
        }
        format!("{KNOCK_HEADER} {}", parts.join("\n\n"))
    }
}

pub(super) struct KnockStore {
    dir: PathBuf,
}

impl KnockStore {
    pub(super) fn system() -> Option<Self> {
        let dir = qol_config::data_subdir("sessions")?.join("knocks");
        Some(Self { dir })
    }

    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, key: &str) -> io::Result<PathBuf> {
        let component = super::mailbox::key_component(key)
            .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error.to_string()))?;
        Ok(self.dir.join(format!("{component}.json")))
    }

    fn locked<T>(&self, key: &str, apply: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        let path = self.path(key)?.with_extension("lock");
        fs::create_dir_all(&self.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        lock.lock()?;
        apply()
    }

    pub(super) fn lock_delivery(&self, key: &str) -> io::Result<Option<File>> {
        let path = self.path(key)?.with_extension("delivery");
        fs::create_dir_all(&self.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    fn load(&self, key: &str) -> io::Result<KnockRecord> {
        match fs::read_to_string(self.path(key)?) {
            Ok(encoded) => {
                Ok(serde_json::from_str(&encoded).unwrap_or_else(|_| KnockRecord::fresh(key)))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(KnockRecord::fresh(key)),
            Err(error) => Err(error),
        }
    }

    fn save(&self, record: &KnockRecord) -> io::Result<()> {
        if record.is_empty() {
            return self.clear(&record.recipient_key);
        }
        let encoded = serde_json::to_string(record).map_err(io::Error::other)?;
        qol_fs::atomic_write(&self.path(&record.recipient_key)?, encoded.as_bytes())
    }

    pub(super) fn set_messages(&self, key: &str, notes: &[KnockNote]) -> io::Result<()> {
        self.locked(key, || {
            let mut record = self.load(key)?;
            if record.notes == notes {
                return Ok(());
            }
            record.notes = notes.to_vec();
            self.save(&record)
        })
    }

    pub(super) fn add_board(&self, key: &str, thread: &str, mention: bool) -> io::Result<()> {
        self.locked(key, || {
            let mut record = self.load(key)?;
            if !record.board.iter().any(|knock| knock.thread == thread) {
                record.board.push(BoardKnock {
                    thread: thread.to_owned(),
                    mention,
                });
            }
            self.save(&record)
        })
    }

    pub(super) fn clear_board(&self, key: &str, thread: &str) -> io::Result<()> {
        self.locked(key, || {
            if !self.path(key)?.exists() {
                return Ok(());
            }
            let mut record = self.load(key)?;
            record.board.retain(|knock| knock.thread != thread);
            self.save(&record)
        })
    }

    pub(super) fn claim(&self, key: &str) -> io::Result<Option<ClaimedKnock>> {
        self.locked(key, || {
            if !self.path(key)?.exists() {
                return Ok(None);
            }
            let record = self.load(key)?;
            self.clear(key)?;
            Ok((!record.is_empty()).then_some(ClaimedKnock(record)))
        })
    }

    pub(super) fn restore(&self, claimed: &ClaimedKnock) -> io::Result<()> {
        let claimed = &claimed.0;
        self.locked(&claimed.recipient_key, || {
            if !self.path(&claimed.recipient_key)?.exists() {
                return self.save(claimed);
            }
            let mut record = self.load(&claimed.recipient_key)?;
            record.since_ms = record.since_ms.min(claimed.since_ms);
            let newer = record
                .notes
                .into_iter()
                .filter(|note| !claimed.notes.iter().any(|kept| kept.id == note.id));
            record.notes = claimed.notes.iter().cloned().chain(newer).collect();
            for knock in &claimed.board {
                if !record.board.iter().any(|kept| kept.thread == knock.thread) {
                    record.board.push(knock.clone());
                }
            }
            self.save(&record)
        })
    }

    pub(super) fn delivered(
        &self,
        mail: &super::mailbox::MailboxStore,
        claimed: &ClaimedKnock,
    ) -> io::Result<()> {
        let claimed = &claimed.0;
        let key = claimed.recipient_key.as_str();
        self.locked(key, || {
            for note in &claimed.notes {
                mail.mark(key, &note.id, "read").map_err(io::Error::other)?;
            }
            if !self.path(key)?.exists() {
                return Ok(());
            }
            let mut record = self.load(key)?;
            record
                .notes
                .retain(|note| !claimed.notes.iter().any(|done| done.id == note.id));
            self.save(&record)
        })
    }

    pub(super) fn clear(&self, key: &str) -> io::Result<()> {
        match fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn pending_knocks(&self) -> Vec<PendingKnock> {
        let Ok(paths) = super::mailbox::json_files(&self.dir) else {
            return Vec::new();
        };
        let mut knocks = paths
            .iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|encoded| serde_json::from_str::<KnockRecord>(&encoded).ok())
            .filter(|record| !record.is_empty())
            .map(|record| PendingKnock {
                text: record.text(),
                recipient_key: record.recipient_key,
                since_ms: record.since_ms,
            })
            .collect::<Vec<_>>();
        knocks.sort_by_key(|knock| knock.since_ms);
        knocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, KnockStore) {
        let root = tempfile::TempDir::new().unwrap();
        let store = KnockStore::with_dir(root.path().join("knocks"));
        (root, store)
    }

    fn note(id: &str, from: &str, text: &str) -> KnockNote {
        KnockNote {
            id: id.to_owned(),
            from: from.to_owned(),
            text: text.to_owned(),
            ask: false,
        }
    }

    #[test]
    fn a_message_knock_is_written_overwritten_and_cleared() {
        let (_root, store) = store();
        assert!(store.pending_knocks().is_empty());
        store
            .set_messages("lane", &[note("m1", "switchboard", "rebase it")])
            .unwrap();
        let first = store.pending_knocks();
        assert_eq!(
            first,
            [PendingKnock {
                recipient_key: "lane".to_owned(),
                text: "[qol session] Message from switchboard: rebase it".to_owned(),
                since_ms: first[0].since_ms,
            }]
        );
        store
            .set_messages(
                "lane",
                &[
                    note("m1", "switchboard", "rebase it"),
                    note("m2", "sib", "tests are green\non my side"),
                ],
            )
            .unwrap();
        let updated = store.pending_knocks();
        assert_eq!(updated.len(), 1);
        assert_eq!(
            updated[0].text,
            "[qol session] Message from switchboard: rebase it\n\nMessage from sib: tests are green\n> on my side"
        );
        assert_eq!(
            updated[0].since_ms, first[0].since_ms,
            "the wait start is kept"
        );
        store.set_messages("lane", &[]).unwrap();
        assert!(store.pending_knocks().is_empty());
        assert!(!store.dir.join("lane.json").exists());
    }

    #[test]
    fn the_user_and_asks_read_as_plain_messages() {
        let (_root, store) = store();
        let ask = KnockNote {
            ask: true,
            ..note("m2", "sib", "which branch?")
        };
        store
            .set_messages("lane", &[note("m1", "user", "ship it"), ask])
            .unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] Message from the user: ship it\n\nMessage from sib: which branch?\n(Answer with session_send kind reply, reply_to m2.)"
        );
    }

    #[test]
    fn board_knocks_coalesce_with_messages_and_clear_per_thread() {
        let (_root, store) = store();
        store.add_board("lane", "t1", false).unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] board reply in thread t1. Read it with session_board."
        );
        store.add_board("lane", "t1", false).unwrap();
        store.add_board("lane", "t2", true).unwrap();
        store
            .set_messages("lane", &[note("m1", "parent-key", "hi")])
            .unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] Message from parent-key: hi\n\nboard reply in thread t1, board mention in thread t2. Read them with session_board."
        );
        store.clear_board("lane", "t1").unwrap();
        store.set_messages("lane", &[]).unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] board mention in thread t2. Read it with session_board."
        );
        store.clear_board("lane", "t2").unwrap();
        assert!(store.pending_knocks().is_empty());
        store.clear_board("nobody", "t2").unwrap();
    }

    #[test]
    fn a_delivered_knock_clears_one_recipient_and_keeps_the_rest() {
        let (_root, store) = store();
        store.set_messages("a", &[note("m1", "x", "hi")]).unwrap();
        store.set_messages("b", &[note("m2", "x", "hi")]).unwrap();
        store.clear("a").unwrap();
        store.clear("a").unwrap();
        let left = store.pending_knocks();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].recipient_key, "b");
        assert!(store.clear("..").is_err());
    }

    #[test]
    fn a_claimed_knock_leaves_room_for_news_and_a_failed_typing_merges_back() {
        let (_root, store) = store();
        assert!(store.claim("lane").unwrap().is_none());
        store
            .set_messages("lane", &[note("m1", "a", "first")])
            .unwrap();
        store.add_board("lane", "t1", false).unwrap();
        let claimed = store.claim("lane").unwrap().unwrap();
        assert_eq!(
            claimed.text(),
            "[qol session] Message from a: first\n\nboard reply in thread t1. Read it with session_board."
        );
        assert!(store.pending_knocks().is_empty());
        store.add_board("lane", "t2", true).unwrap();
        store
            .set_messages(
                "lane",
                &[note("m1", "a", "first"), note("m2", "b", "second")],
            )
            .unwrap();
        store.restore(&claimed).unwrap();
        let merged = store.pending_knocks();
        assert_eq!(
            merged[0].text,
            "[qol session] Message from a: first\n\nMessage from b: second\n\nboard mention in thread t2, board reply in thread t1. Read them with session_board."
        );
        assert_eq!(merged[0].since_ms, claimed.0.since_ms);
    }

    #[test]
    fn a_typed_knock_marks_its_messages_read_and_drops_them_from_the_next_knock() {
        let stores = super::super::mailbox::tests::Stores::new();
        stores.lineage.record("child", Some("architect")).unwrap();
        let messaging = stores.messaging();
        let keys = ["architect".to_owned(), "child".to_owned()];
        let send = |text: &'static str| super::super::mailbox::SendRequest {
            from: "child",
            to: "parent",
            kind: super::super::mailbox::MessageKind::Note,
            text,
            reply_to: None,
            as_user: false,
        };
        messaging.send(&keys, &send("one")).unwrap();
        let claimed = stores.knocks.claim("architect").unwrap().unwrap();
        messaging.send(&keys, &send("two")).unwrap();
        stores.knocks.delivered(&stores.mail, &claimed).unwrap();
        assert_eq!(stores.mail.counts("architect").unwrap().0, 1);
        assert_eq!(
            stores.knocks.pending_knocks()[0].text,
            "[qol session] Message from child: two"
        );
    }

    #[test]
    fn one_deliverer_at_a_time_holds_a_recipient() {
        let (_root, store) = store();
        let held = store.lock_delivery("lane").unwrap();
        assert!(held.is_some());
        assert!(store.lock_delivery("lane").unwrap().is_none());
        assert!(store.lock_delivery("other").unwrap().is_some());
        drop(held);
        assert!(store.lock_delivery("lane").unwrap().is_some());
    }

    #[test]
    fn concurrent_writers_never_drop_each_others_knocks() {
        let (_root, store) = store();
        let store = std::sync::Arc::new(store);
        let writers = (0..8)
            .map(|index| {
                let store = std::sync::Arc::clone(&store);
                std::thread::spawn(move || {
                    for round in 0..10 {
                        store
                            .add_board("lane", &format!("t{index}-{round}"), false)
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        let encoded = fs::read_to_string(store.path("lane").unwrap()).unwrap();
        let record = serde_json::from_str::<KnockRecord>(&encoded).unwrap();
        assert_eq!(record.board.len(), 80);
    }
}
