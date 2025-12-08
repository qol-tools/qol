use global_hotkey::hotkey::Code;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HotkeyConfig {
    #[serde(default)]
    pub hotkeys: Vec<HotkeyBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub id: String,
    pub key: String,
    pub plugin_id: String,
    pub action: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HotkeyAction {
    pub plugin_id: String,
    pub action: String,
}

pub struct ScriptInfo {
    pub shell: &'static str,
    pub flag: Option<&'static str>,
    pub path: PathBuf,
}

pub const SCRIPT_RUNNERS: &[(&str, &str, Option<&str>)] = &[
    #[cfg(windows)]
    ("run.bat", "cmd", Some("/c")),
    #[cfg(windows)]
    ("run.ps1", "powershell", Some("-File")),
    #[cfg(not(windows))]
    ("run.sh", "bash", None),
];

pub static KEY_CODE_MAP: Lazy<HashMap<&'static str, Code>> = Lazy::new(|| {
    HashMap::from([
        ("a", Code::KeyA),
        ("b", Code::KeyB),
        ("c", Code::KeyC),
        ("d", Code::KeyD),
        ("e", Code::KeyE),
        ("f", Code::KeyF),
        ("g", Code::KeyG),
        ("h", Code::KeyH),
        ("i", Code::KeyI),
        ("j", Code::KeyJ),
        ("k", Code::KeyK),
        ("l", Code::KeyL),
        ("m", Code::KeyM),
        ("n", Code::KeyN),
        ("o", Code::KeyO),
        ("p", Code::KeyP),
        ("q", Code::KeyQ),
        ("r", Code::KeyR),
        ("s", Code::KeyS),
        ("t", Code::KeyT),
        ("u", Code::KeyU),
        ("v", Code::KeyV),
        ("w", Code::KeyW),
        ("x", Code::KeyX),
        ("y", Code::KeyY),
        ("z", Code::KeyZ),
        ("0", Code::Digit0),
        ("1", Code::Digit1),
        ("2", Code::Digit2),
        ("3", Code::Digit3),
        ("4", Code::Digit4),
        ("5", Code::Digit5),
        ("6", Code::Digit6),
        ("7", Code::Digit7),
        ("8", Code::Digit8),
        ("9", Code::Digit9),
        ("f1", Code::F1),
        ("f2", Code::F2),
        ("f3", Code::F3),
        ("f4", Code::F4),
        ("f5", Code::F5),
        ("f6", Code::F6),
        ("f7", Code::F7),
        ("f8", Code::F8),
        ("f9", Code::F9),
        ("f10", Code::F10),
        ("f11", Code::F11),
        ("f12", Code::F12),
        ("space", Code::Space),
        ("enter", Code::Enter),
        ("return", Code::Enter),
        ("escape", Code::Escape),
        ("esc", Code::Escape),
        ("tab", Code::Tab),
        ("backspace", Code::Backspace),
        ("delete", Code::Delete),
        ("del", Code::Delete),
        ("insert", Code::Insert),
        ("ins", Code::Insert),
        ("home", Code::Home),
        ("end", Code::End),
        ("pageup", Code::PageUp),
        ("pgup", Code::PageUp),
        ("pagedown", Code::PageDown),
        ("pgdn", Code::PageDown),
        ("up", Code::ArrowUp),
        ("down", Code::ArrowDown),
        ("left", Code::ArrowLeft),
        ("right", Code::ArrowRight),
        ("printscreen", Code::PrintScreen),
        ("print", Code::PrintScreen),
        ("prtsc", Code::PrintScreen),
        ("pause", Code::Pause),
    ])
});
