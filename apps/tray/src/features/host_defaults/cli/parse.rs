use qol_host_defaults::Role;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Topic {
    General,
    List,
    Probe,
    Claim,
    Release,
    Open,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Command {
    Help(Topic),
    List,
    Probe { role: Role },
    Claim { role: Role, plugin: String },
    Release { role: Role },
    Open { role: Role, args: Vec<String> },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Parsed {
    pub command: Command,
    pub json: bool,
}

pub(super) fn parse(args: &[String]) -> Result<Parsed, String> {
    let json = args.iter().any(|arg| arg == "--json");
    let tokens: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--json")
        .collect();
    let command = match help_request(&tokens)? {
        Some(topic) => Command::Help(topic),
        None => command_of(&tokens)?,
    };
    Ok(Parsed { command, json })
}

fn is_help(token: &str) -> bool {
    matches!(token, "help" | "--help" | "-h")
}

fn help_request(tokens: &[&str]) -> Result<Option<Topic>, String> {
    let Some((first, rest)) = tokens.split_first() else {
        return Ok(Some(Topic::General));
    };
    if is_help(first) {
        return match rest {
            [] => Ok(Some(Topic::General)),
            [name] => topic_of(name).map(Some),
            _ => Err("help takes one command name".to_string()),
        };
    }
    let topic = topic_of(first)?;
    if topic == Topic::Open {
        return Ok((rest.len() == 1 && is_help(rest[0])).then_some(topic));
    }
    let middle = rest.split_last().map_or(rest, |(_, middle)| middle);
    if middle.contains(&"help") {
        return Err("help cannot appear in the middle of a command path".to_string());
    }
    Ok(rest.iter().any(|token| is_help(token)).then_some(topic))
}

fn topic_of(name: &str) -> Result<Topic, String> {
    match name {
        "list" => Ok(Topic::List),
        "probe" => Ok(Topic::Probe),
        "claim" => Ok(Topic::Claim),
        "release" => Ok(Topic::Release),
        "open" => Ok(Topic::Open),
        other => Err(format!("unknown command {other:?}")),
    }
}

fn command_of(tokens: &[&str]) -> Result<Command, String> {
    let Some((name, rest)) = tokens.split_first() else {
        return Ok(Command::Help(Topic::General));
    };
    match topic_of(name)? {
        Topic::List => no_arguments(rest).map(|()| Command::List),
        Topic::Probe => single_role(rest).map(|role| Command::Probe { role }),
        Topic::Release => single_role(rest).map(|role| Command::Release { role }),
        Topic::Claim => claim_of(rest),
        Topic::Open => open_of(rest),
        Topic::General => Ok(Command::Help(Topic::General)),
    }
}

fn no_arguments(rest: &[&str]) -> Result<(), String> {
    match rest {
        [] => Ok(()),
        [extra, ..] => Err(format!("unexpected argument {extra:?}")),
    }
}

fn single_role(rest: &[&str]) -> Result<Role, String> {
    match rest {
        [role] => parse_role(role),
        [] => Err("a role is required".to_string()),
        [_, extra, ..] => Err(format!("unexpected argument {extra:?}")),
    }
}

pub(super) fn parse_role(value: &str) -> Result<Role, String> {
    Role::parse(value).ok_or_else(|| {
        let known: Vec<&str> = Role::ALL.iter().map(|role| role.id()).collect();
        format!("unknown role {value:?}; roles: {}", known.join(", "))
    })
}

fn claim_of(rest: &[&str]) -> Result<Command, String> {
    let mut role = None;
    let mut plugin = None;
    let mut index = 0;
    while index < rest.len() {
        let token = rest[index];
        index += 1;
        if let Some(value) = token.strip_prefix("--plugin=") {
            plugin = Some(value.to_string());
            continue;
        }
        if token == "--plugin" {
            let value = rest
                .get(index)
                .ok_or_else(|| "--plugin needs a plugin id".to_string())?;
            plugin = Some((*value).to_string());
            index += 1;
            continue;
        }
        if token.starts_with('-') || role.is_some() {
            return Err(format!("unexpected argument {token:?}"));
        }
        role = Some(parse_role(token)?);
    }
    let role = role.ok_or_else(|| "a role is required".to_string())?;
    let plugin = plugin
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "claim needs --plugin <id>".to_string())?;
    Ok(Command::Claim { role, plugin })
}

fn open_of(rest: &[&str]) -> Result<Command, String> {
    let Some((role, args)) = rest.split_first() else {
        return Err("a role is required".to_string());
    };
    if args.is_empty() {
        return Err("open needs a path or file:// uri".to_string());
    }
    Ok(Command::Open {
        role: parse_role(role)?,
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn parsed(values: &[&str]) -> Parsed {
        parse(&args(values)).expect("parses")
    }

    fn refused(values: &[&str]) -> String {
        parse(&args(values)).expect_err("refused")
    }

    #[test]
    fn every_help_form_resolves_to_its_topic() {
        let cases: &[(&[&str], Topic)] = &[
            (&[], Topic::General),
            (&["help"], Topic::General),
            (&["--help"], Topic::General),
            (&["-h"], Topic::General),
            (&["help", "claim"], Topic::Claim),
            (&["claim", "help"], Topic::Claim),
            (&["claim", "--help"], Topic::Claim),
            (
                &["claim", "slot.screenshot", "--plugin", "x", "help"],
                Topic::Claim,
            ),
            (&["open", "help"], Topic::Open),
            (&["list", "-h"], Topic::List),
            (&["help", "probe"], Topic::Probe),
            (&["release", "help"], Topic::Release),
        ];
        for (values, topic) in cases {
            assert_eq!(parsed(values).command, Command::Help(*topic), "{values:?}");
        }
    }

    #[test]
    fn help_in_the_middle_is_refused() {
        for values in [
            &["claim", "help", "slot.screenshot"][..],
            &["help", "claim", "release"][..],
        ] {
            assert!(refused(values).contains("help"), "{values:?}");
        }
    }

    #[test]
    fn open_keeps_help_looking_arguments_as_targets() {
        assert_eq!(
            parsed(&["open", "folder.open", "help"]).command,
            Command::Open {
                role: Role::FolderOpen,
                args: args(&["help"])
            }
        );
    }

    #[test]
    fn json_is_a_flag_anywhere() {
        for values in [&["--json", "list"][..], &["list", "--json"][..]] {
            let parsed = parsed(values);
            assert_eq!(parsed.command, Command::List);
            assert!(parsed.json);
        }
        assert!(!parsed(&["list"]).json);
    }

    #[test]
    fn commands_parse_with_their_arguments() {
        assert_eq!(
            parsed(&["probe", "slot.launcher"]).command,
            Command::Probe {
                role: Role::SlotLauncher
            }
        );
        assert_eq!(
            parsed(&["release", "folder.open"]).command,
            Command::Release {
                role: Role::FolderOpen
            }
        );
        assert_eq!(
            parsed(&["claim", "slot.screenshot", "--plugin", "qol-shot"]).command,
            Command::Claim {
                role: Role::SlotScreenshot,
                plugin: "qol-shot".to_string()
            }
        );
        assert_eq!(
            parsed(&["claim", "--plugin=qol-shot", "slot.screenshot"]).command,
            Command::Claim {
                role: Role::SlotScreenshot,
                plugin: "qol-shot".to_string()
            }
        );
        assert_eq!(
            parsed(&["open", "folder.open", "file:///tmp/a", "/b"]).command,
            Command::Open {
                role: Role::FolderOpen,
                args: args(&["file:///tmp/a", "/b"])
            }
        );
    }

    #[test]
    fn malformed_commands_are_refused_with_a_reason() {
        let cases: &[(&[&str], &str)] = &[
            (&["frobnicate"], "unknown command"),
            (&["list", "extra"], "unexpected argument"),
            (&["probe"], "role is required"),
            (&["probe", "nope"], "unknown role"),
            (&["probe", "slot.launcher", "x"], "unexpected argument"),
            (&["claim", "slot.launcher"], "--plugin"),
            (&["claim", "--plugin", "x"], "role is required"),
            (&["claim", "slot.launcher", "--plugin"], "needs a plugin id"),
            (
                &["claim", "slot.launcher", "--bogus", "--plugin", "x"],
                "unexpected argument",
            ),
            (&["open", "folder.open"], "path"),
            (&["open"], "role is required"),
        ];
        for (values, needle) in cases {
            let message = refused(values);
            assert!(message.contains(needle), "{values:?}: {message}");
        }
    }

    #[test]
    fn unknown_roles_list_the_valid_ones() {
        let message = refused(&["probe", "slot.window_snap"]);
        assert!(message.contains("folder.open") && message.contains("slot.dictation"));
    }
}
