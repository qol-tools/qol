use std::collections::HashSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const DIR: &str = "fork-watches";
const WATCHER_SCRIPT: &str = "pr-watch.cjs";
const WATCHER_PROGRAM: &str = "node";
const WATCHER_FLAGS: [&str; 2] = ["--pretty", "--until-ready"];
const CACHED_WATCHER: [&str; 4] = ["plugins", "cache", "qol-skills", "qol-workflow"];
const CLONED_WATCHER: [&str; 6] = [
    "plugins",
    "marketplaces",
    "qol-skills",
    "plugins",
    "qol-workflow",
    "bin",
];
const WATCHER_BIN: &str = "bin";
const PULL_HOST: &str = "https://github.com/";
const NAME_LIMIT: usize = 100;
const NUMBER_LIMIT: usize = 10;
const WATCH_PREFIX: &str = "watch-";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    Watching,
    Exited,
    Delivered,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchRequest {
    pub agent_id: String,
    pub conversation: String,
    pub name: Option<String>,
    pub cwd: String,
    pub command: Vec<String>,
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ForkWatch {
    pub id: String,
    pub agent_id: String,
    pub conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cwd: String,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
    pub created_at: u64,
    pub state: WatchState,
    #[serde(default)]
    pub restarts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restarted_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exited_at: Option<u64>,
    #[serde(default)]
    pub notified: bool,
}

impl ForkWatch {
    pub fn start(id: String, request: WatchRequest, created_at: u64) -> Self {
        let pull_request = pull_request_of(&request.command);
        Self {
            id,
            agent_id: request.agent_id,
            conversation: request.conversation,
            name: request.name,
            cwd: request.cwd,
            command: request.command,
            path: request.path,
            pull_request,
            created_at,
            state: WatchState::Watching,
            restarts: 0,
            restarted_at: None,
            exit_code: None,
            merge_commit: None,
            detail: None,
            exited_at: None,
            notified: false,
        }
    }
}

pub fn is_watcher(command: &[String]) -> bool {
    command.iter().any(|arg| arg.ends_with(WATCHER_SCRIPT))
}

pub fn is_watch_id(id: &str) -> bool {
    id.strip_prefix(WATCH_PREFIX).is_some_and(|rest| {
        let parts = rest.split('-').collect::<Vec<_>>();
        parts.len() == 2
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.len() <= NAME_LIMIT
                    && part.bytes().all(|byte| byte.is_ascii_digit())
            })
    })
}

fn is_github_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

pub fn is_pull_request_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(PULL_HOST) else {
        return false;
    };
    match rest.split('/').collect::<Vec<_>>().as_slice() {
        [owner, repo, "pull", number] => {
            is_github_name(owner)
                && is_github_name(repo)
                && !number.is_empty()
                && number.len() <= NUMBER_LIMIT
                && number.bytes().all(|byte| byte.is_ascii_digit())
        }
        _ => false,
    }
}

fn is_installed_script(script: &str) -> Result<bool> {
    let path = Path::new(script);
    if !path.is_absolute() {
        return Ok(false);
    }
    let resolved = fs::canonicalize(path)
        .with_context(|| format!("the watcher script {script} cannot be resolved"))?;
    let parts = resolved
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let Some((name, folders)) = parts.split_last() else {
        return Ok(false);
    };
    let ends_with = |tail: &[&str]| {
        folders.len() >= tail.len()
            && folders[folders.len() - tail.len()..]
                .iter()
                .zip(tail)
                .all(|(folder, expected)| folder == expected)
    };
    let cached = match folders {
        [.., version, bin] if bin == WATCHER_BIN && is_version(version) => {
            let tail = CACHED_WATCHER
                .iter()
                .copied()
                .chain([version.as_str(), WATCHER_BIN])
                .collect::<Vec<_>>();
            ends_with(&tail)
        }
        _ => false,
    };
    Ok(name == WATCHER_SCRIPT && (cached || ends_with(&CLONED_WATCHER)) && resolved.is_file())
}

