use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use super::config::Config;

pub struct Checkout {
    pub temp_path: String,
    pub branch: String,
}

#[derive(Debug)]
pub enum CheckoutError {
    InvalidParams(String),
    ExecutionFailed(String),
    Timeout(String),
}

impl CheckoutError {
    pub fn status(&self) -> u16 {
        match self {
            CheckoutError::InvalidParams(_) => 400,
            CheckoutError::ExecutionFailed(_) | CheckoutError::Timeout(_) => 500,
        }
    }

    pub fn message(&self) -> String {
        match self {
            CheckoutError::InvalidParams(message)
            | CheckoutError::ExecutionFailed(message)
            | CheckoutError::Timeout(message) => message.clone(),
        }
    }
}

pub fn git_checkout(
    project_path: &str,
    branch: &str,
    config: &Config,
) -> Result<Checkout, CheckoutError> {
    validate_path(project_path)?;
    if !Path::new(project_path).is_dir() {
        return Err(CheckoutError::InvalidParams(format!(
            "Project path does not exist: {project_path}"
        )));
    }

    let remote = git_remote_url(project_path)?;
    let temp_path = temp_path_for(project_path, branch, config);
    std::fs::create_dir_all(&config.temp_dir).map_err(|error| {
        CheckoutError::ExecutionFailed(format!("Failed to create temp dir: {error}"))
    })?;

    if temp_path.is_dir() {
        refresh_existing(&temp_path, branch)?;
    } else {
        clone_fresh(&remote, branch, &temp_path)?;
    }

    Ok(Checkout {
        temp_path: temp_path.to_string_lossy().into_owned(),
        branch: branch.to_string(),
    })
}

pub fn open_app(app_id: &str, path: &str, config: &Config) -> Result<(), CheckoutError> {
    let executable = find_executable(app_id, config)
        .ok_or_else(|| CheckoutError::ExecutionFailed(format!("App '{app_id}' not found")))?;
    Command::new(executable)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            CheckoutError::ExecutionFailed(format!("Failed to launch app: {error}"))
        })?;
    Ok(())
}

fn validate_path(path: &str) -> Result<(), CheckoutError> {
    if path.is_empty() {
        return Err(CheckoutError::InvalidParams("Path is empty".to_string()));
    }
    if path.contains('\0') {
        return Err(CheckoutError::InvalidParams(
            "Path contains null bytes".to_string(),
        ));
    }
    if path.contains("..") {
        return Err(CheckoutError::InvalidParams(
            "Path contains directory traversal".to_string(),
        ));
    }
    Ok(())
}

