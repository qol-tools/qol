use qol_conventions::artifact::{
    BuildIntent, SourceIdentity, ENV_BUILD_INTENT, ENV_SOURCE_COMMIT, ENV_SOURCE_HEAD_TREE,
    ENV_SOURCE_WORKING_TREE,
};
use std::fmt;
use std::path::Path;
use std::process::{Command, Output};

const BUILD_SCOPE_PATHS: [&str; 7] = [
    "apps",
    "libs",
    "plugins",
    "tools",
    "vendor",
    "Cargo.toml",
    "Cargo.lock",
];

pub const TRAY_BUILD_SCOPE_PATHS: [&str; 5] =
    ["apps", "libs", "vendor", "Cargo.toml", "Cargo.lock"];

const GIT_ROUTING_ENVIRONMENT: [&str; 7] = [
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentityEnvironment {
    intent: BuildIntent,
    source: SourceIdentity,
    scope: &'static [&'static str],
}

#[derive(Debug)]
pub enum BuildIdentityEnvironmentError {
    GitUnavailable(std::io::Error),
    TemporaryIndex(std::io::Error),
    GitCommand {
        args: Vec<String>,
        stderr: String,
    },
    EnvironmentVariable {
        name: &'static str,
        source: std::env::VarError,
    },
    EnvironmentMismatch(&'static str),
    DirtyProductionTree,
    DirtySubmodule,
    SourceChanged,
    InvalidUtf8(std::string::FromUtf8Error),
    EmptyGitValue(&'static str),
}

impl fmt::Display for BuildIdentityEnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitUnavailable(error) => write!(formatter, "cannot run Git: {error}"),
            Self::TemporaryIndex(error) => {
                write!(formatter, "cannot create temporary Git index: {error}")
            }
            Self::GitCommand { args, stderr } => write!(
                formatter,
                "Git command `git {}` failed: {}",
                args.join(" "),
                stderr.trim()
            ),
            Self::EnvironmentVariable { name, source } => {
                write!(formatter, "cannot read inherited {name}: {source}")
            }
            Self::EnvironmentMismatch(name) => {
                write!(
                    formatter,
                    "inherited {name} does not match the current source"
                )
            }
            Self::DirtyProductionTree => {
                formatter.write_str("production builds require a clean Git working tree")
            }
            Self::DirtySubmodule => formatter
                .write_str("cannot identify a working tree with dirty submodule contents exactly"),
            Self::SourceChanged => formatter.write_str(
                "the build modified tracked files while it was running, so the source identity no longer matches; see `git status`",
            ),
            Self::InvalidUtf8(error) => write!(formatter, "Git returned invalid UTF-8: {error}"),
            Self::EmptyGitValue(name) => write!(formatter, "Git returned an empty {name}"),
        }
    }
}

impl std::error::Error for BuildIdentityEnvironmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GitUnavailable(error) | Self::TemporaryIndex(error) => Some(error),
            Self::EnvironmentVariable { source, .. } => Some(source),
            Self::InvalidUtf8(error) => Some(error),
            Self::GitCommand { .. }
            | Self::EnvironmentMismatch(_)
            | Self::DirtyProductionTree
            | Self::DirtySubmodule
            | Self::SourceChanged
            | Self::EmptyGitValue(_) => None,
        }
    }
}

impl BuildIdentityEnvironment {
    pub fn production(repo: &Path) -> Result<Self, BuildIdentityEnvironmentError> {
        Self::resolve(repo, BuildIntent::Production, true)
    }

    pub fn development(repo: &Path) -> Result<Self, BuildIdentityEnvironmentError> {
        Self::resolve(repo, BuildIntent::Development, false)
    }

