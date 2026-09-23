use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use super::RevealPlan;

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let mut selection = OsString::from("/select,");
    selection.push(path.as_os_str());
    let mut command = Command::new("explorer.exe");
    command.arg(selection);
    command.into()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn reveal_selects_the_path_in_explorer() {
        let RevealPlan::Command(command) = reveal_plan(Path::new(r"C:\capture.png")) else {
            panic!("Windows reveal must use Explorer");
        };
        assert_eq!(command.get_program(), OsStr::new("explorer.exe"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new(r"/select,C:\capture.png")]
        );
    }
}
