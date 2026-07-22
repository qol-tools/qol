pub mod nav;
pub mod notify;
pub mod placement;
pub mod render;
pub mod run;
pub mod selection;
mod trace;

use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, AsyncApp, Context, FocusHandle, Focusable, WeakEntity};

use crate::host::TerminalHost;
use crate::session::registry::Registry;
use crate::ui::selection::Selection;

pub(crate) const WINDOW_TITLE: &str = "cli-sessions-panel";

pub struct SessionsView {
    pub registry: Arc<Mutex<Registry>>,
    pub host: Arc<dyn TerminalHost + Send + Sync>,
    selection: Selection,
    is_showing: bool,
    last_jumped: Option<u64>,
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
            selection: Selection::default(),
            is_showing: true,
            last_jumped: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn rows(&self) -> Vec<crate::session::registry::SessionState> {
        self.registry.lock().map(|r| r.sorted()).unwrap_or_default()
    }

    pub fn is_showing(&self) -> bool {
        self.is_showing
    }

    pub fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
    }

    pub fn dismiss(&mut self) -> bool {
        self.dismiss_with_reason("dismiss")
    }

    pub fn dismiss_with_reason(&mut self, reason: &'static str) -> bool {
        let _scope = qol_gpui::popup_window::reason_scope(reason);
        let hidden = qol_gpui::popup_window::hide_window_by_title(WINDOW_TITLE);
        self.is_showing = false;
        trace::dismiss(reason, hidden);
        hidden
    }

    fn order(&self) -> Vec<u64> {
        self.rows().iter().map(|row| row.window_id).collect()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Focus a session by identity. Selecting and focusing by `window_id`
    /// (never by row index) is what keeps a click on the session the user
    /// actually clicked, no matter how the attention sort has since reordered
    /// the rows beneath the cursor.
    pub fn jump_to_window(&mut self, window_id: u64, reason: &'static str, cx: &mut Context<Self>) {
        self.selection.select(window_id);
        self.last_jumped = Some(window_id);
        let rows = self.rows();
        match rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.window_id == window_id)
        {
            Some((index, row)) => trace::jump_target(reason, index, rows.len(), row),
            None => trace::jump_missing(reason, rows.len(), rows.len()),
        }
        self.focus_window_async(window_id, reason, cx);
    }

    pub fn move_selection_down(&mut self) {
        let order = self.order();
        self.selection.move_down(&order);
    }

    pub fn move_selection_up(&mut self) {
        let order = self.order();
        self.selection.move_up(&order);
    }

    pub fn focus_selected(&mut self, cx: &mut Context<Self>) {
        let order = self.order();
        if let Some(window_id) = self.selection.resolved(&order) {
            self.jump_to_window(window_id, "enter", cx);
        }
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
        let order = self.order();
        if let Some(window_id) = self.selection.resolved(&order) {
            self.acknowledge(window_id);
        }
    }

    pub fn jump_to_next_attention(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let statuses: Vec<crate::session::status::Status> = rows.iter().map(|r| r.status).collect();
        let current = self
            .last_jumped
            .and_then(|wid| rows.iter().position(|r| r.window_id == wid));
        if let Some(window_id) = crate::ui::nav::next_attention(&statuses, current)
            .and_then(|index| rows.get(index))
            .map(|row| row.window_id)
        {
            self.jump_to_window(window_id, "next-attention", cx);
        }
    }
}

impl Focusable for SessionsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