    pub fn development_scoped(
        repo: &Path,
        scope: &'static [&'static str],
    ) -> Result<Self, BuildIdentityEnvironmentError> {
        Self::resolve_with_scope(repo, BuildIntent::Development, false, scope)
    }

    pub fn sandbox(repo: &Path) -> Result<Self, BuildIdentityEnvironmentError> {
        Self::resolve(repo, BuildIntent::Sandbox, false)
    }

    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub fn variables(&self) -> [(&'static str, String); 4] {
        let SourceIdentity::Git {
            commit,
            head_tree,
            working_tree,
        } = &self.source
        else {
            unreachable!("resolved build identity source is Git")
        };
        [
            (ENV_BUILD_INTENT, self.intent.as_str().to_string()),
            (ENV_SOURCE_COMMIT, commit.clone()),
            (ENV_SOURCE_HEAD_TREE, head_tree.clone()),
            (ENV_SOURCE_WORKING_TREE, working_tree.clone()),
        ]
    }

    pub fn apply_to(&self, command: &mut Command) {
        command.envs(self.variables());
    }

    pub fn verify_inherited_environment(&self) -> Result<(), BuildIdentityEnvironmentError> {
        for (name, expected) in self.variables() {
            let actual = std::env::var(name).map_err(|source| {
                BuildIdentityEnvironmentError::EnvironmentVariable { name, source }
            })?;
            if actual != expected {
                return Err(BuildIdentityEnvironmentError::EnvironmentMismatch(name));
            }
        }
        Ok(())
    }

    pub fn verify_unchanged(&self, repo: &Path) -> Result<(), BuildIdentityEnvironmentError> {
        if self.intent == BuildIntent::Unspecified {
            return Err(BuildIdentityEnvironmentError::SourceChanged);
        }
        let current = Self::resolve_with_scope(repo, self.intent, false, self.scope)?;
        if &current == self {
            return Ok(());
        }
        Err(BuildIdentityEnvironmentError::SourceChanged)
    }

    fn resolve(
        repo: &Path,
        intent: BuildIntent,
        require_clean: bool,
    ) -> Result<Self, BuildIdentityEnvironmentError> {
        Self::resolve_with_scope(repo, intent, require_clean, &BUILD_SCOPE_PATHS)
    }

    fn resolve_with_scope(
        repo: &Path,
        intent: BuildIntent,
        require_clean: bool,
        scope: &'static [&'static str],
    ) -> Result<Self, BuildIdentityEnvironmentError> {
        let mut status_args = vec![
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
            "--",
        ];
        status_args.extend(scope);
        let status = git(repo, &status_args)?;
        let dirty = !status.stdout.is_empty();
        if require_clean && dirty {
            return Err(BuildIdentityEnvironmentError::DirtyProductionTree);
        }
        if dirty_submodule(&status.stdout) {
            return Err(BuildIdentityEnvironmentError::DirtySubmodule);
        }

        let commit = git_value(repo, &["rev-parse", "--verify", "HEAD"], "commit")?;
        let head_tree = git_value(repo, &["rev-parse", "--verify", "HEAD^{tree}"], "HEAD tree")?;
        let working_tree = if dirty {
            working_tree(repo, scope)?
        } else {
            head_tree.clone()
        };
        Ok(Self {
            intent,
            source: SourceIdentity::Git {
                commit,
                head_tree,
                working_tree,
            },
            scope,
        })
    }
}

fn dirty_submodule(status: &[u8]) -> bool {
    status.split(|byte| *byte == 0).any(|record| {
        if !matches!(record.first(), Some(b'1' | b'2' | b'u')) {
            return false;
        }
        let submodule = record
            .split(|byte| byte.is_ascii_whitespace())
            .nth(2)
            .unwrap_or_default();
        submodule.first() == Some(&b'S')
            && submodule
                .get(2..4)
                .is_some_and(|state| state.iter().any(|byte| *byte != b'.'))
    })
}

