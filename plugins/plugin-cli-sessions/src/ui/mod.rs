pub mod render;
pub mod run;
mod trace;

use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, AsyncApp, Context, FocusHandle, Focusable, WeakEntity};

use crate::host::TerminalHost;
use crate::registry::Registry;

pub(crate) const WINDOW_TITLE: &str = "cli-sessions-panel";

pub struct SessionsView {
    pub registry: Arc<Mutex<Registry>>,
    pub host: Arc<dyn TerminalHost + Send + Sync>,
    pub selected: usize,
    pub focus_handle: FocusHandle,
}

impl SessionsView {
    pub fn new(
        registry: Arc<Mutex<Registry>>,
        host: Arc<dyn TerminalHost + Send + Sync>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            registry,
            host,
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn rows(&self) -> Vec<crate::registry::SessionState> {
        self.registry.lock().map(|r| r.sorted()).unwrap_or_default()
    }

    pub fn dismiss(&self) -> bool {
        self.dismiss_with_reason("dismiss")
    }

    pub fn dismiss_with_reason(&self, reason: &'static str) -> bool {
        let _scope = qol_gpui::popup_window::reason_scope(reason);
        let hidden = qol_gpui::popup_window::hide_window_by_title(WINDOW_TITLE);
        trace::dismiss(reason, hidden);
        hidden
    }

    pub fn jump_to(&mut self, index: usize, reason: &'static str, cx: &mut Context<Self>) {
        let rows = self.rows();
        trace::jump_requested(reason, index, rows.len());
        let Some(row) = rows.get(index) else {
            trace::jump_missing(reason, index, rows.len());
            return;
        };

        self.selected = index;
        let window_id = row.window_id;
        trace::jump_target(reason, index, rows.len(), row);
        self.focus_window_async(window_id, reason, cx);
    }

    fn focus_window_async(&self, window_id: u64, reason: &'static str, cx: &mut Context<Self>) {
        let host = self.host.clone();
        trace::focus_start(reason, window_id);
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { host.focus(window_id) })
                    .await;
                trace::focus_result(reason, window_id, &result);
            }
        })
        .detach();
    }

    pub fn acknowledge(&self, window_id: u64) {
        if let Ok(mut reg) = self.registry.lock() {
            if let Some(s) = reg.get_mut(window_id) {
                s.acknowledge();
            }
        }
    }

    pub fn acknowledge_selected(&self) {
        let rows = self.rows();
        if let Some(row) = rows.get(self.selected) {
            self.acknowledge(row.window_id);
        }
    }

    pub fn jump_to_next_attention(&mut self, cx: &mut Context<Self>) {
        let statuses: Vec<crate::status::Status> = self.rows().iter().map(|r| r.status).collect();
        if let Some(i) = crate::nav::next_attention(&statuses, self.selected) {
            self.jump_to(i, "next-attention", cx);
        }
    }
}

impl Focusable for SessionsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
