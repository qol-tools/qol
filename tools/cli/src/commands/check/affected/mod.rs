use anyhow::{Context, Result};
use serde::{de, Deserialize, Deserializer};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

mod platform;

pub(super) const WORKTREE_HEAD: &str = "WORKTREE";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Platform {
    Linux,
    Macos,
}

impl Platform {
    const VERIFIED: [Self; 2] = [Self::Linux, Self::Macos];

    pub(super) fn current() -> Result<Self> {
        let current = platform::current()?;
        debug_assert!(Self::VERIFIED.contains(&current));
        Ok(current)
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
        }
    }
}

pub(super) struct CargoPlan {
    pub(super) clippy_args: Vec<OsString>,
    pub(super) test_args: Vec<OsString>,
    pub(super) doctest: bool,
    pub(super) skip: bool,
}

#[derive(Deserialize)]
struct AffectedPlan {
    ubuntu_clippy: String,
    ubuntu_test: String,
    #[serde(
        default = "default_doctest",
        deserialize_with = "deserialize_plan_bool"
    )]
    ubuntu_doctest: bool,
    #[serde(deserialize_with = "deserialize_plan_bool")]
    ubuntu_skip: bool,
    macos_clippy: String,
    macos_test: String,
    #[serde(
        default = "default_doctest",
        deserialize_with = "deserialize_plan_bool"
    )]
    macos_doctest: bool,
    #[serde(deserialize_with = "deserialize_plan_bool")]
    macos_skip: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PlanBool {
    Bool(bool),
    Text(String),
}

fn default_doctest() -> bool {
    true
}

fn deserialize_plan_bool<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    match PlanBool::deserialize(deserializer)? {
        PlanBool::Bool(value) => Ok(value),
        PlanBool::Text(value) if value == "true" => Ok(true),
        PlanBool::Text(value) if value == "false" => Ok(false),
        PlanBool::Text(value) => Err(de::Error::custom(format!("invalid boolean {value:?}"))),
    }
}

impl AffectedPlan {
    fn cargo_plan(self, platform: Platform) -> CargoPlan {
        let (clippy, test, doctest, skip) = match platform {
            Platform::Linux => (
                self.ubuntu_clippy,
                self.ubuntu_test,
                self.ubuntu_doctest,
                self.ubuntu_skip,
            ),
            Platform::Macos => (
                self.macos_clippy,
                self.macos_test,
                self.macos_doctest,
                self.macos_skip,
            ),
        };
        CargoPlan {
            clippy_args: split_args(&clippy),
            test_args: split_args(&test),
            doctest,
            skip,
        }
    }
}

pub(super) fn load_plan(path: &Path, platform: Platform) -> Result<CargoPlan> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let plan: AffectedPlan = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(plan.cargo_plan(platform))
}

pub(super) fn planner_command(
    root: &Path,
    base_sha: Option<&str>,
    head: &str,
    output: &Path,
) -> Command {
    let mut command = Command::new("python3");
    command
        .current_dir(root)
        .arg(".github/scripts/affected_crates.py")
        .env("BASE_SHA", base_sha.unwrap_or_default())
        .env("HEAD_SHA", head)
        .env("QOL_AFFECTED_OUTPUT", output);
    command
}

pub(super) fn comparison_base(root: &Path, head: &str) -> Option<String> {
    let parent = format!("{head}^");
    git_stdout(root, &["merge-base", "origin/main", head])
        .or_else(|| git_stdout(root, &["rev-parse", &parent]))
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    super::snapshot::sanitize_git_environment(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

fn split_args(args: &str) -> Vec<OsString> {
    args.split_ascii_whitespace().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_args_preserve_planner_order() {
        assert_eq!(
            split_args("-p qol -p qol-config --all-targets"),
            ["-p", "qol", "-p", "qol-config", "--all-targets"].map(OsString::from)
        );
    }

    #[test]
    fn planner_uses_the_requested_tree() {
        let command = planner_command(
            Path::new("/repo"),
            Some("base"),
            "staged-commit",
            Path::new("/report/affected.json"),
        );
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            environment.get(std::ffi::OsStr::new("BASE_SHA")),
            Some(&std::ffi::OsStr::new("base"))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("HEAD_SHA")),
            Some(&std::ffi::OsStr::new("staged-commit"))
        );
    }

    #[test]
    fn platform_selects_its_own_plan() {
        let plan = || AffectedPlan {
            ubuntu_clippy: "-p linux --all-targets".to_string(),
            ubuntu_test: "-p linux".to_string(),
            ubuntu_doctest: false,
            ubuntu_skip: false,
            macos_clippy: "".to_string(),
            macos_test: "".to_string(),
            macos_doctest: true,
            macos_skip: true,
        };

        let linux = plan().cargo_plan(Platform::Linux);
        assert_eq!(linux.clippy_args, ["-p", "linux", "--all-targets"]);
        assert!(!linux.skip);
        assert!(!linux.doctest);
        let macos = plan().cargo_plan(Platform::Macos);
        assert!(macos.clippy_args.is_empty());
        assert!(macos.test_args.is_empty());
        assert!(macos.skip);
        assert!(macos.doctest);
    }

    #[test]
    fn plan_reader_accepts_the_previous_string_boolean_schema() {
        let plan: AffectedPlan = serde_json::from_str(
            r#"{
                "ubuntu_clippy": "",
                "ubuntu_test": "",
                "ubuntu_skip": "false",
                "macos_clippy": "",
                "macos_test": "",
                "macos_skip": "true"
            }"#,
        )
        .unwrap();

        assert!(!plan.ubuntu_skip);
        assert!(plan.ubuntu_doctest);
        assert!(plan.macos_skip);
    }

    #[test]
    fn comparison_base_is_bound_to_the_captured_head() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "--quiet"]);
        git(repository.path(), &["config", "user.name", "Test User"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        let first = commit(repository.path(), "first");
        let captured = commit(repository.path(), "captured");
        commit(repository.path(), "later");

        assert_eq!(comparison_base(repository.path(), &captured), Some(first));
    }

    fn commit(root: &Path, content: &str) -> String {
        fs::write(root.join("tracked.txt"), content).unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "--quiet", "-m", content]);
        git_stdout(root, &["rev-parse", "HEAD"]).unwrap()
    }

    fn git(root: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
}
