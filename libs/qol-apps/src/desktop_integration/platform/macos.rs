use std::path::Path;
use std::process::Command;

use super::RevealPlan;

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-R").arg(path);
    RevealPlan::Command(command)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn reveal_selects_the_path_in_finder() {
        let RevealPlan::Command(command) = reveal_plan(Path::new("/tmp/capture.png"));
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/open"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("-R"), OsStr::new("/tmp/capture.png")]
        );
    }
}
