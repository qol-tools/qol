use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
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

pub struct HotkeyManager {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;
        let config_path = Self::config_path()?;

        Ok(Self {
            manager,
            registered: Vec::new(),
            bindings: HashMap::new(),
            config_path,
        })
    }

    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
            .join("qol-tray");
        Ok(config_dir.join("hotkeys.json"))
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

            if let Err(e) = self.manager.register(hotkey) {
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

            log::info!("Registered hotkey: {} -> {}::{}", binding.key, binding.plugin_id, binding.action);
        }

        Ok(())
    }

    fn unregister_all(&mut self) {
        if !self.registered.is_empty() {
            let _ = self.manager.unregister_all(&self.registered);
            self.registered.clear();
        }
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
            key => {
                key_code = parse_key_code(key);
            }
        }
    }

    let code = key_code?;
    Some(HotKey::new(Some(modifiers), code))
}

static KEY_CODE_MAP: Lazy<HashMap<&'static str, Code>> = Lazy::new(|| {
    HashMap::from([
        ("a", Code::KeyA), ("b", Code::KeyB), ("c", Code::KeyC), ("d", Code::KeyD),
        ("e", Code::KeyE), ("f", Code::KeyF), ("g", Code::KeyG), ("h", Code::KeyH),
        ("i", Code::KeyI), ("j", Code::KeyJ), ("k", Code::KeyK), ("l", Code::KeyL),
        ("m", Code::KeyM), ("n", Code::KeyN), ("o", Code::KeyO), ("p", Code::KeyP),
        ("q", Code::KeyQ), ("r", Code::KeyR), ("s", Code::KeyS), ("t", Code::KeyT),
        ("u", Code::KeyU), ("v", Code::KeyV), ("w", Code::KeyW), ("x", Code::KeyX),
        ("y", Code::KeyY), ("z", Code::KeyZ),
        ("0", Code::Digit0), ("1", Code::Digit1), ("2", Code::Digit2), ("3", Code::Digit3),
        ("4", Code::Digit4), ("5", Code::Digit5), ("6", Code::Digit6), ("7", Code::Digit7),
        ("8", Code::Digit8), ("9", Code::Digit9),
        ("f1", Code::F1), ("f2", Code::F2), ("f3", Code::F3), ("f4", Code::F4),
        ("f5", Code::F5), ("f6", Code::F6), ("f7", Code::F7), ("f8", Code::F8),
        ("f9", Code::F9), ("f10", Code::F10), ("f11", Code::F11), ("f12", Code::F12),
        ("space", Code::Space), ("enter", Code::Enter), ("return", Code::Enter),
        ("escape", Code::Escape), ("esc", Code::Escape), ("tab", Code::Tab),
        ("backspace", Code::Backspace), ("delete", Code::Delete), ("del", Code::Delete),
        ("insert", Code::Insert), ("ins", Code::Insert), ("home", Code::Home),
        ("end", Code::End), ("pageup", Code::PageUp), ("pgup", Code::PageUp),
        ("pagedown", Code::PageDown), ("pgdn", Code::PageDown),
        ("up", Code::ArrowUp), ("down", Code::ArrowDown),
        ("left", Code::ArrowLeft), ("right", Code::ArrowRight),
        ("printscreen", Code::PrintScreen), ("print", Code::PrintScreen), ("prtsc", Code::PrintScreen),
        ("pause", Code::Pause),
    ])
});

fn parse_key_code(s: &str) -> Option<Code> {
    KEY_CODE_MAP.get(s.to_lowercase().as_str()).copied()
}

