use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::knocks::KnockStore;
use super::mailbox::{check_rate, json_files, key_component, now_ms, unique_name, MailboxStore};
use super::switchboard::SWITCHBOARD_KEY;

pub(super) const TITLE_MAX_CHARS: usize = 120;
pub(super) const MENTIONS_MAX: usize = 8;
const RESOLVED_FILE: &str = "resolved.mark";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ThreadKind {
    Question,
    HeadsUp,
    Found,
    ForUser,
}

impl ThreadKind {
    pub(super) fn from_token(token: &str) -> Option<Self> {
        match token {
            "question" => Some(Self::Question),
            "heads-up" => Some(Self::HeadsUp),
            "found" => Some(Self::Found),
            "for-user" => Some(Self::ForUser),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct Post {
    pub(super) author: String,
    pub(super) text: String,
    pub(super) at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) kind: Option<ThreadKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) mentions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct Resolution {
    by: String,
    at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ThreadSummary {
    pub(super) id: String,
    pub(super) kind: ThreadKind,
    pub(super) title: String,
    pub(super) poster: String,
    pub(super) replies: usize,
    pub(super) updated: u64,
    pub(super) resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Thread {
    pub(super) id: String,
    pub(super) kind: ThreadKind,
    pub(super) title: String,
    pub(super) poster: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) resolved_by: Option<String>,
    pub(super) posts: Vec<Post>,
}

pub(super) struct NewThread<'a> {
    pub(super) kind: ThreadKind,
    pub(super) title: &'a str,
    pub(super) text: &'a str,
    pub(super) mentions: &'a [String],
}

pub(super) struct BoardStore {
    dir: PathBuf,
}

impl BoardStore {
    pub(super) fn system() -> Result<Self> {
        let dir = qol_config::data_subdir("sessions")
            .ok_or_else(|| anyhow!("sessions data directory is unavailable"))?
            .join("board");
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn thread_dir(&self, thread: &str) -> Result<PathBuf> {
        key_component(thread).map_err(|_| anyhow!("`{thread}` is not a board thread id"))?;
        Ok(self.dir.join(thread))
    }

    fn append(&self, thread: &str, post: &Post) -> Result<()> {
        let path = self
            .thread_dir(thread)?
            .join(format!("{}.json", unique_name("p", post.at_ms)));
        let encoded = serde_json::to_string(post)?;
        qol_fs::atomic_write(&path, encoded.as_bytes()).context("failed to write the board post")
    }

    fn open_thread(&self, author: &str, thread: &NewThread<'_>) -> Result<String> {
        fs::create_dir_all(&self.dir).context("failed to create the board directory")?;
        let at_ms = now_ms();
        let id = loop {
            let id = unique_name("t", at_ms);
            match fs::create_dir(self.dir.join(&id)) {
                Ok(()) => break id,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("failed to create the board thread"),
            }
        };
        self.append(
            &id,
            &Post {
                author: author.to_owned(),
                text: thread.text.to_owned(),
                at_ms,
                kind: Some(thread.kind),
                title: Some(thread.title.to_owned()),
                mentions: thread.mentions.to_vec(),
            },
        )?;
        Ok(id)
    }

    pub(super) fn read(&self, thread: &str) -> Result<Thread> {
        let dir = self.thread_dir(thread)?;
        if !dir.is_dir() {
            bail!("no board thread `{thread}`; list the board for thread ids");
        }
        let mut posts = json_files(&dir)?
            .iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|encoded| serde_json::from_str::<Post>(&encoded).ok())
            .collect::<Vec<_>>();
        posts.sort_by_key(|post| post.at_ms);
        let opening = posts
            .iter()
            .position(|post| post.title.is_some())
            .ok_or_else(|| anyhow!("board thread `{thread}` has no opening post yet"))?;
        let first = posts.remove(opening);
        posts.insert(0, first);
        let resolved_by = fs::read_to_string(dir.join(RESOLVED_FILE))
            .ok()
            .and_then(|encoded| serde_json::from_str::<Resolution>(&encoded).ok())
            .map(|resolution| resolution.by);
        Ok(Thread {
            id: thread.to_owned(),
            kind: posts[0].kind.unwrap_or(ThreadKind::HeadsUp),
            title: posts[0].title.clone().unwrap_or_default(),
            poster: posts[0].author.clone(),
            resolved_by,
            posts,
        })
    }

    pub(super) fn list(&self, open_only: bool) -> Result<Vec<ThreadSummary>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("failed to read the board directory"),
        };
        let mut threads = Vec::new();
        for entry in entries {
            let path = entry.context("failed to read the board directory")?.path();
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !path.is_dir() || id.starts_with('.') {
                continue;
            }
            let Ok(thread) = self.read(id) else {
                continue;
            };
            let resolved = thread.resolved_by.is_some();
            if open_only && resolved {
                continue;
            }
            threads.push(ThreadSummary {
                id: thread.id,
                kind: thread.kind,
                title: thread.title,
                poster: thread.poster,
                replies: thread.posts.len() - 1,
                updated: thread
                    .posts
                    .iter()
                    .map(|post| post.at_ms)
                    .max()
                    .unwrap_or(0),
                resolved,
            });
        }
        threads.sort_by(|left, right| {
            (right.kind == ThreadKind::ForUser)
                .cmp(&(left.kind == ThreadKind::ForUser))
                .then_with(|| right.updated.cmp(&left.updated))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(threads)
    }

    pub(super) fn posted_since(&self, author: &str, since_ms: u64) -> Result<usize> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error).context("failed to read the board directory"),
        };
        let mut count = 0;
        for entry in entries {
            let path = entry.context("failed to read the board directory")?.path();
            if !path.is_dir() {
                continue;
            }
            count += json_files(&path)?
                .iter()
                .filter_map(|path| fs::read_to_string(path).ok())
                .filter_map(|encoded| serde_json::from_str::<Post>(&encoded).ok())
                .filter(|post| post.author == author && post.at_ms >= since_ms)
                .count();
        }
        Ok(count)
    }
}

