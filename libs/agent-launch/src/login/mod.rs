mod platform;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

pub(crate) const LOGIN_ENVS: [&str; 3] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];
const LOGIN_DIR: &str = "lane-logins";
static STAGED_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn remember_caller() {
    let config_dir = std::env::var_os(crate::account::CONFIG_DIR_ENV).map(PathBuf::from);
    let Some(path) = file_for(config_dir.as_deref()) else {
        return;
    };
    let login: Vec<(&str, String)> = LOGIN_ENVS
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (*name, value))
        })
        .collect();
    if let Err(error) = store(&path, &login) {
        log::warn!(
            "could not record this session's login for its lanes in {}: {error:#}",
            path.display()
        );
    }
}

pub(crate) fn file_for(config_dir: Option<&Path>) -> Option<PathBuf> {
    qol_config::data_subdir(LOGIN_DIR).map(|dir| dir.join(file_name(config_dir)))
}

pub(crate) fn read(path: &Path) -> Result<Vec<(String, String)>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(parse(&contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => {
            Err(error).with_context(|| format!("cannot read the lane login {}", path.display()))
        }
    }
}

fn file_name(config_dir: Option<&Path>) -> String {
    let normalized: PathBuf = config_dir
        .map(|dir| dir.components().collect())
        .unwrap_or_default();
    if normalized.as_os_str().is_empty() {
        return "default".to_owned();
    }
    let mut name = "dir-".to_owned();
    for byte in normalized.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' {
            name.push(char::from(byte));
        } else {
            name.push_str(&format!("_{byte:02x}"));
        }
    }
    name
}

fn parse(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(name, value)| LOGIN_ENVS.contains(name) && !value.is_empty())
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

fn store(path: &Path, login: &[(&str, String)]) -> Result<()> {
    if login.is_empty() {
        return match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent folder", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let contents: String = login
        .iter()
        .map(|(name, value)| format!("{name}={value}\n"))
        .collect();
    let staged = dir.join(format!(
        ".login-{}-{}",
        std::process::id(),
        STAGED_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let written = platform::write_private(&staged, contents.as_bytes())
        .and_then(|()| std::fs::rename(&staged, path).map_err(Into::into));
    if written.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_dir_names_one_flat_file_whatever_its_trailing_separator() {
        assert_eq!(
            file_name(Some(Path::new("/home/me/.claude-work/"))),
            file_name(Some(Path::new("/home/me/.claude-work")))
        );
        let name = file_name(Some(Path::new("/home/me/.claude-work")));
        assert!(
            name.starts_with("dir-") && name.ends_with("_2eclaude-work"),
            "{name}"
        );
        assert!(!name.contains(['/', '\\']), "{name}");
        for blank in [None, Some(Path::new(""))] {
            assert_eq!(file_name(blank), "default");
        }
    }

    #[test]
    fn config_dirs_that_differ_only_in_punctuation_keep_separate_logins() {
        let names = [
            "/home/me/a_b",
            "/home/me/a.b",
            "/home/me/a/b",
            "default",
            ".",
            "..",
        ]
        .map(|dir| file_name(Some(Path::new(dir))));
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        assert!(
            names.iter().all(|name| name.starts_with("dir-")),
            "{names:?}"
        );
    }

    #[test]
    fn only_login_variables_with_a_value_are_read_back() {
        let parsed =
            parse("CLAUDE_CODE_OAUTH_TOKEN=abc=def\nPATH=/bin\nANTHROPIC_API_KEY=\nnoise\n");
        assert_eq!(
            parsed,
            [("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), "abc=def".to_owned())]
        );
    }

    #[test]
    fn a_login_is_replaced_by_the_next_caller_and_removed_when_the_caller_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logins").join("default");

        store(&path, &[("CLAUDE_CODE_OAUTH_TOKEN", "old".to_owned())]).unwrap();
        store(&path, &[("ANTHROPIC_API_KEY", "new".to_owned())]).unwrap();
        assert_eq!(
            read(&path).unwrap(),
            [("ANTHROPIC_API_KEY".to_owned(), "new".to_owned())]
        );

        store(&path, &[]).unwrap();
        assert!(!path.exists());
        store(&path, &[]).unwrap();
        assert!(read(&path).unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            0
        );
    }
}
