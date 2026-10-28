pub(crate) mod capture_status;
pub(crate) mod controls;
pub(crate) mod editor;
pub(crate) mod pinned;
pub(crate) mod preview;
pub(crate) mod region_selector;
pub(crate) mod settings_panel;
pub(crate) mod shortcuts;

use std::sync::Arc;
use std::time::Duration;

const PARKED_REVEAL_MAX_WAIT: Duration = Duration::from_millis(500);

pub(crate) fn release_image(image: Option<Arc<gpui::RenderImage>>, window: &mut gpui::Window) {
    let Some(image) = image else {
        return;
    };
    let _ = window.drop_image(image);
    window.refresh();
}

pub(crate) fn schedule_parked_reveal(title: &str, cx: &mut gpui::App) -> bool {
    if !qol_gpui::popup_window::prepare_window_reveal_by_title(title) {
        return false;
    }
    let title = title.to_string();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(PARKED_REVEAL_MAX_WAIT).await;
        let visible = cx
            .update(|_| qol_gpui::popup_window::visible_windows_by_title_prefix(&title) > 0)
            .unwrap_or(true);
        if visible {
            return;
        }
        let _ = cx.update(|_| qol_gpui::popup_window::hide_invisible(&title));
        qol_runtime::probe!("SHOT_PARKED_REVEAL", "title={title} state=timed-out");
    })
    .detach();
    true
}
