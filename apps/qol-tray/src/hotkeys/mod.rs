mod types;

use crate::paths;
use crate::plugins::PluginManager;
use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use types::KEY_CODE_MAP;
pub use types::{HotkeyAction, HotkeyConfig};

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();

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

    pub fn register_hotkeys(
        &mut self,
        config: &HotkeyConfig,
        available_actions: &HashMap<String, HashSet<String>>,
    ) -> Result<()> {
        self.unregister_all();

        let new_manager = GlobalHotKeyManager::new()?;

        for binding in &config.hotkeys {
            if !binding.enabled {
                continue;
            }

            if !is_binding_available(available_actions, &binding.plugin_id, &binding.action) {
                log::warn!(
                    "Skipping hotkey {} -> {}::{} (plugin/action unavailable)",
                    binding.key,
                    binding.plugin_id,
                    binding.action
                );
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
            let available_actions = match available_actions(&plugin_manager) {
                Ok(actions) => actions,
                Err(e) => {
                    log::error!("Failed to resolve available plugin actions: {}", e);
                    HashMap::new()
                }
            };
            if let Err(e) = manager.register_hotkeys(&config, &available_actions) {
                log::error!("Failed to register hotkeys: {}", e);
            }
        }

        let hotkey_receiver = GlobalHotKeyEvent::receiver();
        loop {
            try_reload_hotkeys(&reload_rx, &mut manager, &plugin_manager);
            try_handle_hotkey(hotkey_receiver, &manager, &plugin_manager);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(())
}

fn try_reload_hotkeys(
    reload_rx: &mpsc::Receiver<()>,
    manager: &mut HotkeyManager,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) {
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

    let available_actions = match available_actions(plugin_manager) {
        Ok(actions) => actions,
        Err(e) => {
            log::error!("Failed to resolve available plugin actions: {}", e);
            return;
        }
    };

    match manager.register_hotkeys(&config, &available_actions) {
        Ok(()) => log::info!("Hotkeys reloaded successfully"),
        Err(e) => log::error!("Failed to register hotkeys: {}", e),
    }
}

fn available_actions(plugin_manager: &Arc<Mutex<PluginManager>>) -> Result<HashMap<String, HashSet<String>>> {
    let manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock failed"))?;

    let mut actions_by_plugin = HashMap::new();
    for plugin in manager.plugins() {
        let mut action_ids = HashSet::new();
        collect_action_ids(&plugin.manifest.menu.items, &mut action_ids);
        actions_by_plugin.insert(plugin.id.clone(), action_ids);
    }

    Ok(actions_by_plugin)
}

fn collect_action_ids(items: &[crate::plugins::MenuItem], action_ids: &mut HashSet<String>) {
    let mut collect = |item: &crate::plugins::MenuItem| match item {
        crate::plugins::MenuItem::Action { id, .. }
        | crate::plugins::MenuItem::Checkbox { id, .. } => {
            action_ids.insert(id.clone());
        }
        crate::plugins::MenuItem::Separator | crate::plugins::MenuItem::Submenu { .. } => {}
    };
    crate::plugins::manifest::walk_menu_items(items, &mut collect);
}

fn is_binding_available(
    available_actions: &HashMap<String, HashSet<String>>,
    plugin_id: &str,
    action_id: &str,
) -> bool {
    available_actions
        .get(plugin_id)
        .is_some_and(|actions| actions.contains(action_id))
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

    crate::plugins::action_executor::execute_action(
        plugin_manager,
        &action.plugin_id,
        &action.action,
    );
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
