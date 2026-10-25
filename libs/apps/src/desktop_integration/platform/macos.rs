use std::path::Path;
use std::process::Command;

use super::RevealPlan;

pub(super) use super::commands::{open, open_checked};
pub(super) use super::unsupported::{
    file_type, open_handlers, open_with_handler, open_with_supported,
};

pub(super) fn terminal_commands(folder: &Path) -> Vec<Command> {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-a").arg("Terminal").arg(folder);
    vec![command]
}

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-R").arg(path);
    command.into()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn reveal_selects_the_path_in_finder() {
        let RevealPlan::Command(command) = reveal_plan(Path::new("/tmp/capture.png")) else {
            panic!("macOS reveal must use Finder");
        };
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/open"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("-R"), OsStr::new("/tmp/capture.png")]
        );
    }
}
