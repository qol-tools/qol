use crate::features::plugin_manager::PluginManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tray_icon::{TrayIconBuilder, Icon};
use gtk::{self, glib};

pub fn create_tray(plugin_manager: Arc<Mutex<PluginManager>>, icon: Icon) -> Result<()> {
    std::thread::spawn(move || {
        if gtk::init().is_err() {
            log::error!("Failed to initialize GTK");
            return;
        }

        let menu = crate::features::tray::menu::build_menu(&plugin_manager);

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("QoL Tray")
            .with_icon(icon)
            .build();

        let tray_icon = match tray_icon {
            Ok(icon) => icon,
            Err(e) => {
                log::error!("Failed to create tray icon: {}", e);
                return;
            }
        };

        setup_event_loop(plugin_manager);
        std::mem::forget(tray_icon);
        gtk::main();
    });

    Ok(())
}

fn setup_event_loop(plugin_manager: Arc<Mutex<PluginManager>>) {
    use tray_icon::menu::MenuEvent;
    use crate::features::tray::menu::handle_menu_event;

    let menu_receiver = MenuEvent::receiver();

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        while let Ok(event) = menu_receiver.try_recv() {
            let event_id = event.id.0.clone();
            log::debug!("Menu event: {}", event_id);

            if let Err(e) = handle_menu_event(&plugin_manager, &event_id) {
                log::error!("Error handling menu event: {}", e);
            }
        }
        glib::ControlFlow::Continue
    });
}
