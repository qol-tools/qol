use crate::platform;
use gpui::{AnyWindowHandle, AppContext, AsyncApp, WeakEntity};
use std::time::Duration;

const ALT_POLL_INTERVAL_MS: u64 = 50;

/// Start a new Alt-key polling task for HoldToSwitch mode.
/// Drops any previous task (which auto-cancels it).
pub(crate) fn start(
    app: &mut super::AltTabApp,
    window_handle: AnyWindowHandle,
    cx: &mut gpui::Context<super::AltTabApp>,
) {
    let list = app.delegate.clone();
    app.alt_was_held = true;
    app._alt_poll_task = Some(cx.spawn(
        move |this: WeakEntity<super::AltTabApp>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                eprintln!("[alt-tab/hold] X11 modifier poll task started");
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(ALT_POLL_INTERVAL_MS))
                        .await;
                    let alt_held = platform::is_modifier_held();

                    if !alt_held {
                        eprintln!(
                            "[alt-tab/hold] X11 poll: Alt released — activating selected"
                        );
                        let list = list.clone();
                        let _ = cx.update_window(window_handle, |_root, window, cx| {
                            list.update(cx, |s, _cx| {
                                s.activate_selected(window);
                            });
                        });
                        break;
                    }
                }

                // Clear the task reference so subsequent Show requests know we're fully closed
                let _ = cx.update(|cx| {
                    if let Some(entity) = this.upgrade() {
                        let _ = entity.update(cx, |app: &mut super::AltTabApp, _cx| {
                            app._alt_poll_task = None;
                        });
                    }
                });

                eprintln!("[alt-tab/hold] X11 modifier poll task ended");
            }
        },
    ));
}
