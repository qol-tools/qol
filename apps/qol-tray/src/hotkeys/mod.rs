use anyhow::Result;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
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

fn parse_key_code(s: &str) -> Option<Code> {
    match s.to_lowercase().as_str() {
        "a" => Some(Code::KeyA),
        "b" => Some(Code::KeyB),
        "c" => Some(Code::KeyC),
        "d" => Some(Code::KeyD),
        "e" => Some(Code::KeyE),
        "f" => Some(Code::KeyF),
        "g" => Some(Code::KeyG),
        "h" => Some(Code::KeyH),
        "i" => Some(Code::KeyI),
        "j" => Some(Code::KeyJ),
        "k" => Some(Code::KeyK),
        "l" => Some(Code::KeyL),
        "m" => Some(Code::KeyM),
        "n" => Some(Code::KeyN),
        "o" => Some(Code::KeyO),
        "p" => Some(Code::KeyP),
        "q" => Some(Code::KeyQ),
        "r" => Some(Code::KeyR),
        "s" => Some(Code::KeyS),
        "t" => Some(Code::KeyT),
        "u" => Some(Code::KeyU),
        "v" => Some(Code::KeyV),
        "w" => Some(Code::KeyW),
        "x" => Some(Code::KeyX),
        "y" => Some(Code::KeyY),
        "z" => Some(Code::KeyZ),
        "0" => Some(Code::Digit0),
        "1" => Some(Code::Digit1),
        "2" => Some(Code::Digit2),
        "3" => Some(Code::Digit3),
        "4" => Some(Code::Digit4),
        "5" => Some(Code::Digit5),
        "6" => Some(Code::Digit6),
        "7" => Some(Code::Digit7),
        "8" => Some(Code::Digit8),
        "9" => Some(Code::Digit9),
        "f1" => Some(Code::F1),
        "f2" => Some(Code::F2),
        "f3" => Some(Code::F3),
        "f4" => Some(Code::F4),
        "f5" => Some(Code::F5),
        "f6" => Some(Code::F6),
        "f7" => Some(Code::F7),
        "f8" => Some(Code::F8),
        "f9" => Some(Code::F9),
        "f10" => Some(Code::F10),
        "f11" => Some(Code::F11),
        "f12" => Some(Code::F12),
        "space" => Some(Code::Space),
        "enter" | "return" => Some(Code::Enter),
        "escape" | "esc" => Some(Code::Escape),
        "tab" => Some(Code::Tab),
        "backspace" => Some(Code::Backspace),
        "delete" | "del" => Some(Code::Delete),
        "insert" | "ins" => Some(Code::Insert),
        "home" => Some(Code::Home),
        "end" => Some(Code::End),
        "pageup" | "pgup" => Some(Code::PageUp),
        "pagedown" | "pgdn" => Some(Code::PageDown),
        "up" => Some(Code::ArrowUp),
        "down" => Some(Code::ArrowDown),
        "left" => Some(Code::ArrowLeft),
        "right" => Some(Code::ArrowRight),
        "printscreen" | "print" | "prtsc" => Some(Code::PrintScreen),
        "pause" => Some(Code::Pause),
        _ => None,
    }
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
    
    if action == "run" {
        let script_path = plugin_dir.join("run.sh");
        if script_path.exists() {
            log::info!("Executing: {:?}", script_path);
            match std::process::Command::new("bash")
                .arg(&script_path)
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
}

