use crate::menu::router::EventRouter;
use anyhow::Result;
use tray_icon::{TrayIconBuilder, TrayIcon, Icon, menu::Menu};

pub fn create_tray(menu: Menu, _router: EventRouter, icon: Icon) -> Result<TrayIcon> {
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    Ok(tray_icon)
}
