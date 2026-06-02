use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_io;

const BEGIN_MARKER: &str = "# >>> qol-tools shell hook >>>";
const END_MARKER: &str = "# <<< qol-tools shell hook <<<";
const HOOK_BODY: &str = "\
[ -f \"$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh\" ] && \\
  source \"$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh\"";
const RC_FILE_NAMES: &[&str] = &[".zshrc", ".bashrc"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ShellHookStatus {
    AllPresent,
    PartialMissing(Vec<PathBuf>),
    NoneInstalled,
}

pub(crate) fn install() -> Result<()> {
    for rc in existing_rc_files()? {
        install_in_file(&rc)?;
    }
    Ok(())
}

pub(crate) fn uninstall() -> Result<()> {
    for rc in existing_rc_files()? {
        uninstall_in_file(&rc)?;
    }
    Ok(())
}

pub(crate) fn is_installed() -> Result<ShellHookStatus> {
    let existing = existing_rc_files()?;
    if existing.is_empty() {
        return Ok(ShellHookStatus::NoneInstalled);
    }
    let mut missing = Vec::new();
    for rc in &existing {
        let content = read_rc_file(rc)?;
        if !block_matches_canonical(&content) {
            missing.push(rc.clone());
        }
    }
    if missing.is_empty() {
        return Ok(ShellHookStatus::AllPresent);
    }
    if missing.len() == existing.len() {
        return Ok(ShellHookStatus::NoneInstalled);
    }
    Ok(ShellHookStatus::PartialMissing(missing))
}

pub(crate) fn any_rc_file_exists() -> Result<bool> {
    Ok(!existing_rc_files()?.is_empty())
}

fn install_in_file(path: &Path) -> Result<()> {
    let original = read_rc_file(path)?;
    let updated = upsert_block(&original);
    if updated == original {
        return Ok(());
    }
    file_io::atomic_write(path, updated.as_bytes())
}

fn uninstall_in_file(path: &Path) -> Result<()> {
    let original = read_rc_file(path)?;
    let updated = strip_block(&original);
    if updated == original {
        return Ok(());
    }
    file_io::atomic_write(path, updated.as_bytes())
}

fn existing_rc_files() -> Result<Vec<PathBuf>> {
    let home = home_dir()?;
    Ok(RC_FILE_NAMES
        .iter()
        .map(|name| home.join(name))
        .filter(|path| path.is_file())
        .collect())
}

fn home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))
}

fn read_rc_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("Failed to read rc file {}", path.display()))
}

fn canonical_block() -> String {
    format!("{}\n{}\n{}", BEGIN_MARKER, HOOK_BODY, END_MARKER)
}

fn block_matches_canonical(content: &str) -> bool {
    let Some(range) = locate_block(content) else {
        return false;
    };
    content[range.start..range.end_no_newline] == canonical_block()
}

fn upsert_block(content: &str) -> String {
    let canonical = canonical_block();
    if let Some(range) = locate_block(content) {
        if content[range.start..range.end_no_newline] == canonical {
            return content.to_string();
        }
        let mut out = String::with_capacity(content.len());
        out.push_str(&content[..range.start]);
        out.push_str(&canonical);
        out.push_str(&content[range.end_no_newline..]);
        return out;
    }
    let mut out = content.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&canonical);
    out.push('\n');
    out
}

fn strip_block(content: &str) -> String {
    let Some(range) = locate_block(content) else {
        return content.to_string();
    };
    let mut block_start = range.start;
    let mut block_end = range.end_with_newline;
    let leading_blank = block_start > 0 && content.as_bytes()[block_start - 1] == b'\n';
    let trailing_blank = block_end < content.len() && content.as_bytes()[block_end] == b'\n';
    if leading_blank {
        block_start -= 1;
    } else if trailing_blank {
        block_end += 1;
    }
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..block_start]);
    out.push_str(&content[block_end..]);
    out
}

