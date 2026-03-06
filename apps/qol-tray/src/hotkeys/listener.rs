use super::HotkeyManager;
use crate::plugins::PluginManager;
use anyhow::Result;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, HotKeyState};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();
const HOTKEY_LOOP_SLEEP_MS: u64 = 10;

type AvailableActions = HashMap<String, HashSet<String>>;
type SharedPluginManager = Arc<Mutex<PluginManager>>;

pub fn trigger_reload() {
    if let Some(sender) = RELOAD_SENDER.get() {
        let _ = sender.send(());
    }
}

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let runtime = HotkeyListenerRuntime::new(plugin_manager);
    std::thread::spawn(move || runtime.run());
    Ok(())
}

struct HotkeyListenerRuntime {
    plugin_manager: SharedPluginManager,
    reload_rx: Receiver<()>,
}

impl HotkeyListenerRuntime {
    fn new(plugin_manager: SharedPluginManager) -> Self {
        Self {
            plugin_manager,
            reload_rx: install_reload_channel(),
        }
    }

    fn run(self) {
        let Ok(manager) = HotkeyManager::new() else {
            log::error!("Failed to create hotkey manager");
            return;
        };

        HotkeyListenerLoop {
            manager,
            plugin_manager: self.plugin_manager,
            reload_rx: self.reload_rx,
        }
        .run();
    }
}

struct HotkeyListenerLoop {
    manager: HotkeyManager,
    plugin_manager: SharedPluginManager,
    reload_rx: Receiver<()>,
}

impl HotkeyListenerLoop {
    fn register_initial_hotkeys(&mut self) {
        if let Err(error) = self.reload_hotkeys() {
            log::error!("Failed to register hotkeys: {}", error);
        }
    }

    fn reload_hotkeys(&mut self) -> Result<()> {
        let config = self.manager.load_config()?;
        let available_actions = load_available_actions(&self.plugin_manager);
        self.manager.register_hotkeys(&config, &available_actions)
    }

    fn run(mut self) {
        self.register_initial_hotkeys();
        let hotkey_receiver = GlobalHotKeyEvent::receiver();

        loop {
            self.try_reload_hotkeys();
            self.try_handle_hotkeys(hotkey_receiver);
            sleep_between_polls();
        }
    }

    fn try_handle_hotkeys(&self, receiver: &GlobalHotKeyEventReceiver) {
        while let Ok(event) = receiver.try_recv() {
            self.handle_hotkey_event(event);
        }
    }

    fn try_reload_hotkeys(&mut self) {
        if !reload_requested(&self.reload_rx) {
            return;
        }

        log::info!("Reloading hotkeys...");

        match self.reload_hotkeys() {
            Ok(()) => log::info!("Hotkeys reloaded successfully"),
            Err(error) => log::error!("Failed to register hotkeys: {}", error),
        }
    }

    fn handle_hotkey_event(&self, event: GlobalHotKeyEvent) {
        if event.state != HotKeyState::Pressed {
            return;
        }

        let Some(action) = self.manager.get_action(&event) else {
            return;
        };

        log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);

        crate::plugins::action_executor::execute_action(
            &self.plugin_manager,
            &action.plugin_id,
            &action.action,
        );
    }
}

fn available_actions(plugin_manager: &SharedPluginManager) -> Result<AvailableActions> {
    let manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock failed"))?;
    let mut actions_by_plugin = HashMap::new();

    for plugin in manager.plugins() {
        actions_by_plugin.insert(
            plugin.id.clone(),
            collect_action_ids(&plugin.manifest.menu.items),
        );
    }

    Ok(actions_by_plugin)
}

fn collect_action_ids(items: &[crate::plugins::MenuItem]) -> HashSet<String> {
    let mut action_ids = HashSet::new();
    let mut collect = |item: &crate::plugins::MenuItem| match item {
        crate::plugins::MenuItem::Action { id, .. }
        | crate::plugins::MenuItem::Checkbox { id, .. } => {
            action_ids.insert(id.clone());
        }
        crate::plugins::MenuItem::Separator | crate::plugins::MenuItem::Submenu { .. } => {}
    };

    crate::plugins::manifest::walk_menu_items(items, &mut collect);
    action_ids
}

fn install_reload_channel() -> Receiver<()> {
    let (reload_tx, reload_rx) = mpsc::channel::<()>();
    let _ = RELOAD_SENDER.set(reload_tx);
    reload_rx
}

fn load_available_actions(plugin_manager: &SharedPluginManager) -> AvailableActions {
    match available_actions(plugin_manager) {
        Ok(actions) => actions,
        Err(error) => {
            log::error!("Failed to resolve available plugin actions: {}", error);
            HashMap::new()
        }
    }
}

fn reload_requested(reload_rx: &Receiver<()>) -> bool {
    let mut requested = false;

    while reload_rx.try_recv().is_ok() {
        requested = true;
    }

    requested
}

fn sleep_between_polls() {
    std::thread::sleep(std::time::Duration::from_millis(HOTKEY_LOOP_SLEEP_MS));
}
