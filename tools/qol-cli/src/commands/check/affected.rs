use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(super) const WORKTREE_HEAD: &str = "WORKTREE";

#[derive(Clone, Copy)]
pub(super) enum Platform {
    Linux,
    Macos,
}

impl Platform {
    pub(super) fn current() -> Result<Self> {
        match env::consts::OS {
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Macos),
            other => bail!("qol check is not verified on {other}"),
        }
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
    pub(super) skip: bool,
}

#[derive(Deserialize)]
struct AffectedPlan {
    ubuntu_clippy: String,
    ubuntu_test: String,
    ubuntu_skip: String,
    macos_clippy: String,
    macos_test: String,
    macos_skip: String,
}

impl AffectedPlan {
    fn cargo_plan(self, platform: Platform) -> CargoPlan {
        let (clippy, test, skip) = match platform {
            Platform::Linux => (self.ubuntu_clippy, self.ubuntu_test, self.ubuntu_skip),
            Platform::Macos => (self.macos_clippy, self.macos_test, self.macos_skip),
        };
        CargoPlan {
            clippy_args: split_args(&clippy),
            test_args: split_args(&test),
            skip: skip == "true",
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

pub(super) fn planner_command(root: &Path, base_sha: Option<&str>, output: &Path) -> Command {
    let mut command = Command::new("python3");
    command
        .current_dir(root)
        .arg(".github/scripts/affected_crates.py")
        .env("BASE_SHA", base_sha.unwrap_or_default())
        .env("HEAD_SHA", WORKTREE_HEAD)
        .env("QOL_AFFECTED_OUTPUT", output);
    command
}

pub(super) fn comparison_base(root: &Path) -> Option<String> {
    git_stdout(root, ["merge-base", "origin/main", "HEAD"])
        .or_else(|| git_stdout(root, ["rev-parse", "HEAD^"]))
}

fn git_stdout<const N: usize>(root: &Path, args: [&str; N]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
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
    fn platform_selects_its_own_plan() {
        let plan = || AffectedPlan {
            ubuntu_clippy: "-p linux --all-targets".to_string(),
            ubuntu_test: "-p linux".to_string(),
            ubuntu_skip: "false".to_string(),
            macos_clippy: "".to_string(),
            macos_test: "".to_string(),
            macos_skip: "true".to_string(),
        };

        let linux = plan().cargo_plan(Platform::Linux);
        assert_eq!(linux.clippy_args, ["-p", "linux", "--all-targets"]);
        assert!(!linux.skip);
        let macos = plan().cargo_plan(Platform::Macos);
        assert!(macos.clippy_args.is_empty());
        assert!(macos.test_args.is_empty());
        assert!(macos.skip);
    }
}
