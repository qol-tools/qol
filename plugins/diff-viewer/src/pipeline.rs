use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use qol_diff::{DiffError, FileDiff};
use qol_git::{NumstatEntry, StatusEntry};

pub const DEFAULT_RANGE: &str = "HEAD";
pub const HISTORY_LIMIT: usize = 64;

#[derive(Debug, Clone)]
pub enum GitRequest {
    Refresh {
        generation: u64,
    },
    SelectFile {
        generation: u64,
        path: String,
        range: String,
    },
    History {
        generation: u64,
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
        diff: Result<FileDiff, DiffError>,
    },
    History {
        generation: u64,
        commits: Vec<qol_git::LogEntry>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub status: Vec<StatusEntry>,
    pub numstat: Vec<NumstatEntry>,
}

pub fn file_range(path: &str) -> String {
    format!("{DEFAULT_RANGE} -- {path}")
}

pub fn commit_range(index: usize) -> String {
    match index {
        0 => DEFAULT_RANGE.to_string(),
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

pub fn send_history(git_tx: &mpsc::Sender<GitRequest>, generation: &AtomicU64) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = git_tx.send(GitRequest::History { generation: g });
}

pub fn send_refresh(git_tx: &mpsc::Sender<GitRequest>, generation: &AtomicU64) {
    let g = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = git_tx.send(GitRequest::Refresh { generation: g });
}

pub fn spawn_watch_bridge(
    batches: mpsc::Receiver<Vec<PathBuf>>,
    git_tx: mpsc::Sender<GitRequest>,
    generation: Arc<AtomicU64>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("diff-viewer-watch-bridge".to_owned())
        .spawn(move || {
            while let Ok(paths) = batches.recv() {
                if !paths.is_empty() {
                    send_refresh(&git_tx, &generation);
                }
            }
        })
        .expect("spawn diff-viewer watch bridge")
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
            for request in requests {
                match request {
                    GitRequest::Refresh { generation: g } => {
                        if !is_live(g, &generation) {
                            continue;
                        }
                        let outcome = refresh_facts(&repo);
                        if !is_live(g, &generation) {
                            continue;
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
                    }
                    GitRequest::SelectFile {
                        generation: g,
                        path,
                        range,
                    } => {
                        if !is_live(g, &generation) {
                            continue;
                        }
                        let diff = selected_file_diff(&repo, &path, &range);
                        if !is_live(g, &generation) {
                            continue;
                        }
                        let _ = results.send(GitResult::Diff {
                            generation: g,
                            path,
                            diff,
                        });
                    }
                    GitRequest::History { generation: g } => {
                        if !is_live(g, &generation) {
                            continue;
                        }
                        let commits = qol_git::log(&repo, HISTORY_LIMIT).unwrap_or_default();
                        if !is_live(g, &generation) {
                            continue;
                        }
                        let _ = results.send(GitResult::History {
                            generation: g,
                            commits,
                        });
                    }
                }
            }
        })
        .expect("spawn diff-viewer git facts thread")
}

fn is_live(generation: u64, current: &AtomicU64) -> bool {
    generation == current.load(Ordering::SeqCst)
}

fn refresh_facts(repo: &Path) -> Result<Facts, String> {
    let status = qol_git::status_porcelain(repo).map_err(|error| error.to_string())?;
    let numstat = qol_git::diff_numstat(repo, DEFAULT_RANGE).map_err(|error| error.to_string())?;
    Ok(Facts { status, numstat })
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
        let _ = requests.send(GitRequest::Refresh { generation: 4 });
        assert!(
            results.recv_timeout(Duration::from_millis(200)).is_err(),
            "a stale generation must never reach the git CLI"
        );
    }
}
