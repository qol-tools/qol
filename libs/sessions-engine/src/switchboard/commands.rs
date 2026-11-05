use std::ffi::OsString;

use anyhow::{bail, Result};
use serde::Serialize;

pub(super) const COMMANDS: &str = "commands";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(super) struct CommandNode {
    pub(super) name: String,
    pub(super) about: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(super) current: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) children: Vec<CommandNode>,
}

#[derive(Debug, Serialize)]
struct Tree {
    status: Option<String>,
    commands: Vec<CommandNode>,
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol sessions switchboard commands");
    }
    let auth = super::auth::offered();
    let tree = Tree {
        status: auth.status,
        commands: vec![auth.command],
    };
    crate::cli::line(serde_json::to_string(&tree)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_leaves_out_an_empty_current_mark_and_no_children() {
        let leaf = CommandNode {
            name: "work".to_owned(),
            about: "logged in".to_owned(),
            current: false,
            children: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&leaf).unwrap(),
            serde_json::json!({ "name": "work", "about": "logged in" })
        );
    }

    #[test]
    fn commands_takes_no_arguments() {
        assert!(run(&[OsString::from("x")]).is_err());
    }
}
