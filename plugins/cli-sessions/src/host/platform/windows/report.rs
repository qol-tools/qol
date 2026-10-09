use std::collections::{BTreeSet, HashMap};

use qol_app_icon::ProcessEntry;
use qol_terminal_sessions::{BackendId, SessionCapabilities, SessionFacts, SessionId};
use serde::{Deserialize, Serialize};

const TERMINAL_HOSTS: &[&str] = &["alacritty", "wezterm-gui", "windowsterminal"];
const CONSOLE_SERVERS: &[&str] = &["conhost", "openconsole"];
const SHELLS: &[&str] = &[
    "bash",
    "cmd",
    "fish",
    "nu",
    "powershell",
    "pwsh",
    "wsl",
    "zsh",
];
const SCRIPT_HOSTS: &[&str] = &["bun", "deno", "node"];
const SCRIPT_TOOLS: &[(&str, &str)] = &[
    ("@anthropic-ai/claude-code/", "claude"),
    ("@openai/codex/", "codex"),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProcessDetail {
    pub(super) pid: i32,
    pub(super) cwd: Option<String>,
    pub(super) command_line: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ConsoleReport {
    pub(super) root: i32,
    pub(super) attached: Vec<i32>,
    pub(super) title: String,
    pub(super) window: u64,
    pub(super) window_visible: bool,
    pub(super) screen: String,
    pub(super) viewport_current: bool,
    pub(super) processes: Vec<ProcessDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Root {
    pub(super) pid: i32,
    pub(super) hosted: bool,
}

pub(super) fn image_stem(name: &str) -> String {
    let lower = name.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

pub(super) fn candidate_roots(table: &[ProcessEntry], own_pid: i32) -> Vec<Root> {
    let stems: HashMap<i32, String> = table
        .iter()
        .map(|process| (process.pid, image_stem(&process.name)))
        .collect();
    let is = |pid: i32, names: &[&str]| {
        stems
            .get(&pid)
            .is_some_and(|stem| names.contains(&stem.as_str()))
    };
    let mut roots = BTreeSet::new();
    for process in table {
        let eligible = |pid: i32| {
            pid > 0 && pid != own_pid && !is(pid, TERMINAL_HOSTS) && !is(pid, CONSOLE_SERVERS)
        };
        if is(process.parent_pid, TERMINAL_HOSTS) && eligible(process.pid) {
            roots.insert(process.pid);
        }
    }
    let hosted: BTreeSet<i32> = roots.clone();
    for process in table {
        let parent = process.parent_pid;
        let eligible = parent > 0
            && parent != own_pid
            && stems.contains_key(&parent)
            && !is(parent, TERMINAL_HOSTS)
            && !is(parent, CONSOLE_SERVERS);
        if is(process.pid, CONSOLE_SERVERS) && eligible {
            roots.insert(parent);
        }
    }
    roots
        .into_iter()
        .map(|pid| Root {
            pid,
            hosted: hosted.contains(&pid),
        })
        .collect()
}

pub(super) fn foreground_order(report: &ConsoleReport, table: &[ProcessEntry]) -> Vec<i32> {
    let attached: BTreeSet<i32> = report.attached.iter().copied().collect();
    let mut order = vec![report.root];
    let mut index = 0;
    while index < order.len() {
        let parent = order[index];
        let mut children: Vec<i32> = table
            .iter()
            .filter(|process| process.parent_pid == parent && attached.contains(&process.pid))
            .map(|process| process.pid)
            .filter(|pid| !order.contains(pid))
            .collect();
        children.sort_unstable();
        order.extend(children);
        index += 1;
    }
    let mut rest: Vec<i32> = attached
        .into_iter()
        .filter(|pid| !order.contains(pid))
        .collect();
    rest.sort_unstable();
    order.extend(rest);
    order
}

pub(super) fn process_basename(image: &str, command_line: Option<&str>) -> String {
    let stem = image_stem(image);
    if !SCRIPT_HOSTS.contains(&stem.as_str()) {
        return stem;
    }
    let command = command_line
        .unwrap_or_default()
        .replace('\\', "/")
        .to_lowercase();
    SCRIPT_TOOLS
        .iter()
        .find(|(package, _)| command.contains(package))
        .map_or(stem, |(_, tool)| (*tool).to_string())
}

fn usable_cwd(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches('\\');
    let cwd = if trimmed.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    };
    (!trimmed.is_empty() && !cwd.chars().any(char::is_control)).then_some(cwd)
}

pub(super) fn session_facts(
    backend: &BackendId,
    native_id: &str,
    report: &ConsoleReport,
    table: &[ProcessEntry],
) -> Option<SessionFacts> {
    let names: HashMap<i32, &str> = table
        .iter()
        .map(|process| (process.pid, process.name.as_str()))
        .collect();
    let details: HashMap<i32, &ProcessDetail> = report
        .processes
        .iter()
        .map(|detail| (detail.pid, detail))
        .collect();
    let foreground = foreground_order(report, table);
    let basenames: Vec<String> = foreground
        .iter()
        .map(|pid| {
            process_basename(
                names.get(pid).copied().unwrap_or_default(),
                details
                    .get(pid)
                    .and_then(|detail| detail.command_line.as_deref()),
            )
        })
        .collect();
    let root_is_shell = basenames
        .first()
        .is_some_and(|name| SHELLS.contains(&name.as_str()));
    let at_prompt = root_is_shell && foreground.len() == 1;
    let reported_cmd = (!at_prompt)
        .then(|| {
            basenames
                .iter()
                .rev()
                .find(|name| !SHELLS.contains(&name.as_str()))
                .cloned()
        })
        .flatten();
    let cwd = foreground
        .iter()
        .rev()
        .find_map(|pid| {
            details
                .get(pid)
                .and_then(|detail| detail.cwd.as_deref())
                .and_then(usable_cwd)
        })
        .unwrap_or_default();
    Some(SessionFacts {
        id: SessionId::new(backend.clone(), native_id).ok()?,
        root_pid: report.root,
        cwd,
        title: report.title.trim().to_string(),
        at_prompt,
        reported_cmd,
        foreground_basenames: basenames,
        foreground_pids: foreground,
        capabilities: SessionCapabilities::SCREEN_READING | SessionCapabilities::FOCUS,
        spawn_identity: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: i32, parent_pid: i32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            name: name.to_string(),
        }
    }

    fn table() -> Vec<ProcessEntry> {
        vec![
            process(4, 0, "System"),
            process(100, 4, "explorer.exe"),
            process(200, 100, "WindowsTerminal.exe"),
            process(201, 200, "OpenConsole.exe"),
            process(202, 200, "pwsh.exe"),
            process(203, 202, "node.exe"),
            process(204, 203, "git.exe"),
            process(300, 100, "cmd.exe"),
            process(301, 300, "conhost.exe"),
            process(400, 1, "qol-cli-sessions.exe"),
            process(401, 400, "conhost.exe"),
        ]
    }

    fn report(root: i32, attached: &[i32]) -> ConsoleReport {
        ConsoleReport {
            root,
            attached: attached.to_vec(),
            title: " Claude Code ".into(),
            processes: vec![
                ProcessDetail {
                    pid: 202,
                    cwd: Some(r"C:\work\shell\".into()),
                    command_line: None,
                },
                ProcessDetail {
                    pid: 203,
                    cwd: Some(r"C:\work\project\".into()),
                    command_line: Some(
                        r#"node C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js"#
                            .into(),
                    ),
                },
            ],
            ..ConsoleReport::default()
        }
    }

    #[test]
    fn roots_are_terminal_children_and_classic_console_owners() {
        let roots = candidate_roots(&table(), 400);
        assert_eq!(
            roots,
            vec![
                Root {
                    pid: 202,
                    hosted: true
                },
                Root {
                    pid: 300,
                    hosted: false
                },
            ]
        );
    }

    #[test]
    fn basenames_strip_exe_and_name_script_hosted_tools() {
        let cases = [
            ("Claude.EXE", None, "claude"),
            ("codex.exe", None, "codex"),
            (
                "node.exe",
                Some(r"node C:\npm\node_modules\@anthropic-ai\claude-code\cli.js"),
                "claude",
            ),
            (
                "node.exe",
                Some("node /usr/lib/node_modules/@openai/codex/bin/codex.js"),
                "codex",
            ),
            ("node.exe", Some("node server.js"), "node"),
            ("pwsh.exe", Some("@anthropic-ai/claude-code/"), "pwsh"),
        ];
        for (image, command_line, expected) in cases {
            assert_eq!(process_basename(image, command_line), expected, "{image}");
        }
    }

    #[test]
    fn foreground_starts_at_the_root_and_follows_the_attached_tree() {
        let order = foreground_order(&report(202, &[204, 202, 203, 999]), &table());
        assert_eq!(order, vec![202, 203, 204, 999]);
    }

    #[test]
    fn session_facts_describe_the_running_tool() {
        let backend = BackendId::new("console").unwrap();
        let busy = session_facts(&backend, "202-1", &report(202, &[202, 203]), &table()).unwrap();
        assert_eq!(busy.foreground_basenames, ["pwsh", "claude"]);
        assert_eq!(busy.reported_cmd.as_deref(), Some("claude"));
        assert!(!busy.at_prompt);
        assert_eq!(busy.cwd, r"C:\work\project");
        assert_eq!(busy.title, "Claude Code");
        assert!(!busy.capabilities.contains(SessionCapabilities::TEXT_INPUT));

        let idle = session_facts(&backend, "202-1", &report(202, &[202]), &table()).unwrap();
        assert!(idle.at_prompt);
        assert_eq!(idle.reported_cmd, None);
        assert_eq!(idle.cwd, r"C:\work\shell");
    }

    #[test]
    fn drive_roots_keep_their_separator() {
        let cases = [
            (r"C:\", Some(r"C:\")),
            (r"C:\work\", Some(r"C:\work")),
            ("", None),
            ("C:\\bad\u{1}", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(usable_cwd(raw).as_deref(), expected, "{raw:?}");
        }
    }
}