pub(super) struct Board<'a> {
    pub(super) board: &'a BoardStore,
    pub(super) mail: &'a MailboxStore,
    pub(super) knocks: &'a KnockStore,
}

impl Board<'_> {
    pub(super) fn post(&self, author: &str, thread: &NewThread<'_>) -> Result<String> {
        validate_title(thread.title)?;
        super::mailbox::validate_text(thread.text, "post")?;
        if thread.mentions.len() > MENTIONS_MAX {
            bail!("a post mentions at most {MENTIONS_MAX} keys");
        }
        for mention in thread.mentions {
            key_component(mention)
                .map_err(|_| anyhow!("mention `{mention}` is not a session key"))?;
        }
        check_rate(self.mail, self.board, author)?;
        let id = self.board.open_thread(author, thread)?;
        let mut knocked = Vec::new();
        for mention in thread.mentions {
            if mention == author || knocked.contains(mention) {
                continue;
            }
            self.knocks
                .add_board(mention, &id, true)
                .context("failed to queue the mention knock")?;
            knocked.push(mention.clone());
        }
        qol_runtime::probe!(
            "CLI_SESSION_BOARD",
            "event=posted thread={} author={} kind={:?} mentions={}",
            id,
            author,
            thread.kind,
            knocked.len()
        );
        Ok(id)
    }

    pub(super) fn reply(&self, author: &str, thread: &str, text: &str) -> Result<Thread> {
        super::mailbox::validate_text(text, "reply")?;
        let current = self.board.read(thread)?;
        if current.resolved_by.is_some() {
            bail!("board thread `{thread}` is resolved; open a new thread instead");
        }
        check_rate(self.mail, self.board, author)?;
        self.board.append(
            thread,
            &Post {
                author: author.to_owned(),
                text: text.to_owned(),
                at_ms: now_ms(),
                kind: None,
                title: None,
                mentions: Vec::new(),
            },
        )?;
        if current.poster != author {
            self.knocks
                .add_board(&current.poster, thread, false)
                .context("failed to queue the reply knock")?;
        }
        self.board.read(thread)
    }

    pub(super) fn resolve(&self, by: &str, thread: &str) -> Result<Thread> {
        let current = self.board.read(thread)?;
        if current.poster != by && by != SWITCHBOARD_KEY {
            bail!(
                "only the poster `{}` or the switchboard resolves board thread `{thread}`",
                current.poster
            );
        }
        if current.resolved_by.is_none() {
            let encoded = serde_json::to_string(&Resolution {
                by: by.to_owned(),
                at_ms: now_ms(),
            })?;
            qol_fs::atomic_write(
                &self.board.thread_dir(thread)?.join(RESOLVED_FILE),
                encoded.as_bytes(),
            )
            .context("failed to resolve the board thread")?;
        }
        self.board.read(thread)
    }

    pub(super) fn read(&self, reader: Option<&str>, thread: &str) -> Result<Thread> {
        let thread = self.board.read(thread)?;
        if let Some(reader) = reader {
            self.knocks
                .clear_board(reader, &thread.id)
                .context("failed to clear the board knock")?;
        }
        Ok(thread)
    }
}

