use crate::features::MenuProvider;
use crate::mode::{ModeConfig, ModeFlag};
use crate::plugins::MenuItem as PluginMenuItem;
use anyhow::Result;

const MENU_ITEM_ID: &str = "mode_toggle";

pub struct ModeToggle;

impl Default for ModeToggle {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeToggle {
    pub fn new() -> Self {
        Self
    }
}

impl MenuProvider for ModeToggle {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        let is_dev = ModeConfig::load().map(|c| c.is_dev()).unwrap_or(false);
        let label = if is_dev {
            "Mode: dev (active)".to_string()
        } else {
            "Mode: prod".to_string()
        };
        vec![PluginMenuItem::Checkbox {
            id: MENU_ITEM_ID.to_string(),
            label,
            checked: is_dev,
            action: crate::plugins::ActionType::Run,
            config_key: None,
        }]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        if !event_id.ends_with(&format!("::{}", MENU_ITEM_ID)) {
            return Ok(());
        }
        let current = ModeConfig::load().unwrap_or_default();
        let next = if current.is_dev() {
            ModeFlag::Prod
        } else {
            ModeFlag::Dev
        };
        ModeConfig::set(next)?;
        log::info!(
            "Runtime mode flipped to {next:?}. Restart qol-tray for the menu label to refresh."
        );
        Ok(())
    }
}
