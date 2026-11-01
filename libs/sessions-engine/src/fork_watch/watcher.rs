use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_agent_homes::{Harness, Registry};

use super::platform;

pub(super) const WATCHER_SCRIPT: &str = "pr-watch.cjs";
pub const WATCHER_PROGRAM: &str = "node";
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watched {
    pub script: PathBuf,
    pub url: String,
    pub flags: Vec<String>,
    root: PathBuf,
}

impl Watched {
    pub fn args(&self) -> Vec<OsString> {
        [
            self.script.clone().into_os_string(),
            OsString::from(&self.url),
        ]
        .into_iter()
        .chain(self.flags.iter().map(OsString::from))
        .collect()
    }

    pub fn trusted_path(&self, path: Option<&str>) -> Option<OsString> {
        let keeper = platform::keeper_of(&self.root)?;
        let entries = std::env::split_paths(OsStr::new(path?))
            .filter(|entry| entry.is_absolute())
            .filter_map(|entry| fs::canonicalize(entry).ok())
            .filter(|entry| entry.is_dir() && platform::is_trusted_tool(entry, &keeper))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return None;
        }
        std::env::join_paths(entries).ok()
    }

    pub fn tool(&self, name: &str, path: Option<&OsStr>) -> Result<PathBuf> {
        let keeper = platform::keeper_of(&self.root)
            .with_context(|| format!("cannot read the owner of {}", self.root.display()))?;
        let binary = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        path.into_iter()
            .flat_map(std::env::split_paths)
            .filter(|dir| dir.is_absolute())
            .filter_map(|dir| fs::canonicalize(dir.join(&binary)).ok())
            .find(|tool| {
                tool.is_file()
                    && platform::is_trusted_tool(tool, &keeper)
                    && tool
                        .parent()
                        .is_some_and(|dir| platform::is_trusted_tool(dir, &keeper))
            })
            .with_context(|| format!("no {name} owned by you or the system is on the trusted PATH"))
    }
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

fn is_version(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
}

fn under(root: &Path, folders: &[&str]) -> PathBuf {
    folders
        .iter()
        .fold(root.to_path_buf(), |path, folder| path.join(folder))
}

fn is_installed_under(script: &Path, root: &Path) -> bool {
    let cached = script
        .strip_prefix(under(root, &CACHED_WATCHER))
        .is_ok_and(|rest| {
            let parts = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            matches!(parts.as_slice(), [version, bin, name]
                if is_version(version) && bin == WATCHER_BIN && name == WATCHER_SCRIPT)
        });
    cached || script == under(root, &CLONED_WATCHER).join(WATCHER_SCRIPT)
}

fn is_kept_tree(script: &Path, root: &Path) -> bool {
    let Some(keeper) = platform::keeper_of(root) else {
        return false;
    };
    script
        .ancestors()
        .take_while(|entry| entry.starts_with(root))
        .all(|entry| platform::is_users(entry, &keeper))
}