fn validate_title(title: &str) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a board post needs a title");
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        bail!("a board title is at most {TITLE_MAX_CHARS} characters");
    }
    if title.chars().any(char::is_control) {
        bail!("a board title is one line with no control characters");
    }
    super::mailbox::validate_text(title, "title")
}

#[cfg(test)]
mod tests {
    use super::super::mailbox::tests::Stores;
    use super::*;

    fn board(stores: &Stores) -> Board<'_> {
        Board {
            board: &stores.board,
            mail: &stores.mail,
            knocks: &stores.knocks,
        }
    }

    fn thread<'a>(kind: ThreadKind, title: &'a str, mentions: &'a [String]) -> NewThread<'a> {
        NewThread {
            kind,
            title,
            text: "details",
            mentions,
        }
    }

    #[test]
    fn a_post_opens_a_thread_and_replies_append_and_knock_the_poster() {
        let stores = Stores::new();
        let board = board(&stores);
        let id = board
            .post(
                "lane-a",
                &thread(ThreadKind::Question, "Who owns port 8080?", &[]),
            )
            .unwrap();
        assert!(
            stores.knocks.pending_knocks().is_empty(),
            "posting interrupts nobody"
        );

        board.reply("lane-b", &id, "the lights plugin").unwrap();
        let read = board.reply("lane-c", &id, "confirmed").unwrap();
        assert_eq!(read.poster, "lane-a");
        assert_eq!(read.posts.len(), 3);
        assert_eq!(read.posts[0].title.as_deref(), Some("Who owns port 8080?"));
        assert_eq!(read.posts[1].author, "lane-b");
        let knocks = stores.knocks.pending_knocks();
        assert_eq!(knocks.len(), 1);
        assert_eq!(knocks[0].recipient_key, "lane-a");
        assert_eq!(
            knocks[0].text,
            format!("[qol session] board reply in thread {id}. Read it with session_board.")
        );

        board.reply("lane-a", &id, "thanks").unwrap();
        board.read(Some("lane-a"), &id).unwrap();
        assert!(
            stores.knocks.pending_knocks().is_empty(),
            "reading the thread clears its knock"
        );
    }

    #[test]
    fn mentions_knock_each_named_key_once_but_never_the_poster() {
        let stores = Stores::new();
        let board = board(&stores);
        let mentions = [
            "lane-b".to_owned(),
            "lane-b".to_owned(),
            "lane-a".to_owned(),
        ];
        let id = board
            .post(
                "lane-a",
                &thread(ThreadKind::HeadsUp, "Lockfile churn", &mentions),
            )
            .unwrap();
        let knocks = stores.knocks.pending_knocks();
        assert_eq!(knocks.len(), 1);
        assert_eq!(knocks[0].recipient_key, "lane-b");
        assert!(knocks[0]
            .text
            .contains(&format!("board mention in thread {id}")));
    }

    #[test]
    fn list_puts_for_user_first_and_hides_resolved_threads_unless_asked() {
        let stores = Stores::new();
        let board = board(&stores);
        let found = board
            .post("lane-a", &thread(ThreadKind::Found, "Root cause", &[]))
            .unwrap();
        let ask = board
            .post("lane-b", &thread(ThreadKind::ForUser, "Pick a model", &[]))
            .unwrap();
        board.reply("lane-c", &found, "nice").unwrap();
        let listed = stores.board.list(true).unwrap();
        assert_eq!(listed[0].id, ask);
        assert_eq!(listed[1].id, found);
        assert_eq!(listed[1].replies, 1);
        assert_eq!(listed[1].poster, "lane-a");

        let refused = board.resolve("lane-c", &found).unwrap_err().to_string();
        assert!(refused.contains("only the poster"), "{refused}");
        board.resolve("lane-a", &found).unwrap();
        board.resolve(SWITCHBOARD_KEY, &ask).unwrap();
        assert!(stores.board.list(true).unwrap().is_empty());
        let all = stores.board.list(false).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|summary| summary.resolved));
        let refused = board
            .reply("lane-c", &found, "late")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("resolved"), "{refused}");
    }

    #[test]
    fn posts_share_the_message_caps() {
        let stores = Stores::new();
        let board = board(&stores);
        let long_title = "t".repeat(TITLE_MAX_CHARS + 1);
        let cases: Vec<(NewThread<'_>, &str)> = vec![
            (thread(ThreadKind::Found, "  ", &[]), "needs a title"),
            (thread(ThreadKind::Found, "a\nb", &[]), "one line"),
            (thread(ThreadKind::Found, &long_title, &[]), "at most"),
            (
                thread(ThreadKind::Found, "QOL_BRIDGE_DONE_x", &[]),
                "completion-marker",
            ),
        ];
        for (new_thread, expected) in cases {
            let refused = board.post("lane-a", &new_thread).unwrap_err().to_string();
            assert!(refused.contains(expected), "{expected}: {refused}");
        }
        let mut oversized = thread(ThreadKind::Found, "Big", &[]);
        let text = "x".repeat(super::super::mailbox::TEXT_MAX_BYTES + 1);
        oversized.text = &text;
        assert!(board.post("lane-a", &oversized).is_err());
        let bad_mention = ["../x".to_owned()];
        assert!(board
            .post("lane-a", &thread(ThreadKind::Found, "Hi", &bad_mention))
            .is_err());
        assert!(stores.board.list(false).unwrap().is_empty());
        assert!(board.read(None, "t-missing").is_err());
        assert!(board.read(None, "..").is_err());
        for index in 0..super::super::mailbox::RATE_MAX {
            board
                .post(
                    "lane-a",
                    &thread(ThreadKind::Found, &format!("n{index}"), &[]),
                )
                .unwrap();
        }
        let refused = board
            .post("lane-a", &thread(ThreadKind::Found, "one more", &[]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("slow down"), "{refused}");
    }

    #[test]
    fn concurrent_repliers_never_lose_or_corrupt_a_post() {
        let stores = Stores::new();
        let id = board(&stores)
            .post("lane-a", &thread(ThreadKind::Question, "Race", &[]))
            .unwrap();
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let store = &stores.board;
                let id = &id;
                scope.spawn(move || {
                    for post in 0..5 {
                        store
                            .append(
                                id,
                                &Post {
                                    author: format!("w{writer}"),
                                    text: format!("post {post}"),
                                    at_ms: now_ms(),
                                    kind: None,
                                    title: None,
                                    mentions: Vec::new(),
                                },
                            )
                            .unwrap();
                    }
                });
            }
        });
        let thread = stores.board.read(&id).unwrap();
        assert_eq!(thread.posts.len(), 41);
        assert_eq!(thread.posts[0].author, "lane-a");
    }
}