pub fn start_hotkey_listener(
    plugins_dir: PathBuf,
) -> Result<()> {
    let mut manager = HotkeyManager::new()?;
    let config = manager.load_config()?;
    manager.register_hotkeys(&config)?;

    let receiver = GlobalHotKeyEvent::receiver();

    std::thread::spawn(move || {
        loop {
            if let Ok(event) = receiver.try_recv() {
                if let Some(action) = manager.get_action(&event) {
                    log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);
                    execute_plugin_action(&plugins_dir, &action.plugin_id, &action.action);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(())
}

fn execute_plugin_action(plugins_dir: &PathBuf, plugin_id: &str, action: &str) {
    let plugin_dir = plugins_dir.join(plugin_id);
    let script_path = plugin_dir.join("run.sh");

    if script_path.exists() {
        log::info!("Executing: {:?} with action {}", script_path, action);
        match std::process::Command::new("bash")
            .arg(&script_path)
            .arg(action)
            .current_dir(&plugin_dir)
            .spawn()
        {
            Ok(_) => log::info!("Plugin action started"),
            Err(e) => log::error!("Failed to execute plugin action: {}", e),
        }
    } else {
        log::warn!("Plugin script not found: {:?}", script_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_code_returns_letter_codes() {
        // Arrange
        let inputs = ["a", "A", "z", "Z"];
        let expected = [Code::KeyA, Code::KeyA, Code::KeyZ, Code::KeyZ];

        // Act & Assert
        for (input, exp) in inputs.iter().zip(expected.iter()) {
            assert_eq!(parse_key_code(input), Some(*exp));
        }
    }

    #[test]
    fn parse_key_code_returns_digit_codes() {
        // Arrange
        let input = "5";

        // Act
        let result = parse_key_code(input);

        // Assert
        assert_eq!(result, Some(Code::Digit5));
    }

    #[test]
    fn parse_key_code_returns_function_keys() {
        // Arrange
        let inputs = ["f1", "F12"];
        let expected = [Code::F1, Code::F12];

        // Act & Assert
        for (input, exp) in inputs.iter().zip(expected.iter()) {
            assert_eq!(parse_key_code(input), Some(*exp));
        }
    }

    #[test]
    fn parse_key_code_returns_special_keys() {
        // Arrange
        let inputs = ["space", "enter", "return", "escape", "esc", "tab"];
        let expected = [Code::Space, Code::Enter, Code::Enter, Code::Escape, Code::Escape, Code::Tab];

        // Act & Assert
        for (input, exp) in inputs.iter().zip(expected.iter()) {
            assert_eq!(parse_key_code(input), Some(*exp));
        }
    }

    #[test]
    fn parse_key_code_returns_navigation_keys() {
        // Arrange
        let inputs = ["up", "down", "left", "right", "home", "end", "pageup", "pgdn"];
        let expected = [
            Code::ArrowUp, Code::ArrowDown, Code::ArrowLeft, Code::ArrowRight,
            Code::Home, Code::End, Code::PageUp, Code::PageDown
        ];

        // Act & Assert
        for (input, exp) in inputs.iter().zip(expected.iter()) {
            assert_eq!(parse_key_code(input), Some(*exp));
        }
    }

    #[test]
    fn parse_key_code_returns_none_for_unknown() {
        // Arrange
        let input = "unknown";

        // Act
        let result = parse_key_code(input);

        // Assert
        assert_eq!(result, None);
    }

    #[test]
    fn parse_hotkey_parses_single_key() {
        // Arrange
        let input = "R";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_some());
        let hotkey = result.unwrap();
        assert_eq!(hotkey.key, Code::KeyR);
    }

    #[test]
    fn parse_hotkey_parses_ctrl_modifier() {
        // Arrange
        let input = "Ctrl+R";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_some());
        let hotkey = result.unwrap();
        assert_eq!(hotkey.key, Code::KeyR);
        assert!(hotkey.mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn parse_hotkey_parses_multiple_modifiers() {
        // Arrange
        let input = "Ctrl+Shift+Alt+R";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_some());
        let hotkey = result.unwrap();
        assert_eq!(hotkey.key, Code::KeyR);
        assert!(hotkey.mods.contains(Modifiers::CONTROL));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
        assert!(hotkey.mods.contains(Modifiers::ALT));
    }

    #[test]
    fn parse_hotkey_parses_super_modifier_variants() {
        // Arrange
        let inputs = ["Super+R", "Win+R", "Meta+R", "Cmd+R"];

        // Act & Assert
        for input in inputs {
            let result = parse_hotkey(input);
            assert!(result.is_some());
            let hotkey = result.unwrap();
            assert!(hotkey.mods.contains(Modifiers::SUPER));
        }
    }

    #[test]
    fn parse_hotkey_handles_whitespace() {
        // Arrange
        let input = "Ctrl + Shift + R";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_some());
        let hotkey = result.unwrap();
        assert_eq!(hotkey.key, Code::KeyR);
        assert!(hotkey.mods.contains(Modifiers::CONTROL));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn parse_hotkey_returns_none_for_empty() {
        // Arrange
        let input = "";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_hotkey_returns_none_for_invalid_key() {
        // Arrange
        let input = "Ctrl+InvalidKey";

        // Act
        let result = parse_hotkey(input);

        // Assert
        assert!(result.is_none());
    }

    #[test]
    fn parse_hotkey_is_case_insensitive() {
        // Arrange
        let inputs = ["ctrl+r", "CTRL+R", "Ctrl+r", "CTRL+r"];

        // Act & Assert
        for input in inputs {
            let result = parse_hotkey(input);
            assert!(result.is_some());
            let hotkey = result.unwrap();
            assert_eq!(hotkey.key, Code::KeyR);
            assert!(hotkey.mods.contains(Modifiers::CONTROL));
        }
    }
}
