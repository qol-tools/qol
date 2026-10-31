use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_terminal_sessions::cli::{CliLaunchProgram, CliToolId};

pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const DEFAULT_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const PROFILE_REGISTRY: [&str; 3] = [".config", "account-profiles", "profiles.json"];

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeAccountSpec {
    secret: String,
    token_env: Option<String>,
}

pub struct LaneExecCommand {
    pub program: PathBuf,
    pub prefix: &'static [&'static str],
}

impl LaneExecCommand {
    pub fn current(prefix: &'static [&'static str]) -> Result<Self> {
        let exe = std::env::current_exe().context("failed to locate this binary for lane-exec")?;
        // Linux appends " (deleted)" when the binary was replaced while this process runs;
        // the replacement lives at the stripped path.
        let program = exe
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
            .map_or(exe.clone(), PathBuf::from);
        Ok(Self { program, prefix })
    }
}

#[derive(serde::Deserialize)]
struct ProfileRegistry {
    #[serde(default)]
    profiles: Vec<RegisteredProfile>,
}

#[derive(serde::Deserialize)]
struct RegisteredProfile {
    #[serde(rename = "configDir")]
    config_dir: Option<String>,
    key: Option<String>,
}

fn registered_accounts(home: Option<&Path>) -> Vec<(String, ClaudeAccountSpec)> {
    let Some(home) = home else {
        return Vec::new();
    };
    let path = PROFILE_REGISTRY
        .iter()
        .fold(home.to_path_buf(), |path, part| path.join(part));
    let Ok(encoded) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_registry(&encoded).unwrap_or_else(|error| {
        log::warn!(
            "account profiles in {} are unreadable: {error:#}",
            path.display()
        );
        Vec::new()
    })
}

fn parse_registry(encoded: &str) -> Result<Vec<(String, ClaudeAccountSpec)>> {
    let registry: ProfileRegistry = serde_json::from_str(encoded)?;
    Ok(registry
        .profiles
        .into_iter()
        .filter_map(|profile| {
            let config_dir = profile.config_dir.filter(|dir| !dir.trim().is_empty())?;
            let secret = profile.key.filter(|key| !key.trim().is_empty())?;
            Some((
                config_dir,
                ClaudeAccountSpec {
                    secret,
                    token_env: None,
                },
            ))
        })
        .collect())
}

pub fn config_claude_accounts() -> Result<BTreeMap<String, ClaudeAccountSpec>> {
    let Some(path) = crate::sessions_config_path() else {
        return Ok(BTreeMap::new());
    };
    Ok(crate::config::read(&path, "the sessions config")?
        .map(|config| config.claude_accounts)
        .unwrap_or_default())
}

pub fn claude_account_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let configured = config_claude_accounts().unwrap_or_else(|error| {
        log::warn!("the claude_accounts in the sessions config are unreadable: {error:#}");
        BTreeMap::new()
    });
    account_dirs(
        configured.into_keys().chain(
            registered_accounts(home.as_deref())
                .into_iter()
                .map(|(dir, _)| dir),
        ),
        home.as_deref(),
    )
}

fn account_dirs(dirs: impl IntoIterator<Item = String>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let dir = expand_home(dir.trim(), home);
        if !dir.as_os_str().is_empty() && !found.contains(&dir) {
            found.push(dir);
        }
    }
    found
}

pub fn apply(
    launch: &mut CliLaunchProgram,
    tool: &CliToolId,
    config_dir: Option<&Path>,
    lane_exec: &LaneExecCommand,
) -> Result<()> {
    if tool.as_str() != "claude" {
        return Ok(());
    }
    let login_file = crate::login::file_for(config_dir).filter(|path| path.is_file());
    let Some(config_dir) = config_dir else {
        return match login_file {
            Some(file) => wrap_with_login_file(launch, lane_exec, &file),
            None => Ok(()),
        };
    };
    let home = dirs::home_dir();
    let mut accounts: Vec<_> = config_claude_accounts()?.into_iter().collect();
    // The registry belongs to another tool, so a profile whose secret cannot be read is skipped.
    accounts.extend(
        registered_accounts(home.as_deref())
            .into_iter()
            .filter(|(dir, account)| match qol_secrets::read(&account.secret) {
                Ok(_) => true,
                Err(error) => {
                    log::warn!(
                        "ignoring the registered account for {dir}: secret `{}` cannot be read: {error:#}",
                        account.secret
                    );
                    false
                }
            }),
    );
    apply_with(
        launch,
        config_dir,
        &accounts,
        home.as_deref(),
        lane_exec,
        login_file.as_deref(),
        |secret| qol_secrets::read(secret).map(drop),
    )
}