fn working_tree(
    repo: &Path,
    scope: &'static [&'static str],
) -> Result<String, BuildIdentityEnvironmentError> {
    let index_dir = tempfile::tempdir().map_err(BuildIdentityEnvironmentError::TemporaryIndex)?;
    let index = index_dir.path().join("index");
    git_with_index(repo, &index, &["read-tree", "HEAD"])?;
    let mut add_args = vec!["add", "-A", "--"];
    add_args.extend(scope);
    git_with_index(repo, &index, &add_args)?;
    git_value_with_index(repo, &index, &["write-tree"], "working tree")
}

fn git_value(
    repo: &Path,
    args: &[&str],
    name: &'static str,
) -> Result<String, BuildIdentityEnvironmentError> {
    output_value(git(repo, args)?, name)
}

fn git_value_with_index(
    repo: &Path,
    index: &Path,
    args: &[&str],
    name: &'static str,
) -> Result<String, BuildIdentityEnvironmentError> {
    output_value(git_with_index(repo, index, args)?, name)
}

fn output_value(
    output: Output,
    name: &'static str,
) -> Result<String, BuildIdentityEnvironmentError> {
    let value =
        String::from_utf8(output.stdout).map_err(BuildIdentityEnvironmentError::InvalidUtf8)?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(BuildIdentityEnvironmentError::EmptyGitValue(name));
    }
    Ok(value)
}

fn git(repo: &Path, args: &[&str]) -> Result<Output, BuildIdentityEnvironmentError> {
    run_git(repo, args, None)
}

fn git_with_index(
    repo: &Path,
    index: &Path,
    args: &[&str],
) -> Result<Output, BuildIdentityEnvironmentError> {
    run_git(repo, args, Some(index))
}

