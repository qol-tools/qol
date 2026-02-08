mod types;

use crate::paths;
use crate::plugins::{Plugin, PluginManager};
use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use types::KEY_CODE_MAP;
pub use types::{HotkeyAction, HotkeyConfig};

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();
static ACTION_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<u32>>>> = OnceLock::new();

fn get_action_processes() -> &'static Mutex<HashMap<String, Vec<u32>>> {
    ACTION_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(dead_code)]
pub fn kill_plugin_processes(plugin_id: &str) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to lock action processes: {}", e);
            return;
        }
    };

    if let Some(pids) = processes.remove(plugin_id) {
        for pid in pids {
            kill_process(pid, plugin_id);
        }
    }
}

pub fn kill_all_plugin_processes() {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to lock action processes: {}", e);
            return;
        }
    };

    for (plugin_id, pids) in processes.drain() {
        for pid in pids {
            kill_process(pid, &plugin_id);
        }
    }
}

#[cfg(unix)]
fn kill_process(pid: u32, plugin_id: &str) {
    unsafe {
        let pid = pid as i32;
        if libc::kill(pid, 0) == 0 {
            log::info!("Killing action process {} for plugin {}", pid, plugin_id);
            libc::kill(pid, libc::SIGTERM);
            std::thread::sleep(std::time::Duration::from_millis(100));
            if libc::kill(pid, 0) == 0 {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn kill_process(_pid: u32, _plugin_id: &str) {}

fn track_action_process(plugin_id: &str, pid: u32) {
    let mut processes = match get_action_processes().lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    processes
        .entry(plugin_id.to_string())
        .or_default()
        .push(pid);
}

pub fn trigger_reload() {
    if let Some(sender) = RELOAD_SENDER.get() {
        let _ = sender.send(());
    }
}

pub struct HotkeyManager {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        let config_path = paths::hotkeys_path()?;
        Ok(Self {
            manager: None,
            registered: Vec::new(),
            bindings: HashMap::new(),
            config_path,
        })
    }

    pub fn load_config(&self) -> Result<HotkeyConfig> {
        if !self.config_path.exists() {
            return Ok(HotkeyConfig::default());
        }

        let content = std::fs::read_to_string(&self.config_path)?;
        let config: HotkeyConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_config(&self, config: &HotkeyConfig) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(config)?;
        std::fs::write(&self.config_path, content)?;
        Ok(())
    }

    pub fn register_hotkeys(&mut self, config: &HotkeyConfig) -> Result<()> {
        self.unregister_all();

        let new_manager = GlobalHotKeyManager::new()?;

        for binding in &config.hotkeys {
            if !binding.enabled {
                continue;
            }

            let hotkey = match parse_hotkey(&binding.key) {
                Some(hk) => hk,
                None => {
                    log::warn!("Invalid hotkey string: {}", binding.key);
                    continue;
                }
            };

            if let Err(e) = new_manager.register(hotkey) {
                log::error!("Failed to register hotkey {}: {}", binding.key, e);
                continue;
            }

            self.registered.push(hotkey);
            self.bindings.insert(
                hotkey.id(),
                HotkeyAction {
                    plugin_id: binding.plugin_id.clone(),
                    action: binding.action.clone(),
                },
            );

            log::info!(
                "Registered hotkey: {} -> {}::{}",
                binding.key,
                binding.plugin_id,
                binding.action
            );
        }

        self.manager = Some(new_manager);
        Ok(())
    }

    fn unregister_all(&mut self) {
        if let Some(ref manager) = self.manager {
            if !self.registered.is_empty() {
                log::info!("Unregistering {} hotkeys", self.registered.len());
                if let Err(e) = manager.unregister_all(&self.registered) {
                    log::error!("Failed to unregister hotkeys: {}", e);
                }
            }
        }
        self.manager = None;
        self.registered.clear();
        self.bindings.clear();
    }

    pub fn get_action(&self, event: &GlobalHotKeyEvent) -> Option<&HotkeyAction> {
        self.bindings.get(&event.id())
    }
}

fn parse_hotkey(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    let mut key_code = None;

    for part in parts {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" => modifiers |= Modifiers::SUPER,
            key => key_code = parse_key_code(key),
        }
    }

    Some(HotKey::new(Some(modifiers), key_code?))
}

fn parse_key_code(s: &str) -> Option<Code> {
    KEY_CODE_MAP.get(s.to_lowercase().as_str()).copied()
}

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let (reload_tx, reload_rx) = mpsc::channel::<()>();
    let _ = RELOAD_SENDER.set(reload_tx);

    std::thread::spawn(move || {
        let mut manager = match HotkeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                log::error!("Failed to create hotkey manager: {}", e);
                return;
            }
        };

        if let Ok(config) = manager.load_config() {
            if let Err(e) = manager.register_hotkeys(&config) {
                log::error!("Failed to register hotkeys: {}", e);
            }
        }

        let hotkey_receiver = GlobalHotKeyEvent::receiver();
        loop {
            try_reload_hotkeys(&reload_rx, &mut manager);
            try_handle_hotkey(hotkey_receiver, &manager, &plugin_manager);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(())
}

fn try_reload_hotkeys(reload_rx: &mpsc::Receiver<()>, manager: &mut HotkeyManager) {
    if reload_rx.try_recv().is_err() {
        return;
    }

    log::info!("Reloading hotkeys...");
    let config = match manager.load_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to load hotkey config: {}", e);
            return;
        }
    };

    match manager.register_hotkeys(&config) {
        Ok(()) => log::info!("Hotkeys reloaded successfully"),
        Err(e) => log::error!("Failed to register hotkeys: {}", e),
    }
}