fn is_version(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
}

pub fn check_watcher(command: &[String]) -> Result<()> {
    let [program, script, url, flags @ ..] = command else {
        bail!("a fork watch runs only `node <installed qol-workflow>/bin/{WATCHER_SCRIPT} <pull request url> [--pretty] [--until-ready]`");
    };
    if program.as_str() != WATCHER_PROGRAM {
        bail!("a fork watch runs only `{WATCHER_PROGRAM}`, not `{program}`");
    }
    if !is_installed_script(script)? {
        bail!("a fork watch runs only the qol-workflow bin/{WATCHER_SCRIPT} installed under a Claude plugins folder, not {script}");
    }
    if !is_pull_request_url(url) {
        bail!("a fork watch needs a GitHub pull request URL, not `{url}`");
    }
    if let Some(flag) = flags
        .iter()
        .find(|flag| !WATCHER_FLAGS.contains(&flag.as_str()))
    {
        bail!("a fork watch takes only --pretty and --until-ready, not `{flag}`");
    }
    Ok(())
}

pub fn pull_request_of(command: &[String]) -> Option<String> {
    if !is_watcher(command) {
        return None;
    }
    crate::prs::pull_requests(&command.join(" "))
        .into_iter()
        .next()
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PullState {
    pub agent_id: String,
    pub pull_request: String,
    pub number: Option<u64>,
    pub merge_commit: Option<String>,
    pub is_watched: bool,
}

fn number_of(pull_request: &str) -> Option<u64> {
    pull_request.rsplit('/').next()?.parse().ok()
}

pub struct WatchStore {
    dir: PathBuf,
}

impl WatchStore {
    pub fn system() -> Result<Self> {
        qol_config::data_subdir("sessions")
            .map(|sessions| Self::with_dir(sessions.join(DIR)))
            .context("cannot resolve the qol-tray data directory for fork watches")
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str, extension: &str) -> PathBuf {
        self.dir.join(format!("{id}.{extension}"))
    }

    pub fn log_path(&self, id: &str) -> PathBuf {
        self.file(id, "log")
    }

    pub fn record(&self, watch: &ForkWatch) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create the fork watch directory")?;
        let encoded = serde_json::to_string(watch)?;
        qol_fs::atomic_write(&self.file(&watch.id, "json"), encoded.as_bytes())
            .context("failed to write the fork watch")
    }

    pub fn load(&self, id: &str) -> Result<ForkWatch> {
        if !is_watch_id(id) {
            bail!("`{id}` is not a fork watch id");
        }
        let path = self.file(id, "json");
        let encoded = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&encoded)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn list(&self) -> Vec<ForkWatch> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut watches = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|encoded| serde_json::from_str::<ForkWatch>(&encoded).ok())
            .collect::<Vec<_>>();
        watches.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        watches
    }

    pub fn claim(&self, id: &str) -> Result<bool> {
        fs::create_dir_all(&self.dir).context("failed to create the fork watch directory")?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.file(id, "claim"))
        {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error).context("failed to claim the fork watch"),
        }
    }

    pub fn release(&self, id: &str) {
        let _ = fs::remove_file(self.file(id, "claim"));
    }

    pub fn is_claimed(&self, id: &str) -> bool {
        self.file(id, "claim").exists()
    }

    fn lock_file(&self, id: &str, extension: &str) -> Result<File> {
        fs::create_dir_all(&self.dir).context("failed to create the fork watch directory")?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.file(id, extension))
            .context("failed to open a fork watch lock")
    }

    fn try_lock(&self, id: &str, extension: &str) -> Result<Option<File>> {
        let file = self.lock_file(id, extension)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error).context("failed to lock the fork watch"),
        }
    }

    pub fn attach(&self, id: &str) -> Result<File> {
        let file = self.lock_file(id, "attach")?;
        file.lock().context("failed to attach to the fork watch")?;
        Ok(file)
    }

    pub fn try_attach(&self, id: &str) -> Result<Option<File>> {
        self.try_lock(id, "attach")
    }

    pub fn is_attached(&self, id: &str) -> bool {
        matches!(self.try_lock(id, "attach"), Ok(None))
    }

    pub fn lock_runner(&self, id: &str) -> Result<Option<File>> {
        self.try_lock(id, "runner")
    }

    pub fn is_running(&self, id: &str) -> bool {
        matches!(self.try_lock(id, "runner"), Ok(None))
    }

    pub fn is_pending(&self, watch: &ForkWatch) -> bool {
        match watch.state {
            WatchState::Watching => true,
            WatchState::Exited | WatchState::Failed => !self.is_claimed(&watch.id),
            WatchState::Delivered => false,
        }
    }

    pub fn is_pending_for(&self, agent_id: &str) -> bool {
        self.list()
            .iter()
            .any(|watch| watch.agent_id == agent_id && self.is_pending(watch))
    }

    pub fn pulls(&self) -> Vec<PullState> {
        pulls_of(&self.list())
    }

    pub fn prune(&self, before: u64) {
        for watch in self.list() {
            let ended = matches!(watch.state, WatchState::Delivered | WatchState::Failed)
                && watch.exited_at.is_some_and(|at| at < before);
            if !ended || self.is_attached(&watch.id) {
                continue;
            }
            for extension in ["json", "log", "claim", "attach", "runner"] {
                let _ = fs::remove_file(self.file(&watch.id, extension));
            }
        }
    }
}

