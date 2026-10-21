use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const KNOCK_HEADER: &str = "[qol session]";

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

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct KnockRecord {
    recipient_key: String,
    since_ms: u64,
    #[serde(default)]
    messages: usize,
    #[serde(default)]
    senders: Vec<String>,
    #[serde(default)]
    from_user: usize,
    #[serde(default)]
    board: Vec<BoardKnock>,
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
        self.messages == 0 && self.from_user == 0 && self.board.is_empty()
    }

    fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.from_user > 0 {
            let pronoun = if self.from_user == 1 { "it" } else { "them" };
            parts.push(format!(
                "The user wrote to you through the switchboard. Read {pronoun} with session_inbox."
            ));
        }
        if self.messages > 0 {
            let (noun, pronoun) = match self.messages {
                1 => ("message", "it"),
                _ => ("messages", "them"),
            };
            parts.push(format!(
                "{} new {noun} from {}. Read {pronoun} with session_inbox.",
                self.messages,
                self.senders.join(", ")
            ));
        }
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
        format!("{KNOCK_HEADER} {}", parts.join(" "))
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

    #[cfg(test)]
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

    pub(super) fn set_messages(
        &self,
        key: &str,
        unread: usize,
        senders: &[String],
        from_user: usize,
    ) -> io::Result<()> {
        self.locked(key, || {
            let mut record = self.load(key)?;
            if record.messages == unread
                && record.senders == senders
                && record.from_user == from_user
            {
                return Ok(());
            }
            record.messages = unread;
            record.senders = senders.to_vec();
            record.from_user = from_user;
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
            if record.messages == 0 {
                record.messages = claimed.messages;
                record.senders.clone_from(&claimed.senders);
            }
            for knock in &claimed.board {
                if !record.board.iter().any(|kept| kept.thread == knock.thread) {
                    record.board.push(knock.clone());
                }
            }
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

    #[test]
    fn a_message_knock_is_written_overwritten_and_cleared() {
        let (_root, store) = store();
        assert!(store.pending_knocks().is_empty());
        store
            .set_messages("lane", 1, &["switchboard".to_owned()], 0)
            .unwrap();
        let first = store.pending_knocks();
        assert_eq!(
            first,
            [PendingKnock {
                recipient_key: "lane".to_owned(),
                text: "[qol session] 1 new message from switchboard. Read it with session_inbox."
                    .to_owned(),
                since_ms: first[0].since_ms,
            }]
        );
        store
            .set_messages("lane", 3, &["switchboard".to_owned(), "sib".to_owned()], 0)
            .unwrap();
        let updated = store.pending_knocks();
        assert_eq!(updated.len(), 1);
        assert_eq!(
            updated[0].text,
            "[qol session] 3 new messages from switchboard, sib. Read them with session_inbox."
        );
        assert_eq!(
            updated[0].since_ms, first[0].since_ms,
            "the wait start is kept"
        );
        store.set_messages("lane", 0, &[], 0).unwrap();
        assert!(store.pending_knocks().is_empty());
        assert!(!store.dir.join("lane.json").exists());
    }

    #[test]
    fn the_users_words_lead_the_knock_ahead_of_session_messages() {
        let (_root, store) = store();
        store.set_messages("lane", 0, &[], 1).unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] The user wrote to you through the switchboard. Read it with session_inbox."
        );
        store
            .set_messages("lane", 1, &["sib".to_owned()], 2)
            .unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] The user wrote to you through the switchboard. Read them with session_inbox. 1 new message from sib. Read it with session_inbox."
        );
        store.set_messages("lane", 0, &[], 0).unwrap();
        assert!(store.pending_knocks().is_empty());
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
            .set_messages("lane", 1, &["parent-key".to_owned()], 0)
            .unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] 1 new message from parent-key. Read it with session_inbox. board reply in thread t1, board mention in thread t2. Read them with session_board."
        );
        store.clear_board("lane", "t1").unwrap();
        store.set_messages("lane", 0, &[], 0).unwrap();
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
        store.set_messages("a", 1, &["x".to_owned()], 0).unwrap();
        store.set_messages("b", 1, &["x".to_owned()], 0).unwrap();
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
        store.set_messages("lane", 1, &["a".to_owned()], 0).unwrap();
        store.add_board("lane", "t1", false).unwrap();
        let claimed = store.claim("lane").unwrap().unwrap();
        assert_eq!(
            claimed.text(),
            "[qol session] 1 new message from a. Read it with session_inbox. board reply in thread t1. Read it with session_board."
        );
        assert!(store.pending_knocks().is_empty());
        store.add_board("lane", "t2", true).unwrap();
        assert_eq!(
            store.pending_knocks()[0].text,
            "[qol session] board mention in thread t2. Read it with session_board."
        );
        store.restore(&claimed).unwrap();
        let merged = store.pending_knocks();
        assert_eq!(
            merged[0].text,
            "[qol session] 1 new message from a. Read it with session_inbox. board mention in thread t2, board reply in thread t1. Read them with session_board."
        );
        assert_eq!(merged[0].since_ms, claimed.0.since_ms);
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
