use std::ffi::OsString;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde_json::json;

use super::agents::{self, ForkAgent};
use super::commands::CommandNode;
use super::{paths, plugin};

pub(super) const UPDATE: &str = "update";
const CHECK: &str = "--check";

pub(super) fn command() -> CommandNode {
    CommandNode {
        name: UPDATE.to_owned(),
        about: "load the installed CLI Sessions build into this switchboard".to_owned(),
        current: false,
        children: Vec::new(),
    }
}

pub(super) fn run(args: &[OsString], warn: &mut dyn FnMut(String)) -> Result<Vec<String>> {
    let is_check = match args {
        [] => false,
        [flag] if flag == CHECK => true,
        _ => bail!("usage: qol sessions switchboard update [--check]"),
    };
    let dir =
        paths::switchboard_dir().ok_or_else(|| anyhow!("the sessions data folder is not known"))?;
    let fork = ForkAgent::system(warn);
    let stale = stale(&dir, &fork);
    if is_check {
        return Ok(vec![json!({ "stale": stale }).to_string()]);
    }
    if stale.is_empty() {
        return Ok(vec![json!({ "stale": stale, "message": "This switchboard already runs the installed build." }).to_string()]);
    }
    plugin::write(&plugin::dir(&dir))?;
    agents::write_agent_files(&dir, &fork)?;
    Ok(vec![json!({ "stale": stale, "message": "The switchboard loaded the installed build: its plugin reloads now, and forks start with the new instructions." }).to_string()])
}

fn stale(dir: &Path, fork: &ForkAgent) -> Vec<String> {
    plugin::stale(&plugin::dir(dir))
        .into_iter()
        .map(|relative| format!("plugin/{relative}"))
        .chain(agents::stale_agent_files(dir, fork))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_folder_is_stale_until_written_and_then_current() {
        let root = tempfile::TempDir::new().unwrap();
        let fork = ForkAgent::default();
        let before = stale(root.path(), &fork);
        assert!(
            before.contains(&"plugin/hooks/register.tsx".to_owned()),
            "{before:?}"
        );
        assert!(
            before
                .iter()
                .any(|file| file.starts_with(".claude/agents/")),
            "{before:?}"
        );

        plugin::write(&plugin::dir(root.path())).unwrap();
        agents::write_agent_files(root.path(), &fork).unwrap();
        assert_eq!(stale(root.path(), &fork), Vec::<String>::new());

        let module = plugin::dir(root.path()).join("hooks/register.tsx");
        std::fs::write(&module, "an older build").unwrap();
        std::fs::write(
            plugin::dir(root.path()).join("hooks/register.ts"),
            "retired",
        )
        .unwrap();
        assert_eq!(
            stale(root.path(), &fork),
            vec![
                "plugin/hooks/register.tsx".to_owned(),
                "plugin/hooks/register.ts".to_owned()
            ]
        );
    }

    #[test]
    fn update_takes_only_the_check_flag() {
        let mut warn = |_: String| {};
        assert!(run(&[OsString::from("--now")], &mut warn).is_err());
        assert!(run(&[OsString::from(CHECK), OsString::from(CHECK)], &mut warn).is_err());
    }
}
