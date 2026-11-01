use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub(super) const NAME: &str = "qol-switchboard";
pub(super) const BOARD_TOOL: &str = "mcp__qol-switchboard__fork_board";
pub(super) const BOARD_COMMAND: &str = "/board";
const FOLDER: &str = "plugin";
const MANIFEST: &str = r#"{
  "name": "qol-switchboard",
  "version": "1.7.0",
  "description": "Starts each switchboard fork in its project folder, files each fork's final report on the fork board beside the chat without a switchboard turn, runs each fork's pull request watcher under CLI Sessions and gives each fork its watcher's result.",
  "types": "./types/index.d.ts"
}
"#;
const HOOKS: &str = r#"{
  "modules": ["./register.tsx"]
}
"#;
const MODULE: &str = include_str!("register.tsx");
const STATE_TYPES: &str = include_str!("types.d.ts");
const MODULE_TESTS: &str = include_str!("register.test.tsx");
const RETIRED: [&str; 1] = ["hooks/register.ts"];

pub(super) fn dir(switchboard: &Path) -> PathBuf {
    switchboard.join(FOLDER)
}

fn files() -> [(&'static str, &'static str); 5] {
    [
        (".claude-plugin/plugin.json", MANIFEST),
        ("hooks/hooks.json", HOOKS),
        ("hooks/register.tsx", MODULE),
        ("types/index.d.ts", STATE_TYPES),
        ("tests/register.test.tsx", MODULE_TESTS),
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
    for relative in RETIRED {
        let path = dir.join(relative);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to remove {}", path.display()))
            }
        }
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
        assert_eq!(hooks["modules"], serde_json::json!(["./register.tsx"]));
        assert_eq!(manifest["types"], "./types/index.d.ts");
        assert!(MODULE.contains("from '../types'"));
        assert!(STATE_TYPES.contains("'qol-switchboard': { board: Shaped<Entry[]>"));
    }

    #[test]
    fn the_module_places_new_project_forks_and_leaves_resumes_alone() {
        for needle in [
            "const FORK = 'project-fork'",
            "on('agent.spawn', { subagentType: FORK }, ($, e, next) => {",
            "if (e.resume !== undefined) return next(e)",
            "next({ ...e, cwd: project[1], prompt: withLauncher(e.prompt, e.parentAgentId) })",
            "on('session.send', async ($, e, next) => {",
            "mayMessage(await read($, board), e.agentId, e.to) ? next(e) : { isDelivered: false, reason: FORK_TO_FORK }",
            "next.called ? next(e) : { deny:",
            "const PROJECT = /^\\s*Project:\\s*(.+?)\\s*$/",
        ] {
            assert!(MODULE.contains(needle), "{needle}");
        }
    }

    #[test]
    fn a_message_to_a_stopped_fork_continues_it_as_a_new_fork_the_board_links() {
        for needle in [
            "on('tool.call', { tool: SEND }, async ($, e, next) => {",
            "if (!isUndelivered(answer)) return answer",
            "const takeover = await takeOver($, target, e.message)",
            "$.env.get('QOL_SWITCHBOARD_CONVERSATIONS')",
            "if (stopped !== undefined) await succeed($, stopped, agentId)",
            "const stopped = isNew && e.parentAgentId === undefined ? stoppedFork(e.prompt) : undefined",
        ] {
            assert!(MODULE.contains(needle), "{needle}");
        }
        let sentence = "The message above is for the fork ";
        let taken = ", which was stopped, so you take over its work and its conversation.";
        assert!(MODULE.contains(sentence) && MODULE.contains(taken));
        assert!(super::super::SWITCHBOARD_ROLE.contains(sentence));
        assert!(super::super::SWITCHBOARD_ROLE.contains(taken));
        assert!(MODULE_TESTS.contains("reaches it as a new fork that reads its conversation"));
    }

    #[test]
    fn the_module_files_fork_reports_on_the_board_and_serves_its_tool_and_command() {
        for needle in [
            "on('session.append', async ($, e, next) => {",
            "(origin !== 'peer' && origin !== 'task-notification')) return next(e)",
            "const HANDBACK = /^(Another Claude session sent a message:\\n<agent-message from=",
            "const NOTIFICATION = /^<task-notification>\\n[\\s\\S]*<\\/task-notification>$/",
            "if (agentId === undefined || !isFork(agentId)) return undefined",
            "Send \"{entry.recommend}\" to {keyName(entry.name)}?",
            "const KINDS: readonly Kind[] = ['ask', 'test', 'failed', 'blocked', 'waiting', 'merged', 'note']",
            "const boardLine = (entry: Entry): string => `[fork board] ${keyName(entry.name)}: ${entry.headline}`",
            "headline: headlineOf(report)",
            "model: 'haiku'",
            "on('ui.render', { component: 'Pane', requestId: PANE }",
            "$.ui.view({ agentId: entry.agentId })",
            "const text = entry.recommend ?? ''",
            "$.session.send({ to: { agentId: entry.agentId }, text })",
            "on('ui.render', { component: 'AbovePrompt' }, ($, e, next) => {",
            "if (docks) void $.ui.open({ id: PANE, title: TITLE })",
            "const TYPED_TOGGLE = /^\\s*qols\\s+board\\s*$/i",
            "on('turn.start', async ($, e, next) => {",
        ] {
            assert!(MODULE.contains(needle), "{needle}");
        }
        assert!(MODULE.contains(&format!("const TOOL_NAME = '{BOARD_TOOL}'")));
        let tool = BOARD_TOOL.strip_prefix(&format!("mcp__{NAME}__")).unwrap();
        assert!(MODULE.contains(&format!("const TOOL = '{tool}'")));
        let command = BOARD_COMMAND.strip_prefix('/').unwrap();
        assert!(MODULE.contains(&format!("const PANE = '{command}'")));
        assert!(MODULE_TESTS.contains(BOARD_TOOL));
        for retired in ["Recommend(?:ed)?", "<kind>: <headline>", "const HEADER"] {
            assert!(!MODULE.contains(retired), "{retired}");
        }
    }

    #[test]
    fn the_module_runs_fork_watchers_under_cli_sessions_and_wakes_no_fork_at_start() {
        for needle in [
            "on('tool.call', { tool: 'Bash' }, async ($, e, next) => {",
            "e.run_in_background !== true",
            "'fork-watch', '--agent', agentId, '--conversation', conversation",
            "$.process.run([program, 'fork-watch', '--deliver']",
            "$.env.get('QOL_CLI_SESSIONS')",
            "$.env.get('QOL_SWITCHBOARD_WATCHES')",
            "'open · CLI Sessions watching' : 'open · no watcher'",
        ] {
            assert!(MODULE.contains(needle), "{needle}");
        }
        assert!(!MODULE.contains("QOL_SWITCHBOARD_WAKE"));
        assert!(STATE_TYPES.contains("pulls: Pull[]"));
        assert!(MODULE_TESTS.contains("runs under CLI Sessions, and no other command is rewritten"));
    }

    #[test]
    fn writing_creates_every_file_replaces_a_stale_one_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let plugin = dir(root.path());
        fs::create_dir_all(plugin.join("hooks")).unwrap();
        fs::write(plugin.join("hooks/register.ts"), "retired").unwrap();
        fs::write(plugin.join("hooks/register.tsx"), "stale").unwrap();
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
        assert!(!plugin.join("hooks/register.ts").exists());
        let written = fs::metadata(plugin.join("hooks/register.tsx"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        write(&plugin).unwrap();
        let again = fs::metadata(plugin.join("hooks/register.tsx"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(written, again, "an unchanged file is not rewritten");
    }
}
