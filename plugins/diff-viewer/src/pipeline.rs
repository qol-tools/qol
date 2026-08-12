use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use qol_diff::{DiffError, FileDiff};
use qol_git::NumstatEntry;

pub const DEFAULT_RANGE: &str = "HEAD";
pub const HISTORY_LIMIT: usize = 64;
const WATCH_MAX_LATENCY: Duration = Duration::from_secs(1);
pub const WATCH_BUDGET: usize = 20_000;

#[derive(Debug, Clone)]
pub enum GitRequest {
    Refresh {
        generation: u64,
        changed: Vec<PathBuf>,
    },
    SelectFile {
        generation: u64,
        path: String,
        range: String,
    },
}

#[derive(Debug, Clone)]
pub enum GitResult {
    Facts {
        generation: u64,
        facts: Facts,
    },
    FactsFailed {
        generation: u64,
        message: String,
    },
    Diff {
        generation: u64,
        path: String,
        range: String,
        diff: Result<FileDiff, DiffError>,
        touched_at: Option<Instant>,
    },
    History {
        generation: u64,
        commits: Vec<qol_git::LogEntry>,
        magnitudes: Vec<qol_git::LogStatEntry>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub numstat: Vec<NumstatEntry>,
    pub changed: Vec<PathBuf>,
}

impl PartialEq for Facts {
    fn eq(&self, other: &Self) -> bool {
        self.numstat == other.numstat
    }
}

pub fn file_range(path: &str) -> String {
    format!("{DEFAULT_RANGE} -- {path}")
}

pub fn commit_range(index: usize) -> String {
    match index {
        0 => DEFAULT_RANGE.to_string(),
        1 => "HEAD~1..HEAD".to_string(),
        _ => format!("HEAD~{}..HEAD~{}", index, index - 1),
    }
}

pub fn resolve_repo(launch_cwd: &Path, env_repo: Option<&Path>) -> Option<PathBuf> {
    if let Some(env) = env_repo {
        let candidate = if env.is_absolute() {
            env.to_path_buf()
        } else {
            launch_cwd.join(env)
        };
        if candidate.join(".git").exists() {
            return Some(candidate);
        }
    }
    for ancestor in launch_cwd.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub fn send_refresh(git_tx: &mpsc::Sender<GitRequest>, generation: &AtomicU64) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = git_tx.send(GitRequest::Refresh {
        generation: g,
        changed: Vec::new(),
    });
}

pub fn send_boot(git_tx: &mpsc::Sender<GitRequest>, generation: &AtomicU64) {
    send_refresh(git_tx, generation);
}

pub fn send_refresh_changed(
    git_tx: &mpsc::Sender<GitRequest>,
    generation: &AtomicU64,
    changed: Vec<PathBuf>,
) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = git_tx.send(GitRequest::Refresh {
        generation: g,
        changed,
    });
}