fn locate_block(content: &str) -> Option<BlockRange> {
    let begin = find_marker_line(content, BEGIN_MARKER)?;
    let after_begin = begin + line_length_at(content, begin);
    let end_relative = find_marker_line(&content[after_begin..], END_MARKER)?;
    let end_absolute = after_begin + end_relative;
    let block_end_with_newline = end_absolute + line_length_at(content, end_absolute);
    let block_end_without_newline = trim_trailing_newline(content, block_end_with_newline);
    Some(BlockRange {
        start: begin,
        end_no_newline: block_end_without_newline,
        end_with_newline: block_end_with_newline,
    })
}

struct BlockRange {
    start: usize,
    end_no_newline: usize,
    end_with_newline: usize,
}

fn find_marker_line(content: &str, marker: &str) -> Option<usize> {
    let mut cursor = 0;
    while cursor <= content.len() {
        let slice = &content[cursor..];
        let line_end = slice
            .find('\n')
            .map(|n| cursor + n)
            .unwrap_or(content.len());
        let line = &content[cursor..line_end];
        if line.trim_end_matches('\r') == marker {
            return Some(cursor);
        }
        if line_end == content.len() {
            return None;
        }
        cursor = line_end + 1;
    }
    None
}

fn line_length_at(content: &str, start: usize) -> usize {
    let slice = &content[start..];
    match slice.find('\n') {
        Some(n) => n + 1,
        None => slice.len(),
    }
}