struct ResolvedAction {
    plugin_id: String,
    plugin_dir: PathBuf,
    command_path: PathBuf,
    args: Vec<String>,
}

fn try_handle_hotkey(
    receiver: &global_hotkey::GlobalHotKeyEventReceiver,
    manager: &HotkeyManager,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) {
    let event = match receiver.try_recv() {
        Ok(e) if e.state == HotKeyState::Pressed => e,
        _ => return,
    };

    let Some(action) = manager.get_action(&event) else {
        return;
    };
    log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);

    let resolved = {
        let plugins = match plugin_manager.lock() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to lock plugin manager: {}", e);
                return;
            }
        };

        let Some(plugin) = plugins.get(&action.plugin_id) else {
            log::warn!("Plugin not found: {}", action.plugin_id);
            return;
        };

        match resolve_action(plugin, &action.action) {
            Some(r) => r,
            None => return,
        }
    };

    execute_plugin_action(&resolved);
}

fn resolve_action(plugin: &Plugin, action: &str) -> Option<ResolvedAction> {
    if !crate::plugins::manifest::is_valid_action_id(action) {
        log::warn!("Invalid action ID: {:?}", action);
        return None;
    }

    let runtime = plugin.manifest.runtime.as_ref().or_else(|| {
        log::warn!("Plugin {} has no runtime config", plugin.id);
        None
    })?;

    let command = std::path::Path::new(&runtime.command);
    let has_traversal = command.is_absolute()
        || command
            .components()
            .any(|c| c == std::path::Component::ParentDir);
    if has_traversal {
        log::warn!(
            "Plugin {} runtime command escapes plugin directory: {:?}",
            plugin.id,
            runtime.command
        );
        return None;
    }
    let command_path = plugin.path.join(command);

    let args = match &runtime.actions {
        Some(map) => match map.get(action) {
            Some(args) => args.clone(),
            None => {
                log::warn!(
                    "Plugin {} has no action mapping for {:?}",
                    plugin.id,
                    action
                );
                return None;
            }
        },
        None => vec![action.to_string()],
    };

    Some(ResolvedAction {
        plugin_id: plugin.id.clone(),
        plugin_dir: plugin.path.clone(),
        command_path,
        args,
    })
}

