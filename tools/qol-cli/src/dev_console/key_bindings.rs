use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::emu_panel::emu_detail_shows_warnings;
use super::{Dash, View};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum Action {
    ToggleKeys,
    ToggleArm,
    FeatureFlags,
    Worktrees,
    Rebuild,
    Doctor,
    ToggleTraceDetails,
    ToggleTraceRate,
    Back,
    Activate,
    Dive,
    Quit,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Follow,
    Filter,
    Copy,
    OpenEmuDir,
    RunSandboxFlow,
    DecreaseSandboxFlowLanes,
    IncreaseSandboxFlowLanes,
    OpenCurrentLogFolder,
    OpenCurrentLogEditor,
    OpenCurrentLogRaw,
    VerifySandboxImage,
    Ignore,
}

pub(super) fn preserves_arm(action: Action) -> bool {
    matches!(
        action,
        Action::ScrollUp
            | Action::ScrollDown
            | Action::PageUp
            | Action::PageDown
            | Action::Dive
            | Action::Back
            | Action::ToggleKeys
            | Action::Follow
    )
}

#[derive(Clone, Copy)]
pub(super) struct KeyHint {
    pub(super) key: &'static str,
    pub(super) desc: &'static str,
}

#[derive(Clone, Copy)]
struct KeyStroke {
    code: KeyCode,
    mods: KeyModifiers,
}

impl KeyStroke {
    fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyModifiers::NONE,
        }
    }

    fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: KeyModifiers::CONTROL,
        }
    }

    fn matches(self, code: KeyCode, mods: KeyModifiers) -> bool {
        if self.code != code {
            return false;
        }
        normalized_mods(mods) == self.mods
    }
}

#[derive(Clone)]
pub(super) struct KeyBinding {
    hint: KeyHint,
    action: Action,
    strokes: Vec<KeyStroke>,
}

impl KeyBinding {
    fn matches(&self, code: KeyCode, mods: KeyModifiers) -> bool {
        self.strokes.iter().any(|stroke| stroke.matches(code, mods))
    }
}

fn normalized_mods(mut mods: KeyModifiers) -> KeyModifiers {
    mods.remove(KeyModifiers::SHIFT);
    mods
}

fn binding(
    key: &'static str,
    desc: &'static str,
    action: Action,
    strokes: Vec<KeyStroke>,
) -> KeyBinding {
    KeyBinding {
        hint: KeyHint { key, desc },
        action,
        strokes,
    }
}

fn char_binding(key: &'static str, desc: &'static str, action: Action, c: char) -> KeyBinding {
    let mut strokes = vec![KeyStroke::plain(KeyCode::Char(c))];
    let upper = c.to_ascii_uppercase();
    if upper != c {
        strokes.push(KeyStroke::plain(KeyCode::Char(upper)));
    }
    binding(key, desc, action, strokes)
}

pub(super) fn global_action_bindings(armed: bool) -> Vec<KeyBinding> {
    let ctrl_r_desc = if armed {
        "reload qol dev"
    } else {
        "rebuild tray+plugins"
    };
    vec![
        binding(
            "ctrl+r",
            ctrl_r_desc,
            Action::Rebuild,
            vec![KeyStroke::ctrl('r')],
        ),
        binding(
            "ctrl+k",
            "keys",
            Action::ToggleKeys,
            vec![KeyStroke::ctrl('k')],
        ),
        binding(
            "ctrl+w",
            "worktrees",
            Action::Worktrees,
            vec![KeyStroke::ctrl('w')],
        ),
        binding(
            "ctrl+f",
            "feature flags",
            Action::FeatureFlags,
            vec![KeyStroke::ctrl('f')],
        ),
        binding(
            "ctrl+q",
            "quit (press twice)",
            Action::Quit,
            vec![KeyStroke::ctrl('q')],
        ),
    ]
}