fn run_git(
    repo: &Path,
    args: &[&str],
    index: Option<&Path>,
) -> Result<Output, BuildIdentityEnvironmentError> {
    let mut command = git_command(repo, args, index);
    let output = command
        .output()
        .map_err(BuildIdentityEnvironmentError::GitUnavailable)?;
    if output.status.success() {
        return Ok(output);
    }
    Err(BuildIdentityEnvironmentError::GitCommand {
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn git_command(repo: &Path, args: &[&str], index: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo);
    for variable in GIT_ROUTING_ENVIRONMENT {
        command.env_remove(variable);
    }
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::{
        git_command, BuildIdentityEnvironment, BuildIdentityEnvironmentError,
        GIT_ROUTING_ENVIRONMENT,
    };
    use qol_conventions::artifact::{BuildIntent, SourceIdentity};
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {}", args.join(" "));
    }

    fn repo() -> TempDir {
        let repo = TempDir::new().unwrap();
        git(repo.path(), &["init", "--quiet"]);
        git(repo.path(), &["config", "user.name", "QoL Tests"]);
        git(
            repo.path(),
            &["config", "user.email", "qol-tests@example.invalid"],
        );
        std::fs::create_dir_all(repo.path().join("apps")).unwrap();
        std::fs::create_dir_all(repo.path().join("libs")).unwrap();
        std::fs::create_dir_all(repo.path().join("plugins")).unwrap();
        std::fs::create_dir_all(repo.path().join("tools")).unwrap();
        std::fs::create_dir_all(repo.path().join("vendor")).unwrap();
        std::fs::write(repo.path().join("apps/tracked.txt"), "one\n").unwrap();
        std::fs::write(repo.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(repo.path().join("Cargo.lock"), "").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[test]
    fn production_identity_requires_and_records_a_clean_tree() {
        let repo = repo();
        let identity = BuildIdentityEnvironment::production(repo.path()).unwrap();
        assert_eq!(identity.intent, BuildIntent::Production);
        let SourceIdentity::Git {
            commit,
            head_tree,
            working_tree,
        } = identity.source()
        else {
            panic!("expected Git identity");
        };
        assert!(matches!(commit.len(), 40 | 64));
        assert_eq!(head_tree, working_tree);

        std::fs::write(repo.path().join("untracked.txt"), "dirty\n").unwrap();
        assert!(BuildIdentityEnvironment::production(repo.path()).is_ok());

        std::fs::write(repo.path().join("apps/untracked.txt"), "dirty\n").unwrap();
        assert!(matches!(
            BuildIdentityEnvironment::production(repo.path()),
            Err(BuildIdentityEnvironmentError::DirtyProductionTree)
        ));
    }

    #[test]
    fn development_identity_hashes_dirty_contents_without_touching_the_index() {
        let repo = repo();
        std::fs::write(repo.path().join("apps/tracked.txt"), "two\n").unwrap();
        std::fs::write(repo.path().join("apps/untracked.txt"), "three\n").unwrap();

        let identity = BuildIdentityEnvironment::development(repo.path()).unwrap();
        assert_eq!(identity.intent, BuildIntent::Development);
        let SourceIdentity::Git {
            head_tree,
            working_tree,
            ..
        } = identity.source()
        else {
            panic!("expected Git identity");
        };
        assert_ne!(head_tree, working_tree);
        let original_working_tree = working_tree.clone();
        std::fs::write(repo.path().join("apps/untracked.txt"), "different\n").unwrap();
        let changed = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git { working_tree, .. } = changed.source() else {
            panic!("expected Git identity");
        };
        assert_ne!(&original_working_tree, working_tree);
        let staged = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        assert!(staged.success());
        assert!(repo.path().join("apps/untracked.txt").is_file());
    }

    #[test]
    fn applying_identity_sets_only_the_canonical_build_variables() {
        let repo = repo();
        let identity = BuildIdentityEnvironment::sandbox(repo.path()).unwrap();
        let mut command = Command::new("cargo");
        identity.apply_to(&mut command);
        let environment = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.unwrap().to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            environment
                .get(OsStr::new(qol_conventions::artifact::ENV_BUILD_INTENT))
                .unwrap(),
            OsStr::new("sandbox")
        );
        for key in [
            qol_conventions::artifact::ENV_SOURCE_COMMIT,
            qol_conventions::artifact::ENV_SOURCE_HEAD_TREE,
            qol_conventions::artifact::ENV_SOURCE_WORKING_TREE,
        ] {
            assert!(environment.contains_key(OsStr::new(key)), "{key}");
        }
        assert_eq!(environment.len(), 4);
    }

    #[test]
    fn development_identity_ignores_out_of_scope_untracked_files() {
        let repo = repo();
        std::fs::write(repo.path().join("untracked.txt"), "noise\n").unwrap();
        let baseline = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git {
            working_tree: baseline_tree,
            ..
        } = baseline.source()
        else {
            panic!("expected Git identity");
        };
        let baseline_tree = baseline_tree.clone();

        std::fs::write(repo.path().join("untracked.txt"), "still noise\n").unwrap();
        std::fs::write(repo.path().join("junk.tmp"), "more noise\n").unwrap();
        let unchanged = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git { working_tree, .. } = unchanged.source() else {
            panic!("expected Git identity");
        };
        assert_eq!(working_tree, &baseline_tree);

        std::fs::write(repo.path().join("apps/untracked.txt"), "asset\n").unwrap();
        let changed = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git { working_tree, .. } = changed.source() else {
            panic!("expected Git identity");
        };
        assert_ne!(working_tree, &baseline_tree);
    }

    #[test]
    fn scoped_development_identity_ignores_dirty_files_outside_scope() {
        let repo = repo();
        std::fs::write(repo.path().join("plugins/dirty.rs"), "plugin edit\n").unwrap();
        let baseline = BuildIdentityEnvironment::development_scoped(
            repo.path(),
            &super::TRAY_BUILD_SCOPE_PATHS,
        )
        .unwrap();
        let SourceIdentity::Git {
            working_tree: baseline_tree,
            ..
        } = baseline.source()
        else {
            panic!("expected Git identity");
        };
        let baseline_tree = baseline_tree.clone();

        std::fs::write(
            repo.path().join("plugins/dirty.rs"),
            "another plugin edit\n",
        )
        .unwrap();
        let unchanged = BuildIdentityEnvironment::development_scoped(
            repo.path(),
            &super::TRAY_BUILD_SCOPE_PATHS,
        )
        .unwrap();
        let SourceIdentity::Git { working_tree, .. } = unchanged.source() else {
            panic!("expected Git identity");
        };
        assert_eq!(working_tree, &baseline_tree);

        std::fs::write(repo.path().join("libs/dirty.rs"), "library edit\n").unwrap();
        let changed = BuildIdentityEnvironment::development_scoped(
            repo.path(),
            &super::TRAY_BUILD_SCOPE_PATHS,
        )
        .unwrap();
        let SourceIdentity::Git { working_tree, .. } = changed.source() else {
            panic!("expected Git identity");
        };
        assert_ne!(working_tree, &baseline_tree);
    }

    #[test]
    fn scoped_identity_verifies_unchanged_across_out_of_scope_edits() {
        let repo = repo();
        let identity = BuildIdentityEnvironment::development_scoped(
            repo.path(),
            &super::TRAY_BUILD_SCOPE_PATHS,
        )
        .unwrap();

        std::fs::write(repo.path().join("plugins/dirty.rs"), "plugin edit\n").unwrap();
        identity.verify_unchanged(repo.path()).unwrap();

        std::fs::write(repo.path().join("libs/dirty.rs"), "library edit\n").unwrap();
        assert!(matches!(
            identity.verify_unchanged(repo.path()),
            Err(BuildIdentityEnvironmentError::SourceChanged)
        ));
    }

    #[test]
    fn full_scope_development_identity_still_tracks_plugin_edits() {
        let repo = repo();
        let baseline = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git {
            working_tree: baseline_tree,
            ..
        } = baseline.source()
        else {
            panic!("expected Git identity");
        };
        let baseline_tree = baseline_tree.clone();

        std::fs::write(repo.path().join("plugins/dirty.rs"), "plugin edit\n").unwrap();
        let changed = BuildIdentityEnvironment::development(repo.path()).unwrap();
        let SourceIdentity::Git { working_tree, .. } = changed.source() else {
            panic!("expected Git identity");
        };
        assert_ne!(working_tree, &baseline_tree);
    }

    #[test]
    fn unchanged_verification_detects_source_mutation() {
        let repo = repo();
        let identity = BuildIdentityEnvironment::development(repo.path()).unwrap();
        identity.verify_unchanged(repo.path()).unwrap();

        std::fs::write(repo.path().join("apps/tracked.txt"), "changed\n").unwrap();

        assert!(matches!(
            identity.verify_unchanged(repo.path()),
            Err(BuildIdentityEnvironmentError::SourceChanged)
        ));
    }

    #[test]
    fn unchanged_verification_blames_the_build_for_a_tree_it_dirtied() {
        let repo = repo();
        let identity = BuildIdentityEnvironment::production(repo.path()).unwrap();

        std::fs::write(repo.path().join("Cargo.lock"), "rewritten mid-build\n").unwrap();

        assert!(matches!(
            identity.verify_unchanged(repo.path()),
            Err(BuildIdentityEnvironmentError::SourceChanged)
        ));
    }

    #[test]
    fn git_identity_ignores_inherited_repository_routing() {
        let command = git_command(Path::new("/repository"), &["status"], None);
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        for variable in GIT_ROUTING_ENVIRONMENT {
            assert_eq!(environment.get(OsStr::new(variable)), Some(&None));
        }

        let index = Path::new("/temporary/index");
        let command = git_command(Path::new("/repository"), &["status"], Some(index));
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get(OsStr::new("GIT_INDEX_FILE")),
            Some(&Some(index.as_os_str()))
        );
    }
}
