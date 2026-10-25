use std::path::Path;
use std::process::Command;

use super::RevealPlan;

pub(super) use super::commands::{open, open_checked};
pub(super) use super::handlers::{
    file_type, open_handlers, open_with_handler, open_with_supported,
};

const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "gnome-terminal",
    "kgx",
    "konsole",
    "xfce4-terminal",
    "mate-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "tilix",
    "xterm",
];

pub(super) fn terminal_commands(folder: &Path) -> Vec<Command> {
    let preferred = std::env::var("TERMINAL")
        .ok()
        .filter(|name| !name.is_empty());
    preferred
        .iter()
        .map(String::as_str)
        .chain(TERMINALS.iter().copied())
        .map(|program| {
            let mut command = Command::new(program);
            command.current_dir(folder);
            command
        })
        .collect()
}

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    target.to_path_buf().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_opens_a_files_parent_directory() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("capture.png");
        std::fs::write(&file, []).unwrap();

        let RevealPlan::Open(target) = reveal_plan(&file) else {
            panic!("Linux reveal must open a directory");
        };
        assert_eq!(target, temp.path());
    }

    #[test]
    fn reveal_opens_a_directory_itself() {
        let temp = tempfile::tempdir().unwrap();
        let RevealPlan::Open(target) = reveal_plan(temp.path()) else {
            panic!("Linux reveal must open a directory");
        };
        assert_eq!(target, temp.path());
    }
}
