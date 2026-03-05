use super::HotkeyManager;
use crate::plugins::PluginManager;
use anyhow::Result;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();
const HOTKEY_LOOP_SLEEP_MS: u64 = 10;

pub fn trigger_reload() {
    if let Some(sender) = RELOAD_SENDER.get() {
        let _ = sender.send(());
    }
}

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let (reload_tx, reload_rx) = mpsc::channel::<()>();
    let _ = RELOAD_SENDER.set(reload_tx);
    std::thread::spawn(move || run_listener_loop(reload_rx, plugin_manager));
    Ok(())
}

fn run_listener_loop(reload_rx: mpsc::Receiver<()>, plugin_manager: Arc<Mutex<PluginManager>>) {
    let Ok(mut manager) = HotkeyManager::new() else {
        log::error!("Failed to create hotkey manager");
        return;
    };

    register_initial_hotkeys(&mut manager, &plugin_manager);
    let hotkey_receiver = GlobalHotKeyEvent::receiver();
    loop {
        try_reload_hotkeys(&reload_rx, &mut manager, &plugin_manager);
        try_handle_hotkey(hotkey_receiver, &manager, &plugin_manager);
        std::thread::sleep(std::time::Duration::from_millis(HOTKEY_LOOP_SLEEP_MS));
    }
}

fn register_initial_hotkeys(
    manager: &mut HotkeyManager,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) {
    let Ok(config) = manager.load_config() else {
        return;
    };
    let available_actions = load_available_actions(plugin_manager);
    if let Err(error) = manager.register_hotkeys(&config, &available_actions) {
        log::error!("Failed to register hotkeys: {}", error);
    }
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
    let Ok(config) = manager.load_config() else {
        log::error!("Failed to load hotkey config");
        return;
    };
    let available_actions = load_available_actions(plugin_manager);
    match manager.register_hotkeys(&config, &available_actions) {
        Ok(()) => log::info!("Hotkeys reloaded successfully"),
        Err(error) => log::error!("Failed to register hotkeys: {}", error),
    }
}

fn load_available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> HashMap<String, HashSet<String>> {
    match available_actions(plugin_manager) {
        Ok(actions) => actions,
        Err(error) => {
            log::error!("Failed to resolve available plugin actions: {}", error);
            HashMap::new()
        }
    }
}

fn available_actions(
    plugin_manager: &Arc<Mutex<PluginManager>>,
) -> Result<HashMap<String, HashSet<String>>> {
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

fn try_handle_hotkey(
    receiver: &global_hotkey::GlobalHotKeyEventReceiver,
    manager: &HotkeyManager,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) {
    while let Ok(event) = receiver.try_recv() {
        if event.state != HotKeyState::Pressed {
            continue;
        }
        let Some(action) = manager.get_action(&event) else {
            continue;
        };
        log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);
        crate::plugins::action_executor::execute_action(
            plugin_manager,
            &action.plugin_id,
            &action.action,
        );
    }
}
