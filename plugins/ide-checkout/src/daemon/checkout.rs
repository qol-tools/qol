use std::ffi::OsString;
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
    validate_branch(branch)?;
    if !Path::new(project_path).is_dir() {
        return Err(CheckoutError::InvalidParams(format!(
            "Project path does not exist: {project_path}"
        )));
    }

    let remote = git_remote_url(project_path)?;
    let temp_path = temp_path_for(project_path, branch, config);
    std::fs::create_dir_all(config.checkout_root()).map_err(|error| {
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

fn validate_branch(branch: &str) -> Result<(), CheckoutError> {
    if branch.is_empty() || branch.starts_with('-') || branch.contains('\0') {
        return Err(CheckoutError::InvalidParams(
            "Branch name is invalid".to_string(),
        ));
    }
    let output = git_capture(
        &["check-ref-format", "--branch", branch],
        None,
        Duration::from_secs(5),
    )?;
    if !output.status.success() {
        return Err(CheckoutError::InvalidParams(
            "Branch name is invalid".to_string(),
        ));
    }
    Ok(())
}

pub fn open_app(app_id: &str, path: &str, config: &Config) -> Result<(), CheckoutError> {
    let executable = find_executable(app_id, config)
        .ok_or_else(|| CheckoutError::ExecutionFailed(format!("App '{app_id}' not found")))?;
    let launch_path = super::platform::launch_path(Path::new(path)).map_err(|error| {
        CheckoutError::InvalidParams(format!("Could not resolve app path: {error}"))
    })?;
    let mut command = Command::new(executable);
    qol_process::hide_console_window(&mut command);
    command
        .arg(launch_path)
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
    config.checkout_root().join(format!("{repo}_{safe_branch}"))
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
            "--",
            remote,
            &temp,
        ],
        None,
        Duration::from_secs(300),
    )?;
    if single.success() {
        return Ok(());
    }

    let full = git_status(
        &["clone", "--", remote, &temp],
        None,
        Duration::from_secs(300),
    )?;
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
    qol_process::hide_console_window(&mut command);
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

pub(crate) fn find_executable(app_id: &str, config: &Config) -> Option<PathBuf> {
    let app = config.apps.get(app_id)?;
    app.paths.iter().find_map(|entry| resolve_launcher(entry))
}

pub(crate) fn executable_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    executable_in_directories(program, std::env::split_paths(&paths))
}

fn executable_in_directories(
    program: &str,
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories
        .into_iter()
        .flat_map(|directory| super::platform::executable_candidates(&directory, program))
        .find(|candidate| super::platform::is_executable(candidate))
}

fn resolve_launcher(entry: &str) -> Option<PathBuf> {
    if is_bare_program(entry) {
        return executable_on_path(entry);
    }
    let path = expand_path(entry);
    super::platform::is_executable(&path).then_some(path)
}

fn is_bare_program(entry: &str) -> bool {
    !entry.is_empty() && !entry.starts_with('~') && !entry.contains(['/', '\\', '%', ':'])
}

fn expand_path(entry: &str) -> PathBuf {
    expand_path_with(entry, dirs::home_dir().as_deref(), |name| {
        std::env::var_os(name)
    })
}

fn expand_path_with(
    entry: &str,
    home: Option<&Path>,
    variable: impl Fn(&str) -> Option<OsString>,
) -> PathBuf {
    let home_relative = entry
        .strip_prefix("~/")
        .or_else(|| entry.strip_prefix("~\\"));
    if let (Some(rest), Some(home)) = (home_relative, home) {
        return home.join(expand_variables(rest, &variable));
    }
    PathBuf::from(expand_variables(entry, &variable))
}

fn expand_variables(text: &str, variable: &impl Fn(&str) -> Option<OsString>) -> OsString {
    let mut expanded = OsString::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            break;
        };
        let name = &after[..end];
        let value = (!name.is_empty()).then(|| variable(name)).flatten();
        expanded.push(&rest[..start]);
        match value {
            Some(value) => expanded.push(value),
            None => expanded.push(&rest[start..start + end + 2]),
        }
        rest = &after[end + 1..];
    }
    expanded.push(rest);
    expanded
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
    fn validate_branch_rejects_git_options_and_invalid_refnames() {
        for branch in ["", "--help", "bad..branch", "branch lock", "branch\0name"] {
            assert!(validate_branch(branch).is_err(), "branch={branch:?}");
        }
        for branch in ["main", "feature/x", "release-1.2"] {
            assert!(validate_branch(branch).is_ok(), "branch={branch:?}");
        }
    }

    #[test]
    fn temp_path_for_sanitizes_branch_and_keeps_repo_name() {
        let config = config_with(BTreeMap::new(), PathBuf::from("/tmp/tr"));
        let path = temp_path_for("/a/b/myrepo", "feature/new thing", &config);
        assert_eq!(path, PathBuf::from("/tmp/tr/myrepo_feature-new-thing"));
    }

    #[test]
    fn find_executable_returns_first_present_and_executable_path() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("idea");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
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
    fn expand_path_resolves_home_and_environment_tokens() {
        let home = Path::new("/home/me");
        let cases: [(&str, Option<&Path>, PathBuf); 7] = [
            ("~/bin/zed", Some(home), home.join("bin/zed")),
            ("~\\bin\\zed", Some(home), home.join("bin\\zed")),
            ("~/bin/zed", None, PathBuf::from("~/bin/zed")),
            (
                "%LOCALAPPDATA%\\Programs\\Code.exe",
                None,
                PathBuf::from("C:\\Users\\me\\AppData\\Local\\Programs\\Code.exe"),
            ),
            (
                "%MISSING%\\Code.exe",
                None,
                PathBuf::from("%MISSING%\\Code.exe"),
            ),
            ("100%%done", None, PathBuf::from("100%%done")),
            ("/usr/bin/code", Some(home), PathBuf::from("/usr/bin/code")),
        ];
        for (entry, home, expected) in cases {
            let expanded = expand_path_with(entry, home, |name| {
                (name == "LOCALAPPDATA").then(|| OsString::from("C:\\Users\\me\\AppData\\Local"))
            });
            assert_eq!(expanded, expected, "entry={entry:?}");
        }
    }

    #[test]
    fn bare_program_names_use_path_lookup() {
        let cases = [
            ("code", true),
            ("idea64", true),
            ("", false),
            ("~/bin/zed", false),
            ("/usr/bin/code", false),
            ("C:\\Tools\\code.exe", false),
            ("C:code.exe", false),
            ("%LOCALAPPDATA%\\Code.exe", false),
        ];
        for (entry, expected) in cases {
            assert_eq!(is_bare_program(entry), expected, "entry={entry:?}");
        }
    }

    #[test]
    fn path_lookup_finds_a_program_without_executing_it() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root
            .path()
            .join(format!("git{}", std::env::consts::EXE_SUFFIX));
        let marker = root.path().join("launched");
        std::fs::write(
            &executable,
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/launched\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();

        let found = executable_in_directories("git", [root.path().to_path_buf()]);

        assert_eq!(found.as_deref(), Some(executable.as_path()));
        assert!(!marker.exists());
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
