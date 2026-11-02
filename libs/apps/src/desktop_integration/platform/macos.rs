use std::path::Path;
use std::process::Command;
use std::time::Duration;

use qol_process::BoundedCommandOutput;

use super::RevealPlan;

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const QUERY_LIMIT: usize = 4096;
const DEFAULT_APP_SCRIPT: &str = "function run(argv) { ObjC.import('AppKit'); \
const app = $.NSWorkspace.sharedWorkspace.URLForApplicationToOpenURL($.NSURL.fileURLWithPath(argv[0])); \
return app.isNil() ? '' : $.NSFileManager.defaultManager.displayNameAtPath(app.path).js; }";

pub(super) use super::commands::{open, open_checked};
pub(super) use super::unsupported::{
    file_type, open_handlers, open_with_handler, open_with_supported,
};

pub(super) fn default_app_name(path: &Path) -> Option<String> {
    let mut command = Command::new("/usr/bin/osascript");
    command
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(DEFAULT_APP_SCRIPT)
        .arg(path);
    match qol_process::run_owned_with_output_timeout(command, QUERY_TIMEOUT, QUERY_LIMIT).ok()? {
        BoundedCommandOutput::Completed(output) if output.status.success() => {
            app_display_name(&String::from_utf8_lossy(output.stdout.as_bytes()))
        }
        _ => None,
    }
}

fn app_display_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let name = name.strip_suffix(".app").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

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
    fn the_app_name_drops_the_bundle_suffix_and_blank_answers() {
        assert_eq!(
            app_display_name("TextEdit.app\n").as_deref(),
            Some("TextEdit")
        );
        assert_eq!(
            app_display_name("Visual Studio Code").as_deref(),
            Some("Visual Studio Code")
        );
        assert_eq!(app_display_name("  \n"), None);
    }

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
