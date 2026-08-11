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
}