pub fn spawn_watch_bridge(
    batches: mpsc::Receiver<Vec<PathBuf>>,
    git_tx: mpsc::Sender<GitRequest>,
    generation: Arc<AtomicU64>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("diff-viewer-watch-bridge".to_owned())
        .spawn(move || {
            let mut changed: Vec<PathBuf> = Vec::new();
            let mut deadline: Option<Instant> = None;
            loop {
                let timeout = match deadline {
                    Some(deadline) => deadline
                        .saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1)),
                    None => Duration::from_secs(3600),
                };
                match batches.recv_timeout(timeout) {
                    Ok(paths) => {
                        let real: Vec<PathBuf> = paths
                            .into_iter()
                            .filter(|path| !is_watch_noise(path))
                            .collect();
                        if real.is_empty() {
                            continue;
                        }
                        if deadline.is_none() {
                            deadline = Some(Instant::now() + WATCH_MAX_LATENCY);
                        }
                        for path in real {
                            if !changed.contains(&path) {
                                changed.push(path);
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if deadline.take().is_some() {
                            let paths = std::mem::take(&mut changed);
                            send_refresh_changed(&git_tx, &generation, paths);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("spawn diff-viewer watch bridge")
}

fn is_watch_noise(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git") | Some("target") | Some("node_modules")
        )
    })
}

pub fn spawn_git_facts_thread(
    repo: PathBuf,
    requests: mpsc::Receiver<GitRequest>,
    results: mpsc::Sender<GitResult>,
    generation: Arc<AtomicU64>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("diff-viewer-git-facts".to_owned())
        .spawn(move || {
            let mut last_head: Option<String> = None;
            let mut heat_stamps: HashMap<PathBuf, Instant> = HashMap::new();
            while let Ok(first) = requests.recv() {
                let mut pending = vec![first];
                while let Ok(next) = requests.try_recv() {
                    pending.push(next);
                }
                let mut latest_refresh = None;
                let mut latest_select = None;
                for request in pending {
                    match request {
                        GitRequest::Refresh { .. } => latest_refresh = Some(request),
                        GitRequest::SelectFile { .. } => latest_select = Some(request),
                    }
                }
                for request in latest_refresh.into_iter().chain(latest_select) {
                    handle_request(
                        request,
                        &repo,
                        &results,
                        &generation,
                        &mut last_head,
                        &mut heat_stamps,
                    );
                }
            }
        })
        .expect("spawn diff-viewer git facts thread")
}

fn handle_request(
    request: GitRequest,
    repo: &Path,
    results: &mpsc::Sender<GitResult>,
    generation: &AtomicU64,
    last_head: &mut Option<String>,
    heat_stamps: &mut HashMap<PathBuf, Instant>,
) {
    match request {
        GitRequest::Refresh {
            generation: g,
            changed,
        } => {
            if !is_live(g, generation) {
                return;
            }
            let changed: Vec<PathBuf> = changed
                .into_iter()
                .map(|path| repo_relative(repo, &path))
                .collect();
            let now = Instant::now();
            for path in &changed {
                heat_stamps.insert(path.clone(), now);
            }
            let outcome = refresh_facts(repo, changed);
            if !is_live(g, generation) {
                return;
            }
            let result = match outcome {
                Ok(facts) => GitResult::Facts {
                    generation: g,
                    facts,
                },
                Err(message) => GitResult::FactsFailed {
                    generation: g,
                    message,
                },
            };
            let _ = results.send(result);
            let (commits, magnitudes) =
                qol_git::log_with_stats(repo, HISTORY_LIMIT).unwrap_or_default();
            let head = commits.first().map(|entry| entry.sha.clone());
            if head != *last_head {
                *last_head = head;
                if is_live(g, generation) {
                    let _ = results.send(GitResult::History {
                        generation: g,
                        commits,
                        magnitudes,
                    });
                }
            }
        }
        GitRequest::SelectFile {
            generation: g,
            path,
            range,
        } => {
            if range == DEFAULT_RANGE {
                heat_stamps.insert(PathBuf::from(&path), Instant::now());
            }
            let diff = selected_file_diff(repo, &path, &range);
            let touched_at = heat_stamps.get(Path::new(&path)).copied();
            let _ = results.send(GitResult::Diff {
                generation: g,
                path,
                range,
                diff,
                touched_at,
            });
        }
    }
}

fn repo_relative(repo: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(repo)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn is_live(generation: u64, current: &AtomicU64) -> bool {
    generation == current.load(Ordering::SeqCst)
}

fn refresh_facts(repo: &Path, changed: Vec<PathBuf>) -> Result<Facts, String> {
    let numstat = qol_git::diff_numstat(repo, DEFAULT_RANGE).map_err(|error| error.to_string())?;
    Ok(Facts { numstat, changed })
}

fn selected_file_diff(repo: &Path, path: &str, range: &str) -> Result<FileDiff, DiffError> {
    let patch = qol_git::diff_patch(repo, range, &[path]).map_err(|_| DiffError::Other)?;
    let mut diff = qol_diff::engine::parse_patch(path, path, &patch)?;
    qol_diff::engine::apply_heat(&mut diff);
    Ok(diff)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn watch_noise_filters_build_and_vcs_output() {
        assert!(is_watch_noise(Path::new("/repo/target/debug/plugin")),);
        assert!(is_watch_noise(Path::new("/repo/.git/index")));
        assert!(is_watch_noise(Path::new("/repo/src/node_modules/x")));
        assert!(!is_watch_noise(Path::new("/repo/src/main.rs")));
        assert!(!is_watch_noise(Path::new("/repo/targeted/x")));
    }

    #[test]
    fn resolve_repo_walks_up_to_the_nearest_git_root() {
        let dir = std::env::temp_dir().join(format!(
            "diff-viewer-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub/dir")).expect("create tree");
        std::fs::create_dir(dir.join(".git")).expect("create .git");
        let root = resolve_repo(&dir.join("sub/dir"), None);
        assert_eq!(root, Some(dir.clone()));
        assert_eq!(
            resolve_repo(&dir.join("sub/dir"), Some(Path::new("/nonexistent"))),
            Some(dir.clone()),
            "an invalid env repo falls back to the walk-up root"
        );
        assert_eq!(
            resolve_repo(&dir.join("sub/dir"), Some(&dir.join("sub"))),
            Some(dir.clone()),
            "an env path without .git also falls back to the walk-up root"
        );
        let other = dir.join("other");
        std::fs::create_dir(&other).expect("create other");
        std::fs::create_dir(other.join(".git")).expect("create other .git");
        assert_eq!(
            resolve_repo(&dir.join("sub/dir"), Some(&other)),
            Some(other.clone()),
            "a valid env repo wins over the walk-up root"
        );
        let bare =
            std::env::temp_dir().join(format!("diff-viewer-resolve-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(bare.join("x")).expect("create bare tree");
        assert_eq!(resolve_repo(&bare.join("x"), None), None);
        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_range_maps_scrub_index_to_git_range() {
        assert_eq!(commit_range(0), "HEAD");
        assert_eq!(commit_range(1), "HEAD~1..HEAD");
        assert_eq!(commit_range(3), "HEAD~3..HEAD~2");
    }

    #[test]
    fn selected_file_diff_parses_and_heats_a_real_repo_change() {
        let dir = std::env::temp_dir().join(format!(
            "diff-viewer-pipeline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create repo dir");
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git run");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "let x = 1;\n").expect("write");
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "let x = 2;\n").expect("write");
        let diff = selected_file_diff(&dir, "a.txt", DEFAULT_RANGE).expect("diff");
        assert!(!diff.is_empty());
        assert_eq!(diff.hunks.len(), 1);
        let added = diff.hunks[0]
            .lines
            .iter()
            .find(|line| line.kind == qol_diff::LineKind::Added)
            .expect("added line");
        assert_eq!(added.text, "let x = 2;");
        assert_eq!(added.old_line_no, None);
        assert_eq!(added.new_line_no, Some(1));
        assert!(!added.token_spans.is_empty(), "heat spans must be computed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_refresh_requests_are_dropped_before_any_git_spawn() {
        let generation = Arc::new(AtomicU64::new(5));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(
            PathBuf::from("/nonexistent"),
            request_rx,
            result_tx,
            generation.clone(),
        );
        let _ = requests.send(GitRequest::Refresh {
            generation: 4,
            changed: Vec::new(),
        });
        assert!(
            results.recv_timeout(Duration::from_millis(200)).is_err(),
            "a stale generation must never reach the git CLI"
        );
    }

    #[test]
    fn refresh_delivers_history_with_magnitudes() {
        let dir = std::env::temp_dir().join(format!(
            "diff-viewer-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create repo dir");
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git run");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "one\n").expect("write");
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").expect("write");
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "second"]);
        let generation = Arc::new(AtomicU64::new(1));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(dir.clone(), request_rx, result_tx, generation);
        let _ = requests.send(GitRequest::Refresh {
            generation: 1,
            changed: Vec::new(),
        });
        let mut history = None;
        while let Ok(result) = results.recv_timeout(Duration::from_secs(10)) {
            if let GitResult::History {
                commits,
                magnitudes,
                ..
            } = result
            {
                history = Some((commits, magnitudes));
                break;
            }
        }
        let (commits, magnitudes) = history.expect("history result");
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "second");
        assert_eq!(magnitudes.len(), 2);
        assert_eq!(magnitudes[0].sha, commits[0].sha, "magnitudes align by sha");
        assert_eq!(magnitudes[0].magnitude(), 2, "second adds two lines");
        assert_eq!(
            magnitudes[1].magnitude(),
            1,
            "the root commit counts its tree"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn coalescing_keeps_only_the_latest_request_of_each_kind() {
        let generation = Arc::new(AtomicU64::new(6));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(
            PathBuf::from("/nonexistent"),
            request_rx,
            result_tx,
            generation.clone(),
        );
        for index in 0..6 {
            let _ = requests.send(GitRequest::Refresh {
                generation: 1 + index,
                changed: Vec::new(),
            });
            let _ = requests.send(GitRequest::SelectFile {
                generation: 1 + index,
                path: format!("path-{index}"),
                range: "HEAD".to_string(),
            });
        }
        drop(requests);
        let mut refreshes = 0;
        let mut diffs = 0;
        while let Ok(result) = results.recv() {
            match result {
                GitResult::Facts { .. } | GitResult::FactsFailed { .. } => refreshes += 1,
                GitResult::Diff { .. } => diffs += 1,
                _ => {}
            }
        }
        assert_eq!(refreshes, 1, "only the latest refresh is processed");
        assert_eq!(diffs, 1, "only the latest select is processed");
    }

    #[test]
    fn repo_relative_strips_the_repo_prefix_and_keeps_other_paths() {
        let repo = Path::new("/repo");
        assert_eq!(
            repo_relative(repo, Path::new("/repo/src/a.rs")),
            PathBuf::from("src/a.rs")
        );
        assert_eq!(
            repo_relative(repo, Path::new("src/a.rs")),
            PathBuf::from("src/a.rs"),
            "already-relative paths pass through"
        );
    }

    #[test]
    fn commit_range_selects_report_no_stamp_for_never_watched_paths() {
        let generation = Arc::new(AtomicU64::new(1));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(
            PathBuf::from("/nonexistent"),
            request_rx,
            result_tx,
            generation.clone(),
        );
        let _ = requests.send(GitRequest::SelectFile {
            generation: 1,
            path: "src/a.rs".to_string(),
            range: "HEAD~1..HEAD".to_string(),
        });
        drop(requests);
        let mut touched_at = None;
        while let Ok(result) = results.recv() {
            if let GitResult::Diff {
                touched_at: touched,
                ..
            } = result
            {
                touched_at = Some(touched);
            }
        }
        assert_eq!(
            touched_at,
            Some(None),
            "history selects never stamp, so heat cannot reset on scrub"
        );
    }

    #[test]
    fn watch_stamps_use_relative_keys_and_worktree_selects_reheat() {
        let generation = Arc::new(AtomicU64::new(1));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(
            PathBuf::from("/nonexistent"),
            request_rx,
            result_tx,
            generation.clone(),
        );
        let _ = requests.send(GitRequest::Refresh {
            generation: 1,
            changed: vec![PathBuf::from("/nonexistent/src/a.rs")],
        });
        let _ = requests.send(GitRequest::SelectFile {
            generation: 1,
            path: "src/a.rs".to_string(),
            range: "HEAD~1..HEAD".to_string(),
        });
        drop(requests);
        let mut touched_at = None;
        while let Ok(result) = results.recv() {
            if let GitResult::Diff {
                touched_at: touched,
                ..
            } = result
            {
                touched_at = Some(touched);
            }
        }
        let touched_at = touched_at
            .expect("one select result")
            .expect("the watch stamp normalized to the relative key must reach the select");
        assert!(
            touched_at.elapsed() < Duration::from_secs(5),
            "the stamp is from the current refresh"
        );
    }

    #[test]
    fn worktree_selects_always_stamp_even_without_a_watch_event() {
        let generation = Arc::new(AtomicU64::new(1));
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let _thread = spawn_git_facts_thread(
            PathBuf::from("/nonexistent"),
            request_rx,
            result_tx,
            generation.clone(),
        );
        let _ = requests.send(GitRequest::SelectFile {
            generation: 1,
            path: "src/b.rs".to_string(),
            range: "HEAD".to_string(),
        });
        drop(requests);
        let mut touched_at = None;
        while let Ok(result) = results.recv() {
            if let GitResult::Diff {
                touched_at: touched,
                ..
            } = result
            {
                touched_at = Some(touched);
            }
        }
        assert!(
            touched_at.expect("one select result").is_some(),
            "applying heat to the worktree diff is itself a touch"
        );
    }
}
