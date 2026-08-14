mod numstat;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use numstat::parse_numstat_line;

#[derive(Debug)]
pub enum Error {
    Spawn(std::io::Error),
    Exit { code: Option<i32>, stderr: String },
    Parse { line: String, detail: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Spawn(err) => write!(f, "failed to spawn git: {err}"),
            Error::Exit { code, stderr } => {
                write!(f, "git exited with {code:?}: {stderr}")
            }
            Error::Parse { line, detail } => {
                write!(f, "failed to parse git output line {line:?}: {detail}")
            }
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub staged: char,
    pub unstaged: char,
    pub path: PathBuf,
    pub rename_target: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumstatEntry {
    pub added: Option<u64>,
    pub deleted: Option<u64>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub sha: String,
    pub author: String,
    pub subject: String,
    pub authored_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogStatEntry {
    pub sha: String,
    pub added: u64,
    pub deleted: u64,
}

impl LogStatEntry {
    pub fn magnitude(&self) -> u64 {
        self.added + self.deleted
    }
}

fn run_git(repo: &Path, args: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .arg("--no-pager")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(Error::Spawn)?;
    if !output.status.success() {
        return Err(Error::Exit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn head_sha(repo: impl AsRef<Path>) -> Result<String, Error> {
    let out = run_git(repo.as_ref(), &["rev-parse", "HEAD"])?;
    Ok(out.trim().to_string())
}

pub fn branch(repo: impl AsRef<Path>) -> Result<Option<String>, Error> {
    let out = run_git(repo.as_ref(), &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let name = out.trim();
    if name.is_empty() || name == "HEAD" {
        Ok(None)
    } else {
        Ok(Some(name.to_string()))
    }
}

pub fn status_porcelain(repo: impl AsRef<Path>) -> Result<Vec<StatusEntry>, Error> {
    let out = run_git(repo.as_ref(), &["status", "--porcelain"])?;
    out.lines()
        .filter(|line| !line.is_empty())
        .map(parse_status_line)
        .collect()
}

pub fn tracked_files(repo: impl AsRef<Path>) -> Result<Vec<String>, Error> {
    let out = run_git(repo.as_ref(), &["ls-files"])?;
    Ok(out
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

pub fn diff_numstat(repo: impl AsRef<Path>, range: &str) -> Result<Vec<NumstatEntry>, Error> {
    let out = run_git(repo.as_ref(), &["diff", "--numstat", "--no-renames", range])?;
    let mut entries = Vec::new();
    for line in out.lines().filter(|line| !line.is_empty()) {
        match parse_numstat_line(line) {
            Ok(entry) => entries.push(entry),
            Err(error) => eprintln!("qol-git: skipping unparsable numstat line: {error}"),
        }
    }
    Ok(entries)
}

pub fn diff_patch(repo: impl AsRef<Path>, range: &str, paths: &[&str]) -> Result<String, Error> {
    let mut args = vec!["diff", "--no-color", range];
    if !paths.is_empty() {
        args.push("--");
        args.extend_from_slice(paths);
    }
    run_git(repo.as_ref(), &args)
}

pub fn log(repo: impl AsRef<Path>, n: usize) -> Result<Vec<LogEntry>, Error> {
    let format = "%H%x09%an%x09%at%x09%s";
    let out = run_git(
        repo.as_ref(),
        &["log", "-n", &n.to_string(), &format!("--format={format}")],
    )?;
    out.lines()
        .filter(|line| !line.is_empty())
        .map(parse_log_line)
        .collect()
}

pub fn log_with_stats(
    repo: impl AsRef<Path>,
    n: usize,
) -> Result<(Vec<LogEntry>, Vec<LogStatEntry>), Error> {
    let format = "%H%x09%an%x09%at%x09%s";
    let out = run_git(
        repo.as_ref(),
        &[
            "log",
            "-n",
            &n.to_string(),
            "--numstat",
            &format!("--format={format}"),
        ],
    )?;
    let mut entries = Vec::new();
    let mut stats = Vec::new();
    for line in out.lines().filter(|line| !line.is_empty()) {
        if is_full_sha(line) {
            entries.push(parse_log_line(line)?);
            let sha = entries.last().expect("just pushed").sha.clone();
            stats.push(LogStatEntry {
                sha,
                added: 0,
                deleted: 0,
            });
            continue;
        }
        let Some(current) = stats.last_mut() else {
            continue;
        };
        match parse_numstat_line(line) {
            Ok(entry) => {
                current.added += entry.added.unwrap_or(0);
                current.deleted += entry.deleted.unwrap_or(0);
            }
            Err(error) => eprintln!("qol-git: skipping unparsable numstat line: {error}"),
        }
    }
    Ok((entries, stats))
}

fn is_full_sha(line: &str) -> bool {
    line.len() >= 40 && line.as_bytes()[..40].iter().all(u8::is_ascii_hexdigit)
}

fn parse_status_line(line: &str) -> Result<StatusEntry, Error> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut chars = line.chars();
    let staged = chars
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing index status"))?;
    let unstaged = chars
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing worktree status"))?;
    if chars.next() != Some(' ') {
        return Err(numstat::parse_error(
            line,
            "missing separator after status pair",
        ));
    }
    let (path, rename_target) =
        parse_status_paths(&line[3..]).map_err(|detail| numstat::parse_error(line, detail))?;
    Ok(StatusEntry {
        staged,
        unstaged,
        path,
        rename_target,
    })
}

fn parse_status_paths(field: &str) -> Result<(PathBuf, Option<PathBuf>), &'static str> {
    let (first, rest) = if field.starts_with('"') {
        numstat::unquote_prefix(field)
    } else {
        match field.find(" -> ") {
            Some(index) => (field[..index].to_string(), &field[index..]),
            None => (field.to_string(), ""),
        }
    };
    let rename_target = if rest.is_empty() {
        None
    } else if let Some(target) = rest.strip_prefix(" -> ") {
        let target = if target.starts_with('"') {
            numstat::unquote_prefix(target).0
        } else {
            target.to_string()
        };
        Some(target)
    } else {
        return Err("unexpected content after path");
    };
    Ok((first.into(), rename_target.map(PathBuf::from)))
}

fn parse_log_line(line: &str) -> Result<LogEntry, Error> {
    let mut fields = line.splitn(4, '\t');
    let sha = fields
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing sha"))?;
    let author = fields
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing author"))?;
    let authored_at = fields
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing timestamp"))?;
    let subject = fields
        .next()
        .ok_or_else(|| numstat::parse_error(line, "missing subject"))?;
    let authored_at = authored_at
        .parse::<i64>()
        .map_err(|_| numstat::parse_error(line, "timestamp is not a number"))?;
    Ok(LogEntry {
        sha: sha.to_string(),
        author: author.to_string(),
        subject: subject.to_string(),
        authored_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qol-git-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        dir
    }

    fn git(repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn commit(repo: &Path, message: &str) {
        git(
            repo,
            &[
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    }

    fn stage_all(repo: &Path) {
        git(repo, &["add", "-A"]);
    }

    fn write(repo: &Path, rel: &str, contents: &[u8]) {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn head_sha_on_fresh_commit() {
        let repo = repo("head-sha");
        write(&repo, "a.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        let sha = head_sha(&repo).expect("head sha");
        let expected = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(sha, expected);
        assert_eq!(sha.len(), 40);
    }

    #[test]
    fn branch_name_and_detached_head() {
        let repo = repo("branch");
        write(&repo, "a.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        assert_eq!(branch(&repo).expect("branch"), Some("main".to_string()));
        git(&repo, &["checkout", "-q", "--detach", "HEAD"]);
        assert_eq!(branch(&repo).expect("branch"), None);
    }

    #[test]
    fn dirty_status_reports_staged_and_unstaged() {
        let repo = repo("dirty");
        write(&repo, "a.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "a.txt", b"two");
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(
            entries,
            vec![StatusEntry {
                staged: ' ',
                unstaged: 'M',
                path: PathBuf::from("a.txt"),
                rename_target: None,
            }]
        );
        stage_all(&repo);
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(entries[0].staged, 'M');
        assert_eq!(entries[0].unstaged, ' ');
    }

    #[test]
    fn untracked_file_status() {
        let repo = repo("untracked");
        write(&repo, "a.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "new.txt", b"two");
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(
            entries,
            vec![StatusEntry {
                staged: '?',
                unstaged: '?',
                path: PathBuf::from("new.txt"),
                rename_target: None,
            }]
        );
    }

    #[test]
    fn staged_rename_reports_target() {
        let repo = repo("rename");
        write(&repo, "old.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        git(&repo, &["mv", "old.txt", "new.txt"]);
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].staged, 'R');
        assert_eq!(entries[0].unstaged, ' ');
        assert_eq!(entries[0].path, PathBuf::from("old.txt"));
        assert_eq!(entries[0].rename_target, Some(PathBuf::from("new.txt")));
    }

    #[test]
    fn binary_file_in_numstat() {
        let repo = repo("binary");
        write(&repo, "a.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "blob.bin", &[0u8, 1, 2, 3, 0xff]);
        stage_all(&repo);
        commit(&repo, "add binary");
        write(&repo, "blob.bin", &[9u8, 9, 9, 0, 0, 0]);
        let entries = diff_numstat(&repo, "HEAD").expect("numstat");
        assert_eq!(
            entries,
            vec![NumstatEntry {
                added: None,
                deleted: None,
                path: "blob.bin".to_string(),
            }]
        );
    }

    #[test]
    fn quoted_path_with_spaces() {
        let repo = repo("quoted");
        write(&repo, "plain.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "na\u{ef}ve file.txt", b"two");
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("na\u{ef}ve file.txt"));
    }

    #[test]
    fn quoted_path_in_numstat() {
        let repo = repo("quoted-numstat");
        write(&repo, "plain.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "na\u{ef}ve.txt", b"one");
        stage_all(&repo);
        commit(&repo, "add quoted");
        write(&repo, "na\u{ef}ve.txt", b"two");
        let entries = diff_numstat(&repo, "HEAD").expect("numstat");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "na\u{ef}ve.txt");
        assert_eq!(entries[0].added, Some(1));
    }

    #[test]
    fn quoted_rename_with_separator_in_path() {
        let repo = repo("quoted-rename");
        write(&repo, "a -> b.txt", b"one");
        stage_all(&repo);
        commit(&repo, "first");
        git(&repo, &["mv", "a -> b.txt", "c -> d.txt"]);
        let entries = status_porcelain(&repo).expect("status");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].staged, 'R');
        assert_eq!(entries[0].path, PathBuf::from("a -> b.txt"));
        assert_eq!(entries[0].rename_target, Some(PathBuf::from("c -> d.txt")));
    }

    #[test]
    fn diff_patch_returns_raw_unified_diff() {
        let repo = repo("patch");
        write(&repo, "a.txt", b"one\ntwo\n");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "a.txt", b"one\nthree\n");
        let patch = diff_patch(&repo, "HEAD", &[]).expect("patch");
        assert!(patch.starts_with("diff --git a/a.txt b/a.txt"));
        assert!(patch.contains("+three"));
        assert!(patch.contains("-two"));
    }

    #[test]
    fn diff_patch_scopes_to_paths_with_spaces() {
        let repo = repo("patch-scope");
        write(&repo, "keep.txt", b"keep\n");
        write(&repo, "with space.txt", b"one\n");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "keep.txt", b"changed\n");
        write(&repo, "with space.txt", b"two\n");
        let patch = diff_patch(&repo, "HEAD", &["with space.txt"]).expect("patch");
        assert!(patch.contains("diff --git a/with space.txt b/with space.txt"));
        assert!(!patch.contains("keep.txt"));
    }

    #[test]
    fn log_returns_newest_first_and_respects_limit() {
        let repo = repo("log");
        for (i, message) in ["first", "second", "third"].iter().enumerate() {
            write(&repo, "a.txt", format!("line {i}\n").as_bytes());
            stage_all(&repo);
            commit(&repo, message);
        }
        let entries = log(&repo, 2).expect("log");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].subject, "third");
        assert_eq!(entries[1].subject, "second");
        assert_eq!(entries[0].sha, git(&repo, &["rev-parse", "HEAD"]).trim());
        assert_eq!(entries[1].sha, git(&repo, &["rev-parse", "HEAD~1"]).trim());
        assert_eq!(entries[0].author, "Test");
        let timestamp = git(&repo, &["log", "-1", "--format=%at"]);
        assert_eq!(entries[0].authored_at.to_string(), timestamp.trim());
        let all = log(&repo, 10).expect("log");
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].subject, "first");
    }

    #[test]
    fn log_with_stats_aligns_entries_and_totals() {
        let repo = repo("log-stats");
        write(&repo, "a.txt", b"1\n2\n");
        stage_all(&repo);
        commit(&repo, "first");
        write(&repo, "a.txt", b"1\n2\n3\n");
        write(&repo, "blob.bin", &[0u8, 1, 2]);
        stage_all(&repo);
        commit(&repo, "second");
        write(&repo, "a.txt", b"1\n2\n3\n4\n5\n");
        write(&repo, "c.txt", b"x\n");
        stage_all(&repo);
        commit(&repo, "third");
        let (entries, stats) = log_with_stats(&repo, 10).expect("log stats");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].subject, "third");
        assert_eq!(stats.len(), 3);
        for (entry, stat) in entries.iter().zip(&stats) {
            assert_eq!(stat.sha, entry.sha, "stats align with log entries");
        }
        assert_eq!(
            stats[0].magnitude(),
            3,
            "third adds two lines to a.txt and one to c.txt"
        );
        assert_eq!(
            stats[1].magnitude(),
            1,
            "second adds one text line; binary changes count zero"
        );
        assert_eq!(
            stats[2].magnitude(),
            2,
            "the root commit counts its full tree"
        );
        let limited = log_with_stats(&repo, 2).expect("limited log stats");
        assert_eq!(limited.1.len(), 2);
        assert_eq!(limited.1[0].sha, stats[0].sha, "limit applies to stats too");
    }

    #[test]
    fn log_with_stats_counts_merge_commits_as_zero() {
        let repo = repo("log-stats-merge");
        write(&repo, "a.txt", b"1\n");
        stage_all(&repo);
        commit(&repo, "one");
        git(&repo, &["checkout", "-q", "-b", "side"]);
        write(&repo, "b.txt", b"2\n");
        stage_all(&repo);
        commit(&repo, "side");
        git(&repo, &["checkout", "-q", "main"]);
        write(&repo, "c.txt", b"3\n");
        stage_all(&repo);
        commit(&repo, "main");
        git(&repo, &["merge", "-q", "--no-ff", "side", "-m", "merge"]);
        let (entries, stats) = log_with_stats(&repo, 10).expect("log stats");
        assert_eq!(entries[0].subject, "merge");
        assert_eq!(entries.len(), 4);
        assert_eq!(
            stats[0].magnitude(),
            0,
            "merge diffs are not shown without --cc"
        );
        assert_eq!(stats[1].magnitude(), 1, "main adds one line");
        assert_eq!(stats[2].magnitude(), 1, "side adds one line");
        assert_eq!(stats[3].magnitude(), 1, "root commit counts its tree");
    }

    #[test]
    fn tracked_files_lists_committed_paths_only() {
        let repo = repo("tracked-files");
        write(&repo, "a.txt", b"1\n");
        write(&repo, "sub/b.txt", b"2\n");
        stage_all(&repo);
        commit(&repo, "one");
        write(&repo, "c.txt", b"3\n");
        let files = tracked_files(&repo).expect("ls-files");
        assert_eq!(files, vec!["a.txt", "sub/b.txt"]);
        stage_all(&repo);
        commit(&repo, "two");
        let files = tracked_files(&repo).expect("ls-files");
        assert_eq!(files, vec!["a.txt", "c.txt", "sub/b.txt"]);
    }
}
