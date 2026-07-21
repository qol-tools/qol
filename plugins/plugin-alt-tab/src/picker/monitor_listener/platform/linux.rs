use qol_gpui::protocol::{RuntimeEvent, RuntimeEventKind};

pub(crate) fn data_refresh_listener_loop() {
    let client = qol_gpui::PlatformStateClient::from_env();
    let Some(mut subscription) = client.subscribe(vec![
        RuntimeEventKind::WindowListChanged,
        RuntimeEventKind::FocusChanged,
    ]) else {
        return;
    };
    while let Some(event) = subscription.next_event() {
        match event {
            RuntimeEvent::FocusChanged { .. } => {
                super::super::request_previous_frontmost_preview_refresh()
            }
            RuntimeEvent::CursorMoved { .. }
            | RuntimeEvent::ActiveMonitorChanged { .. }
            | RuntimeEvent::MonitorsChanged { .. }
            | RuntimeEvent::LauncherAppsSynced { .. }
            | RuntimeEvent::WindowListChanged => super::super::request_data_refresh(),
        }
    }
}