fn execute_plugin_action(resolved: &ResolvedAction) {
    log::info!("Executing: {:?} {:?}", resolved.command_path, resolved.args);
    let result = std::process::Command::new(&resolved.command_path)
        .args(&resolved.args)
        .current_dir(&resolved.plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(child) => {
            let pid = child.id();
            track_action_process(&resolved.plugin_id, pid);
            log::info!("Plugin action started (pid: {})", pid);
        }
        Err(e) => log::error!("Failed to execute plugin action: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_code_cases() {
        let valid = [
            ("a", Code::KeyA),
            ("A", Code::KeyA),
            ("z", Code::KeyZ),
            ("Z", Code::KeyZ),
            ("0", Code::Digit0),
            ("5", Code::Digit5),
            ("9", Code::Digit9),
            ("f1", Code::F1),
            ("F1", Code::F1),
            ("f12", Code::F12),
            ("F12", Code::F12),
            ("space", Code::Space),
            ("SPACE", Code::Space),
            ("return", Code::Enter),
            ("enter", Code::Enter),
            ("esc", Code::Escape),
            ("escape", Code::Escape),
            ("tab", Code::Tab),
            ("backspace", Code::Backspace),
            ("delete", Code::Delete),
            ("insert", Code::Insert),
            ("up", Code::ArrowUp),
            ("down", Code::ArrowDown),
            ("left", Code::ArrowLeft),
            ("right", Code::ArrowRight),
            ("home", Code::Home),
            ("end", Code::End),
            ("pageup", Code::PageUp),
            ("pgup", Code::PageUp),
            ("pagedown", Code::PageDown),
            ("pgdn", Code::PageDown),
        ];

        for (input, expected) in valid {
            assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
        }

        let invalid = [
            "unknown", "", "ctrl", "shift", "f0", "f13", "key", " ", "aa",
        ];
        for input in invalid {
            assert_eq!(parse_key_code(input), None, "input: {:?}", input);
        }
    }

    #[test]
    fn parse_hotkey_valid_cases() {
        let cases: &[(&str, Code, Modifiers)] = &[
            ("R", Code::KeyR, Modifiers::empty()),
            ("r", Code::KeyR, Modifiers::empty()),
            ("F1", Code::F1, Modifiers::empty()),
            ("Space", Code::Space, Modifiers::empty()),
            ("Ctrl+R", Code::KeyR, Modifiers::CONTROL),
            ("ctrl+r", Code::KeyR, Modifiers::CONTROL),
            ("CTRL+R", Code::KeyR, Modifiers::CONTROL),
            ("Control+R", Code::KeyR, Modifiers::CONTROL),
            ("Alt+R", Code::KeyR, Modifiers::ALT),
            ("Shift+R", Code::KeyR, Modifiers::SHIFT),
            ("Super+R", Code::KeyR, Modifiers::SUPER),
            ("Win+R", Code::KeyR, Modifiers::SUPER),
            ("Meta+R", Code::KeyR, Modifiers::SUPER),
            ("Cmd+R", Code::KeyR, Modifiers::SUPER),
            (
                "Ctrl+Shift+R",
                Code::KeyR,
                Modifiers::CONTROL | Modifiers::SHIFT,
            ),
            (
                "Ctrl+Alt+R",
                Code::KeyR,
                Modifiers::CONTROL | Modifiers::ALT,
            ),
            (
                "Ctrl+Shift+Alt+R",
                Code::KeyR,
                Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT,
            ),
            (
                "Ctrl+Shift+Alt+Super+R",
                Code::KeyR,
                Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER,
            ),
            ("  Ctrl  +  R  ", Code::KeyR, Modifiers::CONTROL),
            (
                "Ctrl + Shift + R",
                Code::KeyR,
                Modifiers::CONTROL | Modifiers::SHIFT,
            ),
            ("+R", Code::KeyR, Modifiers::empty()),
            ("Ctrl++R", Code::KeyR, Modifiers::CONTROL),
            ("Ctrl+F12", Code::F12, Modifiers::CONTROL),
            ("Alt+Tab", Code::Tab, Modifiers::ALT),
        ];

        for (input, expected_key, expected_mods) in cases {
            let result = parse_hotkey(input);
            assert!(result.is_some(), "input: {:?} should parse", input);
            let hk = result.unwrap();
            assert_eq!(hk.key, *expected_key, "input: {:?} key mismatch", input);
            assert_eq!(hk.mods, *expected_mods, "input: {:?} mods mismatch", input);
        }
    }

    #[test]
    fn parse_hotkey_invalid_cases() {
        let cases = [
            "",
            "   ",
            "+++",
            "Ctrl",
            "Ctrl+",
            "Ctrl+Shift",
            "Ctrl+Shift+",
            "+",
            "++",
            "Ctrl+InvalidKey",
            "Ctrl+Shift+Unknown",
            "NotAKey",
            "Ctrl+Alt+",
            "\t",
            "\n",
        ];

        for input in cases {
            assert!(
                parse_hotkey(input).is_none(),
                "input: {:?} should not parse",
                input
            );
        }
    }

    #[test]
    fn is_valid_action_id_cases() {
        let cases = [
            ("run", true),
            ("toggle-feature", true),
            ("action_name", true),
            ("Action123", true),
            ("a", true),
            ("ABC", true),
            ("a-b-c", true),
            ("a_b_c", true),
            ("123", true),
            ("a1b2c3", true),
            (&"a".repeat(64), true),
            ("", false),
            ("-", false),
            ("--help", false),
            ("-v", false),
            ("-flag", false),
            ("foo bar", false),
            ("foo\tbar", false),
            ("foo;bar", false),
            ("foo&bar", false),
            ("foo|bar", false),
            ("foo>bar", false),
            ("foo<bar", false),
            ("$(whoami)", false),
            ("`whoami`", false),
            ("foo\0bar", false),
            ("foo\nbar", false),
            ("foo/bar", false),
            ("foo\\bar", false),
            (&"a".repeat(65), false),
            ("foo=bar", false),
            ("foo'bar", false),
            ("foo\"bar", false),
        ];

        for (input, expected) in cases {
            assert_eq!(
                crate::plugins::manifest::is_valid_action_id(input),
                expected,
                "input: {:?}",
                input
            );
        }
    }
}
