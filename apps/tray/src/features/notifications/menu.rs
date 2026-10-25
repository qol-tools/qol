use crate::features::MenuProvider;
use crate::plugins::MenuItem as PluginMenuItem;
use anyhow::Result;

const MENU_ITEM_ID: &str = "notifications_mute";

pub struct MuteToggle;

impl MenuProvider for MuteToggle {
    fn menu_items(&self) -> Vec<PluginMenuItem> {
        vec![PluginMenuItem::Checkbox {
            id: MENU_ITEM_ID.to_string(),
            label: "Mute notifications".to_string(),
            checked: !super::enabled(),
            action: crate::plugins::ActionType::Run,
            config_key: None,
        }]
    }

    fn handle_event(&self, event_id: &str) -> Result<()> {
        if !event_id.ends_with(&format!("::{MENU_ITEM_ID}")) {
            return Ok(());
        }
        let enabled = !super::enabled();
        super::set_enabled(enabled)?;
        log::info!(
            "Notifications turned {}.",
            if enabled { "on" } else { "off" }
        );
        Ok(())
    }
}