fn apply_with(
    launch: &mut CliLaunchProgram,
    config_dir: &Path,
    accounts: &[(String, ClaudeAccountSpec)],
    home: Option<&Path>,
    lane_exec: &LaneExecCommand,
    login_file: Option<&Path>,
    check_secret: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    let config_dir_text = config_dir
        .to_str()
        .with_context(|| format!("{CONFIG_DIR_ENV} `{}` is not UTF-8", config_dir.display()))?;
    launch
        .env
        .push((CONFIG_DIR_ENV.to_owned(), config_dir_text.to_owned()));
    let Some(account) = accounts
        .iter()
        .find(|(dir, _)| expand_home(dir, home) == config_dir)
        .map(|(_, account)| account)
    else {
        return match login_file {
            Some(file) => wrap_with_login_file(launch, lane_exec, file),
            None => Ok(()),
        };
    };
    let token_env = account.token_env.as_deref().unwrap_or(DEFAULT_TOKEN_ENV);
    if token_env.is_empty() || token_env.contains('=') {
        bail!("claude_accounts token_env `{token_env}` is not a valid environment variable name");
    }
    check_secret(&account.secret).with_context(|| {
        format!(
            "the Claude account for {config_dir_text} names secret `{}`, which cannot be read",
            account.secret
        )
    })?;
    wrap(
        launch,
        lane_exec,
        [
            "--secret".to_owned(),
            account.secret.clone(),
            "--env".to_owned(),
            token_env.to_owned(),
        ],
    );
    Ok(())
}

fn wrap_with_login_file(
    launch: &mut CliLaunchProgram,
    lane_exec: &LaneExecCommand,
    file: &Path,
) -> Result<()> {
    let file = file
        .to_str()
        .with_context(|| format!("the lane login `{}` is not UTF-8", file.display()))?;
    wrap(
        launch,
        lane_exec,
        ["--login-file".to_owned(), file.to_owned()],
    );
    Ok(())
}

fn wrap(
    launch: &mut CliLaunchProgram,
    lane_exec: &LaneExecCommand,
    login_args: impl IntoIterator<Item = String>,
) {
    let mut args: Vec<String> = lane_exec
        .prefix
        .iter()
        .map(|part| (*part).to_owned())
        .collect();
    args.extend(login_args);
    args.extend(["--".to_owned(), launch.program.clone()]);
    args.append(&mut launch.args);
    launch.program = lane_exec.program.to_string_lossy().into_owned();
    launch.args = args;
}