fn pulls_of(watches: &[ForkWatch]) -> Vec<PullState> {
    let mut seen = HashSet::new();
    watches
        .iter()
        .filter_map(|watch| {
            let pull_request = watch.pull_request.clone()?;
            seen.insert((watch.agent_id.clone(), pull_request.clone()))
                .then_some((watch, pull_request))
        })
        .map(|(newest, pull_request)| PullState {
            agent_id: newest.agent_id.clone(),
            number: number_of(&pull_request),
            merge_commit: watches
                .iter()
                .filter(|watch| watch.pull_request.as_deref() == Some(pull_request.as_str()))
                .find_map(|watch| watch.merge_commit.clone()),
            is_watched: newest.state == WatchState::Watching,
            pull_request,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(root: &Path, folders: &[&str]) -> String {
        let bin = folders
            .iter()
            .fold(root.to_path_buf(), |path, folder| path.join(folder));
        fs::create_dir_all(&bin).unwrap();
        let script = bin.join(WATCHER_SCRIPT);
        fs::write(&script, "").unwrap();
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn a_watch_id_is_watch_and_two_numbers_and_nothing_else_is_loaded() {
        assert!(is_watch_id("watch-1793538339-4242"));
        for refused in [
            "",
            "watch-",
            "watch-1",
            "watch-1-2-3",
            "watch-1-x",
            "../watch-1-2",
            "watch-1-2/..",
            "lane-1-2",
        ] {
            assert!(!is_watch_id(refused), "{refused}");
        }
        let root = tempfile::tempdir().unwrap();
        let store = WatchStore::with_dir(root.path().join(DIR));
        assert!(store.load("../../etc/passwd").is_err());
    }

    #[test]
    fn only_the_installed_watcher_on_a_pull_request_url_is_watched() {
        let root = tempfile::tempdir().unwrap();
        let script = installed(
            root.path(),
            &[
                "plugins",
                "cache",
                "qol-skills",
                "qol-workflow",
                "0.11.27",
                "bin",
            ],
        );
        let cloned = installed(root.path(), &CLONED_WATCHER);
        let stray = installed(
            root.path(),
            &["elsewhere", "qol-skills", "qol-workflow", "bin"],
        );
        let renamed = root
            .path()
            .join("plugins/cache/qol-skills/qol-workflow/0.11.27/bin/watch.cjs");
        fs::write(&renamed, "").unwrap();
        let renamed = renamed.to_string_lossy().into_owned();
        let escaped = format!(
            "{}/../../0.11.27/bin/{WATCHER_SCRIPT}",
            root.path()
                .join("plugins/cache/qol-skills/qol-workflow/0.11.27/bin")
                .display()
        );
        let url = "https://github.com/qol-tools/qol/pull/426";
        let command = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>()
        };
        for accepted in [
            command(&["node", &script, url]),
            command(&["node", &script, url, "--pretty"]),
            command(&["node", &script, url, "--pretty", "--until-ready"]),
            command(&["node", &cloned, url, "--pretty"]),
            command(&["node", &escaped, url]),
        ] {
            assert!(check_watcher(&accepted).is_ok(), "{accepted:?}");
        }
        for refused in [
            command(&["sh", "-c", "true"]),
            command(&["node", &script]),
            command(&["bash", &script, url]),
            command(&["node", &stray, url]),
            command(&["node", &renamed, url]),
            command(&[
                "node",
                "/nowhere/plugins/cache/qol-skills/qol-workflow/1/bin/pr-watch.cjs",
                url,
            ]),
            command(&[
                "node",
                "relative/qol-skills/qol-workflow/bin/pr-watch.cjs",
                url,
            ]),
            command(&["node", &script, "https://example.com/o/r/pull/1"]),
            command(&["node", &script, "https://github.com/o/r/pull/1/../2"]),
            command(&["node", &script, "https://github.com/o/r/issues/1"]),
            command(&["node", &script, url, "--eval"]),
            command(&["node", &script, url, "; rm -rf /"]),
        ] {
            assert!(check_watcher(&refused).is_err(), "{refused:?}");
        }
    }

    const PR: &str = "https://github.com/qol-tools/qol-monorepo/pull/424";

    fn request(agent_id: &str, command: &[&str]) -> WatchRequest {
        WatchRequest {
            agent_id: agent_id.to_owned(),
            conversation: "conv".to_owned(),
            name: Some("turn-free-resume: resume without a turn".to_owned()),
            cwd: "/work".to_owned(),
            command: command.iter().map(|arg| (*arg).to_owned()).collect(),
            path: None,
        }
    }

    fn watcher(id: &str, agent_id: &str, created_at: u64, state: WatchState) -> ForkWatch {
        ForkWatch {
            state,
            ..ForkWatch::start(
                id.to_owned(),
                request(agent_id, &["node", "/q/bin/pr-watch.cjs", PR, "--pretty"]),
                created_at,
            )
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_watcher_command_names_its_pull_request_and_other_commands_none() {
        let watch = watcher("watch-1-1", "a1", 1, WatchState::Watching);
        assert_eq!(watch.pull_request.as_deref(), Some(PR));
        assert!(is_watcher(&watch.command));
        assert!(!is_watcher(&args(&["sleep", "5"])));
        assert_eq!(pull_request_of(&args(&["gh", "pr", "view", PR])), None);
        assert_eq!(
            pull_request_of(&args(&["node", "pr-watch.cjs", "424"])),
            None
        );
    }

    #[test]
    fn the_store_lists_newest_first_and_only_the_first_claim_wins() {
        let root = tempfile::tempdir().unwrap();
        let store = WatchStore::with_dir(root.path().join(DIR));
        store
            .record(&watcher("watch-1-1", "a1", 1, WatchState::Exited))
            .unwrap();
        store
            .record(&watcher("watch-2-2", "a1", 2, WatchState::Watching))
            .unwrap();
        let ids = store
            .list()
            .into_iter()
            .map(|watch| watch.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["watch-2-2", "watch-1-1"]);
        assert!(store.claim("watch-1-1").unwrap());
        assert!(!store.claim("watch-1-1").unwrap());
        assert!(store.is_claimed("watch-1-1"));
        assert_eq!(store.load("watch-2-2").unwrap().state, WatchState::Watching);
    }

    #[test]
    fn a_fork_is_pending_while_it_is_watched_or_its_result_is_unclaimed() {
        let root = tempfile::tempdir().unwrap();
        let store = WatchStore::with_dir(root.path().join(DIR));
        for (id, agent, at, state) in [
            ("watch-1-1", "watching", 1, WatchState::Watching),
            ("watch-2-2", "exited", 2, WatchState::Exited),
            ("watch-3-3", "claimed", 3, WatchState::Exited),
            ("watch-4-4", "delivered", 4, WatchState::Delivered),
        ] {
            store.record(&watcher(id, agent, at, state)).unwrap();
        }
        store.claim("watch-3-3").unwrap();
        let pending = ["watching", "exited", "claimed", "delivered", "unknown"]
            .map(|agent| store.is_pending_for(agent));
        assert_eq!(pending, [true, true, false, false, false]);
    }

    #[test]
    fn the_attach_and_runner_locks_show_who_holds_a_watch() {
        let root = tempfile::tempdir().unwrap();
        let store = WatchStore::with_dir(root.path().join(DIR));
        assert!(!store.is_attached("watch-1-1"));
        let attached = store.attach("watch-1-1").unwrap();
        assert!(store.is_attached("watch-1-1"));
        drop(attached);
        assert!(!store.is_attached("watch-1-1"));
        let running = store.lock_runner("watch-1-1").unwrap();
        assert!(running.is_some());
        assert!(store.is_running("watch-1-1"));
        assert!(store.lock_runner("watch-1-1").unwrap().is_none());
        drop(running);
        assert!(!store.is_running("watch-1-1"));
    }

    #[test]
    fn the_ledger_keeps_each_pull_request_once_with_its_merge_commit_or_null() {
        let merged = ForkWatch {
            merge_commit: Some("1a4ac81fb6080dad6412272ca1e6b05e313608b1".to_owned()),
            ..watcher("watch-1-1", "a1", 1, WatchState::Delivered)
        };
        let rewatched = watcher("watch-2-2", "a1", 2, WatchState::Delivered);
        let open = ForkWatch {
            pull_request: Some("https://github.com/o/r/pull/7".to_owned()),
            ..watcher("watch-3-3", "b2", 3, WatchState::Watching)
        };
        let unwatched = ForkWatch {
            pull_request: Some("https://github.com/o/r/pull/8".to_owned()),
            ..watcher("watch-4-4", "b2", 4, WatchState::Exited)
        };
        let pulls = pulls_of(&[unwatched, open, rewatched, merged]);
        assert_eq!(
            pulls
                .iter()
                .map(|pull| (
                    pull.agent_id.as_str(),
                    pull.number,
                    pull.merge_commit.as_deref().map(|hash| &hash[..7]),
                    pull.is_watched
                ))
                .collect::<Vec<_>>(),
            [
                ("b2", Some(8), None, false),
                ("b2", Some(7), None, true),
                ("a1", Some(424), Some("1a4ac81"), false),
            ]
        );
    }

    #[test]
    fn pruning_removes_only_ended_watches_from_before_the_cutoff() {
        let root = tempfile::tempdir().unwrap();
        let store = WatchStore::with_dir(root.path().join(DIR));
        let ended = |id: &str, state, at| ForkWatch {
            exited_at: Some(at),
            ..watcher(id, "a1", at, state)
        };
        store
            .record(&ended("watch-1-1", WatchState::Delivered, 10))
            .unwrap();
        store
            .record(&ended("watch-2-2", WatchState::Delivered, 30))
            .unwrap();
        store
            .record(&ended("watch-3-3", WatchState::Exited, 10))
            .unwrap();
        store.prune(20);
        let ids = store
            .list()
            .into_iter()
            .map(|watch| watch.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["watch-3-3", "watch-2-2"]);
    }
}