fn claude_roots() -> Vec<PathBuf> {
    let mut roots = Registry::load()
        .every_home()
        .into_iter()
        .filter(|home| home.harness == Harness::Claude)
        .filter_map(|home| fs::canonicalize(home.path).ok())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

pub fn check_watcher(command: &[String]) -> Result<Watched> {
    check_watcher_in(command, &claude_roots())
}

pub fn check_watcher_in(command: &[String], roots: &[PathBuf]) -> Result<Watched> {
    let [program, script, url, flags @ ..] = command else {
        bail!("a fork watch runs only `node <Claude config folder>/plugins/cache/qol-skills/qol-workflow/<version>/bin/{WATCHER_SCRIPT} <pull request url> [--pretty] [--until-ready]`");
    };
    if program.as_str() != WATCHER_PROGRAM {
        bail!("a fork watch runs only `{WATCHER_PROGRAM}`, not `{program}`");
    }
    if !Path::new(script).is_absolute() {
        bail!("a fork watch needs the absolute path of {WATCHER_SCRIPT}, not {script}");
    }
    let resolved = fs::canonicalize(script)
        .with_context(|| format!("the watcher script {script} cannot be resolved"))?;
    let root = roots
        .iter()
        .find(|root| {
            resolved.is_file()
                && is_installed_under(&resolved, root)
                && is_kept_tree(&resolved, root)
        })
        .with_context(|| format!("a fork watch runs only the qol-workflow bin/{WATCHER_SCRIPT} installed in a Claude config folder's plugins cache or marketplace clone and kept by its owner, not {script}"))?;
    if !is_pull_request_url(url) {
        bail!("a fork watch needs a GitHub pull request URL, not `{url}`");
    }
    if let Some(flag) = flags
        .iter()
        .find(|flag| !WATCHER_FLAGS.contains(&flag.as_str()))
    {
        bail!("a fork watch takes only --pretty and --until-ready, not `{flag}`");
    }
    Ok(Watched {
        script: resolved,
        url: url.clone(),
        flags: flags.to_vec(),
        root: root.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/qol-tools/qol/pull/426";

    fn installed(root: &Path, folders: &[&str]) -> String {
        let bin = under(root, folders);
        fs::create_dir_all(&bin).unwrap();
        let script = bin.join(WATCHER_SCRIPT);
        fs::write(&script, "").unwrap();
        script.to_string_lossy().into_owned()
    }

    fn command(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn config() -> (tempfile::TempDir, PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(folder.path()).unwrap().join("config");
        fs::create_dir_all(&root).unwrap();
        (folder, root)
    }

    #[test]
    fn only_the_watcher_installed_in_a_claude_config_folder_is_watched() {
        let (folder, root) = config();
        let roots = [root.clone()];
        let cached = |rest: &[&'static str]| {
            CACHED_WATCHER
                .iter()
                .chain(rest)
                .copied()
                .collect::<Vec<_>>()
        };
        let cache = cached(&["0.11.27", WATCHER_BIN]);
        let script = installed(&root, &cache);
        let cloned = installed(&root, &CLONED_WATCHER);
        let nested = installed(&root.join("elsewhere"), &CLONED_WATCHER);
        let deeper = installed(&root, &cached(&["0.11.27", "extra", WATCHER_BIN]));
        let outside = installed(&root.with_file_name("other"), &cache);
        let renamed = under(&root, &cache).join("watch.cjs");
        fs::write(&renamed, "").unwrap();
        let renamed = renamed.to_string_lossy().into_owned();
        let escaped = format!(
            "{}/../../0.11.27/bin/{WATCHER_SCRIPT}",
            under(&folder.path().join("config"), &cache).display()
        );
        for accepted in [
            command(&["node", &script, URL]),
            command(&["node", &script, URL, "--pretty", "--until-ready"]),
            command(&["node", &cloned, URL, "--pretty"]),
            command(&["node", &escaped, URL]),
        ] {
            let watched = check_watcher_in(&accepted, &roots).unwrap();
            assert!(watched.script.starts_with(&root), "{accepted:?}");
            assert_eq!(watched.url, URL);
        }
        for refused in [
            command(&["sh", "-c", "true"]),
            command(&["node", &script]),
            command(&["bash", &script, URL]),
            command(&["node", &nested, URL]),
            command(&["node", &deeper, URL]),
            command(&["node", &outside, URL]),
            command(&["node", &renamed, URL]),
            command(&["node", "relative/bin/pr-watch.cjs", URL]),
            command(&["node", &script, "https://example.com/o/r/pull/1"]),
            command(&["node", &script, "https://github.com/o/r/pull/1/../2"]),
            command(&["node", &script, "https://github.com/o/r/issues/1"]),
            command(&["node", &script, URL, "--eval"]),
            command(&["node", &script, URL, "; rm -rf /"]),
        ] {
            assert!(check_watcher_in(&refused, &roots).is_err(), "{refused:?}");
        }
        assert!(check_watcher_in(&command(&["node", &script, URL]), &[]).is_err());
    }

    #[test]
    fn node_and_the_path_come_only_from_absolute_trusted_folders() {
        let (_folder, root) = config();
        let script = installed(&root, &CLONED_WATCHER);
        let watched = check_watcher_in(
            &command(&["node", &script, URL]),
            std::slice::from_ref(&root),
        )
        .unwrap();
        let bin = root.join("tools");
        fs::create_dir_all(&bin).unwrap();
        let node = bin.join(format!("{WATCHER_PROGRAM}{}", std::env::consts::EXE_SUFFIX));
        fs::write(&node, "").unwrap();
        let path = std::env::join_paths([PathBuf::from("relative"), bin.clone()]).unwrap();
        let path = path.to_str().unwrap();
        let trusted = watched.trusted_path(Some(path)).unwrap();
        assert_eq!(std::env::split_paths(&trusted).collect::<Vec<_>>(), [bin]);
        assert_eq!(
            watched
                .tool(WATCHER_PROGRAM, Some(trusted.as_os_str()))
                .unwrap(),
            node
        );
        assert!(watched.tool(WATCHER_PROGRAM, None).is_err());
        assert!(watched.tool("gh", Some(trusted.as_os_str())).is_err());
        assert!(watched.trusted_path(Some("relative")).is_none());
        assert_eq!(
            watched.args(),
            [watched.script.clone().into_os_string(), OsString::from(URL)]
        );
    }
}