fn expand_home(dir: &str, home: Option<&Path>) -> PathBuf {
    match (dir.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(secret: &str, token_env: Option<&str>) -> ClaudeAccountSpec {
        ClaudeAccountSpec {
            secret: secret.to_owned(),
            token_env: token_env.map(str::to_owned),
        }
    }

    fn qol() -> LaneExecCommand {
        LaneExecCommand {
            program: PathBuf::from("/bin/qol"),
            prefix: &["sessions", "lane-exec"],
        }
    }

    fn claude_launch() -> CliLaunchProgram {
        let mut launch = CliLaunchProgram::new("claude");
        launch.args = vec!["--resume".to_owned(), "abc".to_owned()];
        launch
    }

    fn accounts() -> Vec<(String, ClaudeAccountSpec)> {
        vec![(
            "~/.claude-personal".to_owned(),
            account("claude-code-personal", None),
        )]
    }

    #[test]
    fn a_configured_account_reads_its_token_inside_the_lane() {
        let mut launch = claude_launch();
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal/"),
            &accounts(),
            Some(Path::new("/Users/me")),
            &qol(),
            None,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.program, "/bin/qol");
        assert_eq!(
            launch.args,
            [
                "sessions",
                "lane-exec",
                "--secret",
                "claude-code-personal",
                "--env",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "--",
                "claude",
                "--resume",
                "abc",
            ]
        );
        assert_eq!(
            launch.env,
            [(
                "CLAUDE_CONFIG_DIR".to_owned(),
                "/Users/me/.claude-personal/".to_owned()
            )]
        );
    }

    #[test]
    fn an_unmapped_config_dir_only_travels_as_the_config_dir() {
        let mut launch = claude_launch();
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-work"),
            &accounts(),
            Some(Path::new("/Users/me")),
            &qol(),
            None,
            |_| panic!("no secret is read for an unmapped account"),
        )
        .unwrap();

        assert_eq!(launch.program, "claude");
        assert_eq!(launch.args, ["--resume", "abc"]);
        assert_eq!(launch.env.len(), 1);
    }

    #[test]
    fn an_unmapped_config_dir_starts_under_the_login_its_caller_left() {
        let mut launch = claude_launch();
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-work"),
            &accounts(),
            Some(Path::new("/Users/me")),
            &qol(),
            Some(Path::new("/data/lane-logins/_Users_me_.claude-work")),
            |_| panic!("no secret is read for an unmapped account"),
        )
        .unwrap();

        assert_eq!(launch.program, "/bin/qol");
        assert_eq!(
            launch.args,
            [
                "sessions",
                "lane-exec",
                "--login-file",
                "/data/lane-logins/_Users_me_.claude-work",
                "--",
                "claude",
                "--resume",
                "abc",
            ]
        );
    }

    #[test]
    fn a_configured_account_wins_over_the_callers_login() {
        let mut launch = claude_launch();
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal"),
            &accounts(),
            Some(Path::new("/Users/me")),
            &qol(),
            Some(Path::new("/data/lane-logins/_Users_me_.claude-personal")),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.args[2], "--secret");
    }

    #[test]
    fn an_unreadable_secret_refuses_the_launch_instead_of_falling_back() {
        let mut launch = claude_launch();
        let error = apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal"),
            &accounts(),
            Some(Path::new("/Users/me")),
            &qol(),
            None,
            |_| bail!("no such item"),
        )
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("claude-code-personal"),
            "{error:#}"
        );
    }

    #[test]
    fn an_api_key_account_names_its_own_variable() {
        let mut launch = claude_launch();
        let accounts = vec![(
            "/srv/claude".to_owned(),
            account("claude-api", Some("ANTHROPIC_API_KEY")),
        )];
        apply_with(
            &mut launch,
            Path::new("/srv/claude"),
            &accounts,
            None,
            &qol(),
            None,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.args[5], "ANTHROPIC_API_KEY");
    }

    #[test]
    fn the_sessions_config_wins_over_a_registered_profile_for_the_same_directory() {
        let mut launch = claude_launch();
        let accounts = vec![
            (
                "/Users/me/.claude-personal".to_owned(),
                account("from-config", None),
            ),
            (
                "~/.claude-personal".to_owned(),
                account("from-registry", None),
            ),
        ];
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal"),
            &accounts,
            Some(Path::new("/Users/me")),
            &qol(),
            None,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.args[3], "from-config");
    }

    #[test]
    fn registered_profiles_with_a_key_become_accounts() {
        let registry = r#"{"profiles": [
            {"name": "personal", "configDir": "~/.claude-personal", "key": "claude-personal"},
            {"name": "work", "configDir": "/Users/me/.claude"},
            {"name": "blank", "configDir": "", "key": "claude-blank"}
        ]}"#;

        let accounts = parse_registry(registry).unwrap();

        assert_eq!(
            accounts,
            [(
                "~/.claude-personal".to_owned(),
                account("claude-personal", None)
            )]
        );
    }

    #[test]
    fn a_registered_profile_maps_a_launch_without_any_sessions_config() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".config/account-profiles");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profiles.json"),
            r#"{"profiles": [{"configDir": "~/.claude-personal", "key": "claude-personal"}]}"#,
        )
        .unwrap();
        let config_dir = home.path().join(".claude-personal");
        let mut launch = claude_launch();

        apply_with(
            &mut launch,
            &config_dir,
            &registered_accounts(Some(home.path())),
            Some(home.path()),
            &qol(),
            None,
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.args[3], "claude-personal");
    }

    #[test]
    fn account_dirs_expand_the_home_and_list_each_dir_once() {
        let dirs = account_dirs(
            [
                "~/.claude-deepmind",
                " ~/.claude-deepmind/ ",
                "/srv/claude-team",
                "",
            ]
            .map(str::to_owned),
            Some(Path::new("/home/me")),
        );

        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/me/.claude-deepmind"),
                PathBuf::from("/srv/claude-team"),
            ]
        );
    }

    #[test]
    fn an_unreadable_registry_contributes_no_accounts() {
        assert!(parse_registry("not json").is_err());
        assert!(registered_accounts(Some(Path::new("/nonexistent-home"))).is_empty());
    }
}
