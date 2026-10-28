mod platform;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

const USAGE: &str = "lane-exec (--secret NAME --env VAR | --login-file PATH) -- PROGRAM [ARGS...]";

#[derive(Debug, PartialEq, Eq)]
enum Login {
    Secret { secret: String, env: String },
    File(PathBuf),
}

#[derive(Debug, PartialEq, Eq)]
struct LaneExec {
    login: Login,
    program: OsString,
    args: Vec<OsString>,
}

pub fn run(args: &[OsString]) -> Result<()> {
    let lane = parse(args)?;
    let mut command = Command::new(&lane.program);
    command.args(&lane.args);
    match &lane.login {
        Login::Secret { secret, env } => {
            let token = qol_secrets::read(secret)
                .with_context(|| format!("lane-exec could not read secret `{secret}`"))?;
            command.env(env, token);
        }
        Login::File(path) => {
            for name in crate::login::LOGIN_ENVS {
                command.env_remove(name);
            }
            command.envs(crate::login::read(path)?);
        }
    }
    platform::run_in_place(command).with_context(|| {
        format!(
            "lane-exec failed to start {}",
            lane.program.to_string_lossy()
        )
    })
}

fn parse(args: &[OsString]) -> Result<LaneExec> {
    let (mut secret, mut env, mut login_file) = (None, None, None);
    let mut rest = args.iter();
    loop {
        let Some(flag) = rest.next() else {
            bail!("lane-exec needs `-- PROGRAM`; usage: {USAGE}");
        };
        let slot = match flag.to_str() {
            Some("--") => break,
            Some("--secret") => &mut secret,
            Some("--env") => &mut env,
            Some("--login-file") => &mut login_file,
            _ => bail!(
                "unexpected lane-exec argument `{}`; usage: {USAGE}",
                flag.to_string_lossy()
            ),
        };
        let value = rest
            .next()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .with_context(|| format!("{} needs a value; usage: {USAGE}", flag.to_string_lossy()))?;
        *slot = Some(value.to_owned());
    }
    let login = match (secret, env, login_file) {
        (Some(secret), Some(env), None) => Login::Secret { secret, env },
        (None, None, Some(file)) => Login::File(PathBuf::from(file)),
        _ => bail!("lane-exec needs --secret with --env, or --login-file alone; usage: {USAGE}"),
    };
    let Some(program) = rest.next().cloned() else {
        bail!("lane-exec needs a program after `--`; usage: {USAGE}");
    };
    Ok(LaneExec {
        login,
        program,
        args: rest.cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn everything_after_the_separator_is_the_lane_command_verbatim() {
        let lane = parse(&argv(&[
            "--secret", "s", "--env", "TOKEN", "--", "claude", "--secret", "--", "task",
        ]))
        .unwrap();
        assert_eq!(
            lane,
            LaneExec {
                login: Login::Secret {
                    secret: "s".to_owned(),
                    env: "TOKEN".to_owned(),
                },
                program: "claude".into(),
                args: argv(&["--secret", "--", "task"]),
            }
        );
    }

    #[test]
    fn a_missing_flag_value_or_program_is_refused() {
        for args in [
            argv(&["--env", "TOKEN", "--", "claude"]),
            argv(&["--secret", "s", "--env", "TOKEN", "--"]),
            argv(&["--secret", "--env", "TOKEN", "--", "claude"]),
            argv(&["--secret", "s", "--env", "TOKEN", "claude"]),
            argv(&["--secret", "s", "--login-file", "/l", "--", "claude"]),
            argv(&[
                "--secret",
                "s",
                "--env",
                "T",
                "--login-file",
                "/l",
                "--",
                "claude",
            ]),
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn a_login_file_alone_starts_the_lane() {
        let lane = parse(&argv(&["--login-file", "/l", "--", "claude", "-p"])).unwrap();
        assert_eq!(lane.login, Login::File(PathBuf::from("/l")));
        assert_eq!(lane.args, argv(&["-p"]));
    }
}
