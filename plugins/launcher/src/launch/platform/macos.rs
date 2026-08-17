use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn launch_app(path: &Path, _exec: &[String]) -> io::Result<()> {
    super::super::open_path(path)
}

pub(crate) fn daemon_action_args(path: &Path, _exec: &[String]) -> Option<(String, String)> {
    let run_script = read_run_script(path)?;
    let argv = parse_run_script(&run_script)?;
    super::daemon_exec_args(&argv).map(|(target, action)| (target.to_string(), action.to_string()))
}

fn read_run_script(path: &Path) -> Option<String> {
    let apps_dir = PathBuf::from(std::env::var("HOME").ok()?)
        .join("Applications")
        .join("QoL");
    if !path.starts_with(apps_dir) {
        return None;
    }
    std::fs::read_to_string(path.join("Contents/MacOS/run")).ok()
}

fn parse_run_script(script: &str) -> Option<Vec<String>> {
    let command = script.strip_prefix("#!/bin/sh\nexec ")?.trim_end();
    parse_single_quoted_words(command)
}

fn parse_single_quoted_words(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        rest = rest.strip_prefix('\'')?;
        let mut word = String::new();
        loop {
            let (head, tail) = rest.split_once('\'')?;
            word.push_str(head);
            if let Some(after) = tail.strip_prefix("\\'") {
                word.push('\'');
                rest = after.strip_prefix('\'')?;
                continue;
            }
            rest = tail;
            break;
        }
        if let Some(after) = rest.strip_prefix(' ') {
            rest = after;
        } else if !rest.is_empty() {
            return None;
        }
        words.push(word);
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::parse_run_script;

    #[test]
    fn parses_generated_qol_run_script_argv() {
        let script = "#!/bin/sh\nexec '/Applications/qol-tray.app/Contents/MacOS/qol-courier' 'exec' 'plugin-monitor' 'settings'\n";

        let argv = parse_run_script(script).unwrap();

        assert_eq!(
            argv,
            [
                "/Applications/qol-tray.app/Contents/MacOS/qol-courier",
                "exec",
                "plugin-monitor",
                "settings",
            ]
        );
    }

    #[test]
    fn parses_args_containing_spaces_and_escaped_quotes() {
        let script = "#!/bin/sh\nexec '/opt/qol-tray' 'exec' 'plugin-x' 'my action' 'it'\\''s'\n";

        let argv = parse_run_script(script).unwrap();

        assert_eq!(
            argv,
            ["/opt/qol-tray", "exec", "plugin-x", "my action", "it's"]
        );
    }

    #[test]
    fn rejects_open_style_scripts_with_unquoted_first_word() {
        let script = "#!/bin/sh\nexec /usr/bin/open 'https://example.com'\n";

        assert_eq!(parse_run_script(script), None);
    }

    #[test]
    fn rejects_scripts_without_shebang_or_exec_prefix() {
        assert_eq!(
            parse_run_script("exec '/opt/qol-tray' 'exec' 'a' 'b'\n"),
            None
        );
        assert_eq!(parse_run_script("#!/bin/sh\nexec\n"), None);
    }

    #[test]
    fn rejects_trailing_garbage_after_quoted_words() {
        let script = "#!/bin/sh\nexec '/opt/qol-tray' 'exec' 'a' 'b'\nunexpected\n";

        assert_eq!(parse_run_script(script), None);
    }
}