fn git_remote_url(project_path: &str) -> Result<String, CheckoutError> {
    let output = git_capture(
        &["remote", "get-url", "origin"],
        Some(project_path),
        Duration::from_secs(30),
    )?;
    if !output.status.success() {
        return Err(CheckoutError::ExecutionFailed(
            "Could not get git remote URL".to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn temp_path_for(project_path: &str, branch: &str, config: &Config) -> PathBuf {
    let repo = Path::new(project_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let safe_branch: String = branch
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    config.temp_dir.join(format!("{repo}_{safe_branch}"))
}

fn refresh_existing(temp_path: &Path, branch: &str) -> Result<(), CheckoutError> {
    let cwd = temp_path.to_string_lossy();
    git_step(&["fetch", "--all"], &cwd, 120, "fetch from origin")?;
    git_step(
        &["checkout", branch],
        &cwd,
        30,
        &format!("check out branch {branch}"),
    )?;
    git_step(
        &["pull", "--ff-only"],
        &cwd,
        120,
        &format!("fast-forward branch {branch}"),
    )?;
    Ok(())
}

fn git_step(
    args: &[&str],
    cwd: &str,
    timeout_secs: u64,
    action: &str,
) -> Result<(), CheckoutError> {
    let status = git_status(args, Some(cwd), Duration::from_secs(timeout_secs))?;
    if !status.success() {
        return Err(CheckoutError::ExecutionFailed(format!(
            "could not {action}"
        )));
    }
    Ok(())
}

fn clone_fresh(remote: &str, branch: &str, temp_path: &Path) -> Result<(), CheckoutError> {
    let temp = temp_path.to_string_lossy();
    let single = git_status(
        &[
            "clone",
            "--branch",
            branch,
            "--single-branch",
            remote,
            &temp,
        ],
        None,
        Duration::from_secs(300),
    )?;
    if single.success() {
        return Ok(());
    }

    let full = git_status(&["clone", remote, &temp], None, Duration::from_secs(300))?;
    if !full.success() {
        return Err(CheckoutError::ExecutionFailed(
            "git clone failed".to_string(),
        ));
    }
    let checkout = git_status(&["checkout", branch], Some(&temp), Duration::from_secs(30))?;
    if !checkout.success() {
        return Err(CheckoutError::ExecutionFailed(format!(
            "cloned but could not check out branch {branch}"
        )));
    }
    Ok(())
}

fn git_capture(
    args: &[&str],
    cwd: Option<&str>,
    timeout: Duration,
) -> Result<Output, CheckoutError> {
    let mut child = spawn_git(args, cwd, Stdio::piped(), Stdio::null())?;
    wait_with_timeout(&mut child, timeout)?;
    child
        .wait_with_output()
        .map_err(|error| CheckoutError::ExecutionFailed(format!("git output failed: {error}")))
}

fn git_status(
    args: &[&str],
    cwd: Option<&str>,
    timeout: Duration,
) -> Result<ExitStatus, CheckoutError> {
    let mut child = spawn_git(args, cwd, Stdio::null(), Stdio::null())?;
    wait_with_timeout(&mut child, timeout)?;
    child
        .wait()
        .map_err(|error| CheckoutError::ExecutionFailed(format!("git wait failed: {error}")))
}

fn spawn_git(
    args: &[&str],
    cwd: Option<&str>,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<Child, CheckoutError> {
    let mut command = Command::new("git");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command
        .spawn()
        .map_err(|error| CheckoutError::ExecutionFailed(format!("Failed to spawn git: {error}")))
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<(), CheckoutError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CheckoutError::Timeout(
                        "Git operation timed out".to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(CheckoutError::ExecutionFailed(format!(
                    "git wait failed: {error}"
                )))
            }
        }
    }
}

fn find_executable(app_id: &str, config: &Config) -> Option<PathBuf> {
    let app = config.apps.get(app_id)?;
    app.paths
        .iter()
        .map(|path| expand_tilde(path))
        .find(|path| is_executable(path))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::super::config::AppConfig;
    use super::*;
    use std::collections::BTreeMap;

    fn config_with(apps: BTreeMap<String, AppConfig>, temp_dir: PathBuf) -> Config {
        Config { apps, temp_dir }
    }

    #[test]
    fn validate_path_rejects_unsafe_inputs() {
        let cases = ["", "..", "/a/../b", "/a/\0/b"];
        for case in cases {
            assert!(validate_path(case).is_err(), "should reject {case:?}");
        }
        assert!(validate_path("/a/b/c").is_ok());
    }

    #[test]
    fn temp_path_for_sanitizes_branch_and_keeps_repo_name() {
        let config = config_with(BTreeMap::new(), PathBuf::from("/tmp/tr"));
        let path = temp_path_for("/a/b/myrepo", "feature/new thing", &config);
        assert_eq!(path, PathBuf::from("/tmp/tr/myrepo_feature-new-thing"));
    }

    #[test]
    fn find_executable_returns_first_present_and_executable_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("idea");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut apps = BTreeMap::new();
        apps.insert(
            "idea".to_string(),
            AppConfig {
                paths: vec![
                    "/no/such/path".to_string(),
                    exe.to_string_lossy().into_owned(),
                ],
            },
        );
        let config = config_with(apps, PathBuf::from("/tmp"));
        assert_eq!(find_executable("idea", &config), Some(exe));
    }

    #[test]
    fn git_checkout_clones_requested_branch_into_temp_dir() {
        let source = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        init_repo_with_branch(source.path(), "feature/x", "marker.txt");

        let config = config_with(BTreeMap::new(), temp.path().to_path_buf());
        let result = git_checkout(&source.path().to_string_lossy(), "feature/x", &config).unwrap();

        let checked_out = Path::new(&result.temp_path);
        assert!(
            checked_out.join("marker.txt").is_file(),
            "checked-out branch content must be present at {}",
            result.temp_path
        );
        assert_eq!(result.branch, "feature/x");
    }

    #[test]
    fn git_checkout_fails_when_branch_does_not_exist() {
        let source = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        init_repo_with_branch(source.path(), "feature/x", "marker.txt");

        let config = config_with(BTreeMap::new(), temp.path().to_path_buf());
        let result = git_checkout(&source.path().to_string_lossy(), "no-such-branch", &config);
        assert!(
            matches!(result, Err(CheckoutError::ExecutionFailed(_))),
            "a missing branch must not be reported as a successful checkout"
        );
    }

    #[test]
    fn git_checkout_fast_forwards_an_existing_temp_clone() {
        let source = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        init_repo_with_branch(source.path(), "feature/x", "marker.txt");
        let config = config_with(BTreeMap::new(), temp.path().to_path_buf());

        let first = git_checkout(&source.path().to_string_lossy(), "feature/x", &config).unwrap();
        std::fs::write(source.path().join("marker.txt"), "updated").unwrap();
        git(source.path(), &["commit", "-qam", "advance"]);

        let second = git_checkout(&source.path().to_string_lossy(), "feature/x", &config).unwrap();
        assert_eq!(first.temp_path, second.temp_path, "reuses the cached clone");
        let content = std::fs::read_to_string(format!("{}/marker.txt", second.temp_path)).unwrap();
        assert_eq!(
            content, "updated",
            "refresh must fast-forward to the latest commit"
        );
    }

    #[test]
    fn open_app_errors_when_the_app_is_not_configured() {
        let config = config_with(BTreeMap::new(), PathBuf::from("/tmp"));
        assert!(matches!(
            open_app("missing", "/tmp", &config),
            Err(CheckoutError::ExecutionFailed(_))
        ));
    }

    fn init_repo_with_branch(dir: &Path, branch: &str, file: &str) {
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join(file), "base").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "base"]);
        git(dir, &["checkout", "-q", "-b", branch]);
        std::fs::write(dir.join(file), "feature").unwrap();
        git(dir, &["commit", "-q", "-am", "feature"]);
        git(dir, &["remote", "add", "origin", &dir.to_string_lossy()]);
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@e")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@e")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}
