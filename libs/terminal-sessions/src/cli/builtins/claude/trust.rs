use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

pub(super) fn untrusted_folder(config_file: &Path, cwd: &Path, user_home: &Path) -> Option<String> {
    let config = fs::read_to_string(config_file).ok()?;
    let config = serde_json::from_str::<Value>(&config).ok()?;
    let projects = config.get("projects")?.as_object()?;
    let trusted_folders = projects
        .iter()
        .filter(|(_, project)| {
            project
                .get("hasTrustDialogAccepted")
                .and_then(Value::as_bool)
                == Some(true)
        })
        .map(|(folder, _)| forward_slashes(folder))
        .collect::<HashSet<_>>();
    let trusted = cwd.ancestors().any(|folder| {
        folder
            .to_str()
            .is_some_and(|folder| trusted_folders.contains(&forward_slashes(folder)))
    });
    if trusted {
        return None;
    }
    let cwd = cwd.display();
    let at_home =
        forward_slashes(&cwd.to_string()) == forward_slashes(&user_home.display().to_string());
    Some(match at_home {
        true => format!(
            "Claude Code never remembers trust for the home folder {cwd}, so a claude session started there always stops on its \"Do you trust this folder?\" question, which --dangerously-skip-permissions does not skip, and waits unseen. Start it in a project folder instead."
        ),
        false => format!(
            "Claude Code has not been told to trust {cwd} (no trust record for it or a parent folder in {}), so a claude session started there stops on its \"Do you trust this folder?\" question, which --dangerously-skip-permissions does not skip, and waits unseen. Trusting a folder is the user's call: they run claude in {cwd} once and accept, or the session starts in a folder they already trust.",
            config_file.display()
        ),
    })
}

fn forward_slashes(folder: &str) -> String {
    let folder = folder.replace('\\', "/");
    let folder = folder.strip_prefix("//?/").unwrap_or(&folder);
    let mut characters = folder.chars();
    match (characters.next(), characters.next()) {
        (Some(drive), Some(':')) => format!(
            "{}{}",
            drive.to_ascii_uppercase(),
            &folder[drive.len_utf8()..]
        ),
        _ => folder.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &Path, projects: &str) -> std::path::PathBuf {
        let file = dir.join(".claude.json");
        fs::write(&file, format!("{{\"projects\":{projects}}}")).unwrap();
        file
    }

    #[test]
    fn a_folder_or_any_parent_with_accepted_trust_is_not_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let file = config(
            temp.path(),
            r#"{"/work/repo":{"hasTrustDialogAccepted":true},"/work/repo/sub":{"hasTrustDialogAccepted":false}}"#,
        );
        let home = Path::new("/home/u");
        for cwd in ["/work/repo", "/work/repo/sub", "/work/repo/sub/deep"] {
            assert_eq!(untrusted_folder(&file, Path::new(cwd), home), None, "{cwd}");
        }
    }

    #[test]
    fn a_folder_without_accepted_trust_is_blocked_and_named() {
        let temp = tempfile::tempdir().unwrap();
        let file = config(
            temp.path(),
            r#"{"/work/repo":{"hasTrustDialogAccepted":false},"/work/other":{"hasTrustDialogAccepted":true}}"#,
        );
        let home = Path::new("/home/u");
        for cwd in ["/work/repo", "/work", "/elsewhere"] {
            let reason = untrusted_folder(&file, Path::new(cwd), home).expect(cwd);
            assert!(reason.contains(&format!("trust {cwd} ")), "{reason}");
            assert!(reason.contains("user's call"), "{reason}");
        }
    }

    #[test]
    fn backslash_and_forward_slash_records_name_the_same_folder() {
        let temp = tempfile::tempdir().unwrap();
        let file = config(
            temp.path(),
            r#"{"C:/work/repo":{"hasTrustDialogAccepted":true}}"#,
        );
        assert_eq!(forward_slashes(r"C:\work\repo"), "C:/work/repo");
        assert_eq!(forward_slashes(r"\\?\C:\work\repo"), "C:/work/repo");
        assert_eq!(forward_slashes("c:/work/repo"), "C:/work/repo");
        assert_eq!(forward_slashes("/home/u"), "/home/u");
        assert_eq!(
            untrusted_folder(
                &file,
                Path::new(r"\\?\c:\work\repo"),
                Path::new("C:/Users/u")
            ),
            None
        );
        assert_eq!(
            untrusted_folder(
                &file,
                Path::new("C:/work/repo/sub"),
                Path::new("C:/Users/u")
            ),
            None
        );
    }

    #[test]
    fn the_home_folder_is_blocked_with_its_own_reason() {
        let temp = tempfile::tempdir().unwrap();
        let file = config(
            temp.path(),
            r#"{"/home/u/bin":{"hasTrustDialogAccepted":true}}"#,
        );
        let reason = untrusted_folder(&file, Path::new("/home/u"), Path::new("/home/u")).unwrap();
        assert!(
            reason.contains("never remembers trust for the home folder /home/u"),
            "{reason}"
        );
        assert_eq!(
            untrusted_folder(&file, Path::new("/home/u/bin"), Path::new("/home/u")),
            None
        );
    }

    #[test]
    fn an_unreadable_or_foreign_config_never_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let home = Path::new("/home/u");
        let cwd = Path::new("/work");
        assert_eq!(
            untrusted_folder(&temp.path().join("missing.json"), cwd, home),
            None
        );
        let garbage = temp.path().join("garbage.json");
        fs::write(&garbage, "not json").unwrap();
        assert_eq!(untrusted_folder(&garbage, cwd, home), None);
        let no_projects = temp.path().join("bare.json");
        fs::write(&no_projects, "{}").unwrap();
        assert_eq!(untrusted_folder(&no_projects, cwd, home), None);
    }
}
