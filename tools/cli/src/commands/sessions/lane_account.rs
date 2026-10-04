use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use qol_terminal_sessions::cli::{CliLaunchProgram, CliToolId};

const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const DEFAULT_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ClaudeAccountSpec {
    secret: String,
    token_env: Option<String>,
}

/// Starts a claude lane under the Claude account its caller runs under: the
/// caller's config dir travels with the launch, and when `claude_accounts`
/// names a secret for that dir, the lane reads its token through
/// `qol sessions lane-exec` so the token never passes through the terminal.
pub(super) fn apply(launch: &mut CliLaunchProgram, tool: &CliToolId) -> Result<()> {
    if tool.as_str() != "claude" {
        return Ok(());
    }
    let Some(config_dir) = caller_config_dir() else {
        return Ok(());
    };
    let accounts = super::spawn::config_claude_accounts()?;
    let qol = std::env::current_exe().context("failed to locate the qol binary for lane-exec")?;
    apply_with(
        launch,
        &config_dir,
        &accounts,
        dirs::home_dir().as_deref(),
        &qol,
        |secret| qol_secrets::read(secret).map(drop),
    )
}

#[cfg(not(test))]
fn caller_config_dir() -> Option<PathBuf> {
    std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from)
}

#[cfg(test)]
fn caller_config_dir() -> Option<PathBuf> {
    None
}

fn apply_with(
    launch: &mut CliLaunchProgram,
    config_dir: &Path,
    accounts: &BTreeMap<String, ClaudeAccountSpec>,
    home: Option<&Path>,
    qol: &Path,
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
        return Ok(());
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
    let mut args = vec![
        "sessions".to_owned(),
        "lane-exec".to_owned(),
        "--secret".to_owned(),
        account.secret.clone(),
        "--env".to_owned(),
        token_env.to_owned(),
        "--".to_owned(),
        launch.program.clone(),
    ];
    args.append(&mut launch.args);
    launch.program = qol.to_string_lossy().into_owned();
    launch.args = args;
    Ok(())
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

    fn claude_launch() -> CliLaunchProgram {
        let mut launch = CliLaunchProgram::new("claude");
        launch.args = vec!["--resume".to_owned(), "abc".to_owned()];
        launch
    }

    fn accounts() -> BTreeMap<String, ClaudeAccountSpec> {
        BTreeMap::from([(
            "~/.claude-personal".to_owned(),
            account("claude-code-personal", None),
        )])
    }

    #[test]
    fn a_configured_account_reads_its_token_inside_the_lane() {
        let mut launch = claude_launch();
        apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal/"),
            &accounts(),
            Some(Path::new("/Users/me")),
            Path::new("/bin/qol"),
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
            Path::new("/bin/qol"),
            |_| panic!("no secret is read for an unmapped account"),
        )
        .unwrap();

        assert_eq!(launch.program, "claude");
        assert_eq!(launch.args, ["--resume", "abc"]);
        assert_eq!(launch.env.len(), 1);
    }

    #[test]
    fn an_unreadable_secret_refuses_the_launch_instead_of_falling_back() {
        let mut launch = claude_launch();
        let error = apply_with(
            &mut launch,
            Path::new("/Users/me/.claude-personal"),
            &accounts(),
            Some(Path::new("/Users/me")),
            Path::new("/bin/qol"),
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
        let accounts = BTreeMap::from([(
            "/srv/claude".to_owned(),
            account("claude-api", Some("ANTHROPIC_API_KEY")),
        )]);
        apply_with(
            &mut launch,
            Path::new("/srv/claude"),
            &accounts,
            None,
            Path::new("/bin/qol"),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(launch.args[5], "ANTHROPIC_API_KEY");
    }
}