pub(super) fn context_action_bindings(dash: &Dash) -> Vec<KeyBinding> {
    match dash.view {
        View::Dashboard => vec![
            binding(
                "↑/↓",
                "move",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "move",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "act on row",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "→ / ←",
                "dive · back",
                Action::Dive,
                vec![KeyStroke::plain(KeyCode::Right)],
            ),
            binding(
                "→ / ←",
                "dive · back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
            binding(
                "space",
                "arm, then enter",
                Action::ToggleArm,
                vec![KeyStroke::plain(KeyCode::Char(' '))],
            ),
            char_binding("d", "doctor", Action::Doctor, 'd'),
        ],
        View::Emu => vec![
            binding(
                "↑/↓",
                "select sandbox",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "select sandbox",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "run qol dev · stop one",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "→",
                "detail · log",
                Action::Dive,
                vec![KeyStroke::plain(KeyCode::Right)],
            ),
            char_binding("o", "open run folder", Action::OpenEmuDir, 'o'),
            char_binding("r", "run default flow", Action::RunSandboxFlow, 'r'),
            binding(
                "-",
                "fewer flow lanes",
                Action::DecreaseSandboxFlowLanes,
                vec![KeyStroke::plain(KeyCode::Char('-'))],
            ),
            binding(
                "+",
                "more flow lanes",
                Action::IncreaseSandboxFlowLanes,
                vec![
                    KeyStroke::plain(KeyCode::Char('+')),
                    KeyStroke::plain(KeyCode::Char('=')),
                ],
            ),
            char_binding("a", "verify image", Action::VerifySandboxImage, 'a'),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
        View::Logs => stream_view_bindings(false, true, false),
        View::Trace => stream_view_bindings(true, true, dash.trace_rate.is_realtime()),
        View::EmuDetail if emu_detail_shows_warnings(dash) => arrow_view_bindings("scroll"),
        View::EmuDetail => stream_view_bindings(false, false, false),
        View::Doctor => {
            let mut bindings = vec![
                char_binding("d", "refresh checks", Action::Doctor, 'd'),
                binding(
                    "enter",
                    "details",
                    Action::Activate,
                    vec![KeyStroke::plain(KeyCode::Enter)],
                ),
                binding(
                    "space",
                    "raw output",
                    Action::ToggleArm,
                    vec![KeyStroke::plain(KeyCode::Char(' '))],
                ),
                char_binding("c", "copy message", Action::Copy, 'c'),
            ];
            bindings.extend(arrow_view_bindings("move"));
            bindings
        }
        View::Disk => {
            let mut bindings = vec![binding(
                "enter",
                "rescan",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            )];
            bindings.extend(arrow_view_bindings("scroll"));
            bindings
        }
        View::Plugins => vec![
            binding(
                "↑/↓",
                "move",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "move",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "link/unlink",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
        View::Endpoints => arrow_view_bindings("scroll"),
    }
}

fn arrow_view_bindings(vertical: &'static str) -> Vec<KeyBinding> {
    vec![
        binding(
            "↑/↓",
            vertical,
            Action::ScrollUp,
            vec![KeyStroke::plain(KeyCode::Up)],
        ),
        binding(
            "↑/↓",
            vertical,
            Action::ScrollDown,
            vec![KeyStroke::plain(KeyCode::Down)],
        ),
        binding(
            "←",
            "back",
            Action::Back,
            vec![
                KeyStroke::plain(KeyCode::Left),
                KeyStroke::plain(KeyCode::Esc),
            ],
        ),
    ]
}

pub(super) fn stream_view_bindings(
    trace: bool,
    log_resource: bool,
    trace_realtime: bool,
) -> Vec<KeyBinding> {
    let mut bindings = vec![
        binding(
            "↑/↓",
            "scroll",
            Action::ScrollUp,
            vec![KeyStroke::plain(KeyCode::Up)],
        ),
        binding(
            "↑/↓",
            "scroll",
            Action::ScrollDown,
            vec![KeyStroke::plain(KeyCode::Down)],
        ),
        binding(
            "pgup/pgdn",
            "page",
            Action::PageUp,
            vec![KeyStroke::plain(KeyCode::PageUp)],
        ),
        binding(
            "pgup/pgdn",
            "page",
            Action::PageDown,
            vec![KeyStroke::plain(KeyCode::PageDown)],
        ),
        binding(
            "f / end",
            "follow tail",
            Action::Follow,
            vec![
                KeyStroke::plain(KeyCode::Char('f')),
                KeyStroke::plain(KeyCode::Char('F')),
                KeyStroke::plain(KeyCode::End),
            ],
        ),
        binding(
            "/",
            "filter",
            Action::Filter,
            vec![KeyStroke::plain(KeyCode::Char('/'))],
        ),
        char_binding("c", "copy last N", Action::Copy, 'c'),
    ];
    if log_resource {
        bindings.push(char_binding(
            "o",
            "open folder",
            Action::OpenCurrentLogFolder,
            'o',
        ));
        bindings.push(char_binding(
            "e",
            "open in editor",
            Action::OpenCurrentLogEditor,
            'e',
        ));
        if trace {
            bindings.push(char_binding(
                "r",
                "open raw",
                Action::OpenCurrentLogRaw,
                'r',
            ));
        }
    }
    if trace {
        bindings.push(binding(
            "space",
            "arm: reload",
            Action::ToggleArm,
            vec![KeyStroke::plain(KeyCode::Char(' '))],
        ));
        bindings.push(char_binding(
            "d",
            "details",
            Action::ToggleTraceDetails,
            'd',
        ));
        bindings.push(char_binding(
            "s",
            if trace_realtime {
                "rate (realtime)"
            } else {
                "rate (relaxed)"
            },
            Action::ToggleTraceRate,
            's',
        ));
    }
    bindings.push(binding(
        "←",
        "back",
        Action::Back,
        vec![
            KeyStroke::plain(KeyCode::Left),
            KeyStroke::plain(KeyCode::Esc),
        ],
    ));
    bindings
}

pub(super) fn action_for(dash: &Dash, code: KeyCode, mods: KeyModifiers) -> Action {
    global_action_bindings(dash.armed)
        .into_iter()
        .chain(context_action_bindings(dash))
        .find(|binding| binding.matches(code, mods))
        .map(|binding| binding.action)
        .unwrap_or(Action::Ignore)
}

pub(super) fn is_feature_flags_shortcut(code: KeyCode, mods: KeyModifiers) -> bool {
    KeyStroke::ctrl('f').matches(code, mods)
}

pub(super) fn is_worktrees_shortcut(code: KeyCode, mods: KeyModifiers) -> bool {
    KeyStroke::ctrl('w').matches(code, mods)
}

pub(super) fn is_quit_shortcut(code: KeyCode, mods: KeyModifiers) -> bool {
    KeyStroke::ctrl('q').matches(code, mods)
}

pub(super) fn unique_hints(bindings: Vec<KeyBinding>) -> Vec<KeyHint> {
    let mut hints = Vec::new();
    for binding in bindings {
        if hints
            .iter()
            .any(|hint: &KeyHint| hint.key == binding.hint.key && hint.desc == binding.hint.desc)
        {
            continue;
        }
        hints.push(binding.hint);
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::emu_panel::EmuDetail;

    #[test]
    fn action_for_maps_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let mut dash = Dash::new(Vec::new());
        let cases = [
            (KeyCode::Char('l'), none, Action::Ignore),
            (KeyCode::Char('L'), none, Action::Ignore),
            (KeyCode::Char('d'), none, Action::Doctor),
            (KeyCode::Char('D'), none, Action::Doctor),
            (KeyCode::Esc, none, Action::Back),
            (KeyCode::Left, none, Action::Back),
            (KeyCode::Enter, none, Action::Activate),
            (KeyCode::Right, none, Action::Dive),
            (KeyCode::Char('r'), ctrl, Action::Rebuild),
            (KeyCode::Char('f'), ctrl, Action::FeatureFlags),
            (KeyCode::Char('p'), ctrl, Action::Ignore),
            (KeyCode::Char('u'), ctrl, Action::Ignore),
            (KeyCode::Char('c'), ctrl, Action::Ignore),
            (KeyCode::Char('q'), ctrl, Action::Quit),
            (KeyCode::Char('q'), none, Action::Ignore),
            (KeyCode::Up, none, Action::ScrollUp),
            (KeyCode::Down, none, Action::ScrollDown),
            (KeyCode::Char('k'), ctrl, Action::ToggleKeys),
            (KeyCode::Char('k'), none, Action::Ignore),
            (KeyCode::Char('w'), ctrl, Action::Worktrees),
            (KeyCode::Char('w'), none, Action::Ignore),
            (KeyCode::Char(' '), none, Action::ToggleArm),
            (KeyCode::Char('r'), none, Action::Ignore),
            (KeyCode::Char('p'), none, Action::Ignore),
            (KeyCode::Char('x'), none, Action::Ignore),
            (KeyCode::Char('u'), none, Action::Ignore),
        ];
        for (code, mods, expected) in cases {
            assert_eq!(action_for(&dash, code, mods), expected, "{code:?} {mods:?}");
        }
        dash.view = View::Trace;
        assert_eq!(
            action_for(&dash, KeyCode::Char('d'), none),
            Action::ToggleTraceDetails,
            "d toggles trace details in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('o'), none),
            Action::OpenCurrentLogFolder,
            "o opens trace folder in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('e'), none),
            Action::OpenCurrentLogEditor,
            "e opens the prettified trace in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('r'), none),
            Action::OpenCurrentLogRaw,
            "r opens the raw trace file in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char(' '), none),
            Action::ToggleArm,
            "space arms the reload in the trace view"
        );
        for (code, expected) in [
            (KeyCode::PageUp, Action::PageUp),
            (KeyCode::PageDown, Action::PageDown),
            (KeyCode::End, Action::Follow),
            (KeyCode::Char('f'), Action::Follow),
            (KeyCode::Char('/'), Action::Filter),
            (KeyCode::Char('c'), Action::Copy),
            (KeyCode::Char('C'), Action::Copy),
        ] {
            assert_eq!(
                action_for(&dash, code, none),
                expected,
                "stream key: {code:?}"
            );
        }
        dash.view = View::Logs;
        assert_eq!(
            action_for(&dash, KeyCode::Char('d'), none),
            Action::Ignore,
            "d is not doctor outside its owning contexts"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('o'), none),
            Action::OpenCurrentLogFolder,
            "o opens log folder in the logs view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('e'), none),
            Action::OpenCurrentLogEditor,
            "e opens log file in the logs view"
        );
    }

    #[test]
    fn emu_keys_map_flow_lanes_and_verified_import() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        let cases = [
            (KeyCode::Char('o'), Action::OpenEmuDir),
            (KeyCode::Char('r'), Action::RunSandboxFlow),
            (KeyCode::Char('-'), Action::DecreaseSandboxFlowLanes),
            (KeyCode::Char('+'), Action::IncreaseSandboxFlowLanes),
            (KeyCode::Char('='), Action::IncreaseSandboxFlowLanes),
            (KeyCode::Char('a'), Action::VerifySandboxImage),
        ];
        for (code, expected) in cases {
            assert_eq!(
                action_for(&dash, code, KeyModifiers::NONE),
                expected,
                "code: {code:?}"
            );
        }
        assert_eq!(
            action_for(&dash, KeyCode::Char('+'), KeyModifiers::SHIFT),
            Action::IncreaseSandboxFlowLanes
        );
        let hints = unique_hints(context_action_bindings(&dash));
        assert!(hints.iter().any(|hint| hint.key == "-"));
        assert!(hints.iter().any(|hint| hint.key == "+"));
        assert!(hints
            .iter()
            .any(|hint| hint.key == "a" && hint.desc == "verify image"));
        assert_eq!(
            action_for(&dash, KeyCode::Char('t'), KeyModifiers::NONE),
            Action::Ignore,
            "the removed Dash-only architecture toggle must stay unwired"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char(' '), KeyModifiers::NONE),
            Action::Ignore,
            "sandboxes do not advertise an unwired armed action"
        );
    }

    #[test]
    fn cleanup_history_detail_reuses_scroll_only_bindings() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::EmuDetail;
        dash.emu_detail = Some(EmuDetail {
            id: "linux/mint".to_string(),
            info: Vec::new(),
            warnings: vec![ratatui::text::Line::from("cleanup warning")],
            replay: None,
        });

        assert_eq!(
            action_for(&dash, KeyCode::Up, KeyModifiers::NONE),
            Action::ScrollUp
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('/'), KeyModifiers::NONE),
            Action::Ignore
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('c'), KeyModifiers::NONE),
            Action::Ignore
        );
    }
}
