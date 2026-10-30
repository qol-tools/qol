use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub(super) const NAME: &str = "qol-switchboard";
const FOLDER: &str = "plugin";
const MANIFEST: &str = r#"{
  "name": "qol-switchboard",
  "version": "1.0.0",
  "description": "Starts each switchboard fork in the project folder named on the first line of its prompt."
}
"#;
const HOOKS: &str = r#"{
  "modules": ["./register.ts"]
}
"#;
const MODULE: &str = r#"import type { Register } from 'claude-code'

const PROJECT = /^\s*Project:\s*(.+?)\s*$/

export const register: Register = on => {
  on('agent.spawn', { subagentType: 'project-fork' }, ($, e, next) => {
    if (e.resume !== undefined) return next(e)
    const project = PROJECT.exec(e.prompt.split('\n', 1)[0] ?? '')
    if (project === null) {
      return { deny: "A project-fork's prompt must start with the line `Project: <absolute path of an existing project folder>`." }
    }
    return next({ ...e, cwd: project[1] })
  }).catch(($, e, next) =>
    next.called ? next(e) : { deny: 'The qol-switchboard plugin could not start this fork in its project folder.' },
  )
}
"#;

pub(super) fn dir(switchboard: &Path) -> PathBuf {
    switchboard.join(FOLDER)
}

fn files() -> [(&'static str, &'static str); 3] {
    [
        (".claude-plugin/plugin.json", MANIFEST),
        ("hooks/hooks.json", HOOKS),
        ("hooks/register.ts", MODULE),
    ]
}

pub(super) fn write(dir: &Path) -> Result<()> {
    for (relative, contents) in files() {
        let path = dir.join(relative);
        if fs::read_to_string(&path).is_ok_and(|current| current == contents) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        qol_fs::atomic_write(&path, contents.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_lives_in_the_switchboard_folder_and_names_itself() {
        assert_eq!(
            dir(Path::new("/data/sessions/switchboard")),
            Path::new("/data/sessions/switchboard/plugin")
        );
        let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(manifest["name"], NAME);
        let hooks: serde_json::Value = serde_json::from_str(HOOKS).unwrap();
        assert_eq!(hooks["modules"], serde_json::json!(["./register.ts"]));
    }

    #[test]
    fn the_module_places_new_project_forks_and_leaves_resumes_alone() {
        for needle in [
            "on('agent.spawn', { subagentType: 'project-fork' }",
            "if (e.resume !== undefined) return next(e)",
            "next({ ...e, cwd: project[1] })",
            "next.called ? next(e) : { deny:",
            "const PROJECT = /^\\s*Project:\\s*(.+?)\\s*$/",
        ] {
            assert!(MODULE.contains(needle), "{needle}");
        }
    }

    #[test]
    fn writing_creates_every_file_replaces_a_stale_one_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let plugin = dir(root.path());
        fs::create_dir_all(plugin.join("hooks")).unwrap();
        fs::write(plugin.join("hooks/register.ts"), "stale").unwrap();
        let types = plugin.join(".claude-plugin/types/claude-code/index.d.ts");
        fs::create_dir_all(types.parent().unwrap()).unwrap();
        fs::write(&types, "laid by Claude Code").unwrap();
        write(&plugin).unwrap();
        for (relative, contents) in files() {
            assert_eq!(
                fs::read_to_string(plugin.join(relative)).unwrap(),
                contents,
                "{relative}"
            );
        }
        assert_eq!(fs::read_to_string(&types).unwrap(), "laid by Claude Code");
        let written = fs::metadata(plugin.join("hooks/register.ts"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        write(&plugin).unwrap();
        let again = fs::metadata(plugin.join("hooks/register.ts"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(written, again, "an unchanged file is not rewritten");
    }
}
