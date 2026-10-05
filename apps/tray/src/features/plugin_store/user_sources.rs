use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const USER_SOURCES_FILE: &str = "plugin-sources.json";
const INVALID_REPO: &str = "Type a GitHub repository as owner/repo";
const DUPLICATE_REPO: &str = "That source is already added";
const BUILTIN_REPO: &str = "The default source cannot be removed";
const UNKNOWN_REPO: &str = "Unknown source";
const SAVE_FAILED: &str = "The sources could not be saved";

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct UserSourcesFile {
    #[serde(default)]
    repos: Vec<String>,
}

pub(crate) fn load() -> Vec<String> {
    match path().and_then(|path| crate::file_io::load_json_or_default::<UserSourcesFile>(&path)) {
        Ok(file) => file.repos,
        Err(error) => {
            log::warn!("Failed to read plugin sources: {:#}", error);
            Vec::new()
        }
    }
}

pub(crate) fn add(repo: &str, builtin: &[String]) -> Result<String, String> {
    let (repo, repos) = with_added(load(), builtin, repo)?;
    save(repos)?;
    Ok(repo)
}

pub(crate) fn remove(repo: &str, builtin: &[String]) -> Result<(), String> {
    let repos = with_removed(load(), builtin, repo)?;
    save(repos)
}

fn save(repos: Vec<String>) -> Result<(), String> {
    path()
        .and_then(|path| crate::file_io::write_pretty_json(&path, &UserSourcesFile { repos }))
        .map_err(|error| {
            log::error!("Failed to save plugin sources: {:#}", error);
            SAVE_FAILED.to_string()
        })
}

fn path() -> Result<PathBuf> {
    Ok(crate::paths::shared_config_dir()?.join(USER_SOURCES_FILE))
}

fn with_added(
    mut repos: Vec<String>,
    builtin: &[String],
    repo: &str,
) -> Result<(String, Vec<String>), String> {
    let repo = validate_repo(repo)?.to_string();
    if builtin
        .iter()
        .chain(repos.iter())
        .any(|known| known.eq_ignore_ascii_case(&repo))
    {
        return Err(DUPLICATE_REPO.to_string());
    }
    repos.push(repo.clone());
    Ok((repo, repos))
}

fn with_removed(
    mut repos: Vec<String>,
    builtin: &[String],
    repo: &str,
) -> Result<Vec<String>, String> {
    let repo = repo.trim();
    if builtin.iter().any(|known| known.eq_ignore_ascii_case(repo)) {
        return Err(BUILTIN_REPO.to_string());
    }
    let before = repos.len();
    repos.retain(|known| !known.eq_ignore_ascii_case(repo));
    if repos.len() == before {
        return Err(UNKNOWN_REPO.to_string());
    }
    Ok(repos)
}

fn validate_repo(repo: &str) -> Result<&str, String> {
    let repo = repo.trim();
    let valid = repo
        .split_once('/')
        .is_some_and(|(owner, name)| is_github_owner(owner) && is_github_repo_name(name));
    if valid {
        Ok(repo)
    } else {
        Err(INVALID_REPO.to_string())
    }
}

fn is_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && owner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn is_github_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin() -> Vec<String> {
        vec!["qol-tools/qol".to_string()]
    }

    #[test]
    fn repo_validation_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("some-org/some-repo", Some("some-org/some-repo")),
            ("  me/my.plugins_2  ", Some("me/my.plugins_2")),
            ("Owner9/Repo", Some("Owner9/Repo")),
            ("", None),
            ("owner", None),
            ("owner/", None),
            ("/repo", None),
            ("a/b/c", None),
            ("-owner/repo", None),
            ("own_er/repo", None),
            ("owner/..", None),
            ("owner/.", None),
            ("owner/re po", None),
            ("https://github.com/owner/repo", None),
        ];
        for (input, expected) in cases {
            assert_eq!(validate_repo(input).ok(), *expected, "input: {input:?}");
        }
    }

    #[test]
    fn adding_refuses_duplicates_of_builtin_and_user_sources_in_any_case() {
        let repos = vec!["me/plugins".to_string()];
        for duplicate in [
            "qol-tools/qol",
            "QOL-Tools/QOL",
            "me/plugins",
            " Me/Plugins ",
        ] {
            assert_eq!(
                with_added(repos.clone(), &builtin(), duplicate),
                Err(DUPLICATE_REPO.to_string()),
                "repo: {duplicate}"
            );
        }
        assert_eq!(
            with_added(repos.clone(), &builtin(), "bad"),
            Err(INVALID_REPO.to_string())
        );
        assert_eq!(
            with_added(repos, &builtin(), " you/more "),
            Ok((
                "you/more".to_string(),
                vec!["me/plugins".to_string(), "you/more".to_string()]
            ))
        );
    }

    #[test]
    fn removing_refuses_the_builtin_and_unknown_sources() {
        let repos = vec!["me/plugins".to_string(), "you/more".to_string()];
        assert_eq!(
            with_removed(repos.clone(), &builtin(), "qol-tools/qol"),
            Err(BUILTIN_REPO.to_string())
        );
        assert_eq!(
            with_removed(repos.clone(), &builtin(), "nobody/here"),
            Err(UNKNOWN_REPO.to_string())
        );
        assert_eq!(
            with_removed(repos, &builtin(), "ME/plugins"),
            Ok(vec!["you/more".to_string()])
        );
    }

    #[test]
    fn user_sources_round_trip_through_the_config_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _paths = crate::paths::push_test_path_root(tmp.path());
        assert!(load().is_empty());

        assert_eq!(add("me/plugins", &builtin()), Ok("me/plugins".to_string()));
        assert_eq!(add("you/more", &builtin()), Ok("you/more".to_string()));
        assert_eq!(load(), ["me/plugins", "you/more"]);
        assert_eq!(
            add("me/plugins", &builtin()),
            Err(DUPLICATE_REPO.to_string())
        );

        assert_eq!(remove("me/plugins", &builtin()), Ok(()));
        assert_eq!(load(), ["you/more"]);
        assert!(path().unwrap().starts_with(tmp.path()));
    }
}
