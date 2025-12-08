mod types;

use crate::paths;
use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;

pub use types::{HotkeyAction, HotkeyConfig};
use types::{ScriptInfo, KEY_CODE_MAP, SCRIPT_RUNNERS};

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

pub fn start_hotkey_listener(plugins_dir: PathBuf) -> Result<()> {
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
            try_handle_hotkey(hotkey_receiver, &manager, &plugins_dir);
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

fn try_handle_hotkey(
    receiver: &global_hotkey::GlobalHotKeyEventReceiver,
    manager: &HotkeyManager,
    plugins_dir: &PathBuf,
) {
    let event = match receiver.try_recv() {
        Ok(e) if e.state == HotKeyState::Pressed => e,
        _ => return,
    };

    let Some(action) = manager.get_action(&event) else {
        return;
    };
    log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);
    execute_plugin_action(plugins_dir, &action.plugin_id, &action.action);
}

fn execute_plugin_action(plugins_dir: &PathBuf, plugin_id: &str, action: &str) {
    let plugin_dir = plugins_dir.join(plugin_id);
    let Some(script) = find_plugin_script(&plugin_dir) else {
        log::warn!("No plugin script found in {:?}", plugin_dir);
        return;
    };

    log::info!("Executing: {:?} {}", script.path, action);
    let mut cmd = std::process::Command::new(script.shell);
    if let Some(flag) = script.flag {
        cmd.arg(flag);
    }
    let result = cmd
        .arg(&script.path)
        .arg(action)
        .current_dir(&plugin_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_) => log::info!("Plugin action started"),
        Err(e) => log::error!("Failed to execute plugin action: {}", e),
    }
}

fn find_plugin_script(plugin_dir: &std::path::Path) -> Option<ScriptInfo> {
    SCRIPT_RUNNERS.iter().find_map(|(file, shell, flag)| {
        let path = plugin_dir.join(file);
        path.exists().then_some(ScriptInfo {
            shell,
            flag: *flag,
            path,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_code_maps_keys_correctly() {
        let cases = [
            ("a", Code::KeyA),
            ("A", Code::KeyA),
            ("z", Code::KeyZ),
            ("5", Code::Digit5),
            ("f1", Code::F1),
            ("F12", Code::F12),
            ("space", Code::Space),
            ("return", Code::Enter),
            ("esc", Code::Escape),
            ("up", Code::ArrowUp),
            ("down", Code::ArrowDown),
            ("left", Code::ArrowLeft),
            ("right", Code::ArrowRight),
            ("home", Code::Home),
            ("end", Code::End),
            ("pageup", Code::PageUp),
            ("pgdn", Code::PageDown),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
        }
    }

    #[test]
    fn parse_key_code_returns_none_for_unknown() {
        assert_eq!(parse_key_code("unknown"), None);
    }

    #[test]
    fn parse_hotkey_parses_single_key() {
        let result = parse_hotkey("R").unwrap();
        assert_eq!(result.key, Code::KeyR);
    }

    #[test]
    fn parse_hotkey_parses_modifiers() {
        let result = parse_hotkey("Ctrl+Shift+Alt+R").unwrap();
        assert_eq!(result.key, Code::KeyR);
        assert!(result.mods.contains(Modifiers::CONTROL));
        assert!(result.mods.contains(Modifiers::SHIFT));
        assert!(result.mods.contains(Modifiers::ALT));
    }

    #[test]
    fn parse_hotkey_maps_super_aliases() {
        for alias in ["Super+R", "Win+R", "Meta+R", "Cmd+R"] {
            let result = parse_hotkey(alias).unwrap();
            assert!(result.mods.contains(Modifiers::SUPER), "alias: {}", alias);
        }
    }

    #[test]
    fn parse_hotkey_handles_whitespace() {
        let result = parse_hotkey("Ctrl + Shift + R").unwrap();
        assert_eq!(result.key, Code::KeyR);
        assert!(result.mods.contains(Modifiers::CONTROL));
        assert!(result.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn parse_hotkey_is_case_insensitive() {
        for input in ["ctrl+r", "CTRL+R", "Ctrl+r"] {
            let result = parse_hotkey(input).unwrap();
            assert_eq!(result.key, Code::KeyR, "input: {}", input);
            assert!(result.mods.contains(Modifiers::CONTROL), "input: {}", input);
        }
    }

    #[test]
    fn parse_hotkey_returns_none_for_invalid() {
        let cases = [
            "",
            "Ctrl+InvalidKey",
            "Ctrl+",
            "+++",
            "Ctrl+Shift",
            "Ctrl+Shift+",
            "   ",
        ];

        for input in cases {
            assert!(parse_hotkey(input).is_none(), "input: {:?}", input);
        }
    }

    #[test]
    fn parse_hotkey_ignores_empty_parts() {
        let result = parse_hotkey("+R").unwrap();
        assert_eq!(result.key, Code::KeyR);

        let result = parse_hotkey("Ctrl++R").unwrap();
        assert_eq!(result.key, Code::KeyR);
        assert!(result.mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn parse_hotkey_handles_edge_cases() {
        let cases = [
            ("r", Some(Code::KeyR)),
            ("R", Some(Code::KeyR)),
            ("Ctrl+r", Some(Code::KeyR)),
            ("ctrl+R", Some(Code::KeyR)),
            ("Control+R", Some(Code::KeyR)),
            ("  Ctrl  +  R  ", Some(Code::KeyR)),
        ];

        for (input, expected_key) in cases {
            let result = parse_hotkey(input);
            match expected_key {
                Some(key) => {
                    assert!(result.is_some(), "input: {:?} should parse", input);
                    assert_eq!(result.unwrap().key, key, "input: {:?}", input);
                }
                None => assert!(result.is_none(), "input: {:?} should not parse", input),
            }
        }
    }
}