fn trim_trailing_newline(content: &str, end: usize) -> usize {
    if end > 0 && content.as_bytes().get(end - 1) == Some(&b'\n') {
        end - 1
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct HomeGuard {
        previous: Option<OsString>,
        _tempdir: TempDir,
    }

    impl HomeGuard {
        fn new() -> (Self, PathBuf) {
            let tempdir = TempDir::new().unwrap();
            let previous = std::env::var_os("HOME");
            std::env::set_var("HOME", tempdir.path());
            let path = tempdir.path().to_path_buf();
            (
                Self {
                    previous,
                    _tempdir: tempdir,
                },
                path,
            )
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn write_rc(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    fn read_rc(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn install_appends_canonical_block_when_missing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        write_rc(&zshrc, "export FOO=1\n");

        install().unwrap();

        let content = read_rc(&zshrc);
        let expected = format!("export FOO=1\n\n{}\n", canonical_block());
        assert_eq!(content, expected);
    }

    #[test]
    fn install_is_idempotent() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        write_rc(&zshrc, "export FOO=1\n");

        install().unwrap();
        let after_first = read_rc(&zshrc);
        install().unwrap();
        let after_second = read_rc(&zshrc);
        assert_eq!(after_first, after_second);
    }

    #[test]
    fn install_appends_block_when_rc_missing_trailing_newline() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        write_rc(&zshrc, "export FOO=1");

        install().unwrap();

        let content = read_rc(&zshrc);
        assert!(content.starts_with("export FOO=1\n"));
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains(END_MARKER));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn install_replaces_drifted_block() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let drifted = format!(
            "alias g=git\n\n{}\nsource /old/path/activate.sh\n{}\n\nalias l=ls\n",
            BEGIN_MARKER, END_MARKER
        );
        write_rc(&zshrc, &drifted);

        install().unwrap();

        let content = read_rc(&zshrc);
        let expected = format!("alias g=git\n\n{}\n\nalias l=ls\n", canonical_block());
        assert_eq!(content, expected);
    }

    #[test]
    fn install_replaces_partial_block_missing_body() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let partial = format!("alias g=git\n{}\n{}\n", BEGIN_MARKER, END_MARKER);
        write_rc(&zshrc, &partial);

        install().unwrap();

        let content = read_rc(&zshrc);
        assert!(content.contains(HOOK_BODY));
        assert!(block_matches_canonical(&content));
    }

    #[test]
    fn install_skips_missing_rc_files() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        install().unwrap();
        assert!(!home.join(".zshrc").exists());
        assert!(!home.join(".bashrc").exists());
    }

    #[test]
    fn install_writes_to_both_existing_rc_files() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let bashrc = home.join(".bashrc");
        write_rc(&zshrc, "");
        write_rc(&bashrc, "alias b=bash\n");

        install().unwrap();

        assert!(block_matches_canonical(&read_rc(&zshrc)));
        assert!(block_matches_canonical(&read_rc(&bashrc)));
    }

    #[test]
    fn uninstall_removes_only_block_preserves_neighbors() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let initial = format!("before line\n\n{}\n\nafter line\n", canonical_block());
        write_rc(&zshrc, &initial);

        uninstall().unwrap();

        let content = read_rc(&zshrc);
        assert_eq!(content, "before line\n\nafter line\n");
    }

    #[test]
    fn uninstall_when_block_at_eof() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let initial = format!("export FOO=1\n\n{}\n", canonical_block());
        write_rc(&zshrc, &initial);

        uninstall().unwrap();

        let content = read_rc(&zshrc);
        assert_eq!(content, "export FOO=1\n");
    }

    #[test]
    fn uninstall_when_block_at_start() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let initial = format!("{}\n\nexport FOO=1\n", canonical_block());
        write_rc(&zshrc, &initial);

        uninstall().unwrap();

        let content = read_rc(&zshrc);
        assert_eq!(content, "export FOO=1\n");
    }

    #[test]
    fn uninstall_no_op_when_block_absent() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        write_rc(&zshrc, "export FOO=1\n");

        uninstall().unwrap();

        assert_eq!(read_rc(&zshrc), "export FOO=1\n");
    }

    #[test]
    fn is_installed_reports_all_present() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        write_rc(&home.join(".zshrc"), &format!("{}\n", canonical_block()));
        write_rc(&home.join(".bashrc"), &format!("{}\n", canonical_block()));

        assert_eq!(is_installed().unwrap(), ShellHookStatus::AllPresent);
    }

    #[test]
    fn is_installed_reports_none_when_no_rc_files() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, _home) = HomeGuard::new();
        assert_eq!(is_installed().unwrap(), ShellHookStatus::NoneInstalled);
    }

    #[test]
    fn is_installed_reports_none_when_all_existing_lack_block() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        write_rc(&home.join(".zshrc"), "alias g=git\n");
        write_rc(&home.join(".bashrc"), "alias b=bash\n");

        assert_eq!(is_installed().unwrap(), ShellHookStatus::NoneInstalled);
    }

    #[test]
    fn is_installed_reports_partial() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        write_rc(&home.join(".zshrc"), &format!("{}\n", canonical_block()));
        write_rc(&home.join(".bashrc"), "alias b=bash\n");

        match is_installed().unwrap() {
            ShellHookStatus::PartialMissing(missing) => {
                assert_eq!(missing, vec![home.join(".bashrc")]);
            }
            other => panic!("expected PartialMissing, got {other:?}"),
        }
    }

    #[test]
    fn any_rc_file_exists_table() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        assert!(!any_rc_file_exists().unwrap());
        write_rc(&home.join(".zshrc"), "");
        assert!(any_rc_file_exists().unwrap());
    }

    #[test]
    fn install_then_uninstall_round_trips_to_original() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let (_home_guard, home) = HomeGuard::new();
        let zshrc = home.join(".zshrc");
        let original = "export FOO=1\nalias g=git\n";
        write_rc(&zshrc, original);

        install().unwrap();
        uninstall().unwrap();

        assert_eq!(read_rc(&zshrc), original);
    }

    #[test]
    fn upsert_block_preserves_content_after_block_when_replacing() {
        let drifted = format!(
            "header\n{}\nold body\n{}\nfooter\n",
            BEGIN_MARKER, END_MARKER
        );
        let result = upsert_block(&drifted);
        assert!(result.starts_with("header\n"));
        assert!(result.ends_with("footer\n"));
        assert!(result.contains(HOOK_BODY));
        assert!(!result.contains("old body"));
    }

    #[test]
    fn locate_block_returns_none_when_only_begin() {
        let only_begin = format!("foo\n{}\nbar\n", BEGIN_MARKER);
        assert!(locate_block(&only_begin).is_none());
    }

    #[test]
    fn locate_block_returns_none_when_only_end() {
        let only_end = format!("foo\n{}\nbar\n", END_MARKER);
        assert!(locate_block(&only_end).is_none());
    }
}
