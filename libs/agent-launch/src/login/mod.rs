mod platform;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub(crate) const LOGIN_ENVS: [&str; 3] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];
const LOGIN_DIR: &str = "lane-logins";

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
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the lane login {}", path.display()))?;
    Ok(parse(&contents))
}

fn file_name(config_dir: Option<&Path>) -> String {
    let Some(config_dir) = config_dir else {
        return "default".to_owned();
    };
    let normalized: PathBuf = config_dir.components().collect();
    normalized
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
    let staged = dir.join(format!(".login-{}", std::process::id()));
    platform::write_private(&staged, contents.as_bytes())?;
    std::fs::rename(&staged, path)?;
    Ok(())
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
        assert_eq!(
            file_name(Some(Path::new("/home/me/.claude-work"))),
            "_home_me_.claude-work"
        );
        assert_eq!(file_name(None), "default");
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
    }
}
