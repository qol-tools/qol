pub mod nav;
pub mod notify;
pub mod placement;
pub mod render;
pub mod run;
pub mod selection;
mod trace;

use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, AsyncApp, Context, FocusHandle, Focusable, WeakEntity};
use qol_terminal_sessions::{SessionBinding, SessionId};

use crate::host::TerminalHost;
use crate::session::registry::Registry;
use crate::ui::selection::Selection;

pub(crate) const WINDOW_TITLE: &str = "cli-sessions-panel";

pub struct SessionsView {
    pub registry: Arc<Mutex<Registry>>,
    pub host: Arc<dyn TerminalHost + Send + Sync>,
    selection: Selection,
    is_showing: bool,
    last_jumped: Option<SessionId>,
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

    pub fn dismiss_with_reason(&mut self, reason: &'static str) -> bool {
        let _scope = qol_gpui::popup_window::reason_scope(reason);
        let hidden = qol_gpui::popup_window::hide_window_by_title(WINDOW_TITLE);
        self.is_showing = false;
        trace::dismiss(reason, hidden);
        hidden
    }

    fn order(&self) -> Vec<SessionId> {
        self.rows().iter().map(|row| row.id.clone()).collect()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn jump_to_session(&mut self, id: SessionId, reason: &'static str, cx: &mut Context<Self>) {
        self.selection.select(id.clone());
        self.last_jumped = Some(id.clone());
        let rows = self.rows();
        let binding = match rows.iter().enumerate().find(|(_, row)| row.id == id) {
            Some((index, row)) => {
                trace::jump_target(reason, index, rows.len(), row);
                let Some(binding) = row.binding() else {
                    trace::jump_missing(reason, index, rows.len());
                    return;
                };
                binding
            }
            None => {
                trace::jump_missing(reason, rows.len(), rows.len());
                return;
            }
        };
        self.focus_session_async(binding, reason, cx);
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
        if let Some(id) = self.selection.resolved(&order) {
            self.jump_to_session(id, "enter", cx);
        }
    }

    fn focus_session_async(
        &self,
        target: SessionBinding,
        reason: &'static str,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        trace::focus_start(reason, target.session_id());
        let result_id = target.session_id().clone();
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { host.focus(&target) })
                    .await;
                trace::focus_result(reason, &result_id, &result);
            }
        })
        .detach();
    }

    pub fn acknowledge(&self, id: &SessionId) {
        if let Ok(mut reg) = self.registry.lock() {
            if let Some(s) = reg.get_mut(id) {
                s.acknowledge();
            }
        }
    }

    pub fn acknowledge_selected(&self) {
        let order = self.order();
        if let Some(id) = self.selection.resolved(&order) {
            self.acknowledge(&id);
        }
    }

    pub fn cycle_implementers(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let order = self.order();
        let driver = self
            .selection
            .resolved(&order)
            .filter(|id| {
                rows.iter()
                    .any(|row| &row.id == id && !row.driving.is_empty())
            })
            .or_else(|| {
                rows.iter()
                    .find(|row| !row.driving.is_empty())
                    .map(|row| row.id.clone())
            });
        if let Some(driver) = driver {
            self.cycle_implementers_of(&driver, forward, cx);
        }
    }

    pub fn cycle_implementers_of(
        &mut self,
        driver: &SessionId,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let driven = self
            .rows()
            .iter()
            .find(|row| &row.id == driver)
            .map(|row| row.driving.clone())
            .unwrap_or_default();
        if driven.is_empty() {
            return;
        }
        let current = self
            .last_jumped
            .as_ref()
            .and_then(|id| driven.iter().position(|d| d == id));
        let index = match (current, forward) {
            (Some(i), true) => (i + 1) % driven.len(),
            (Some(i), false) => (i + driven.len() - 1) % driven.len(),
            (None, true) => 0,
            (None, false) => driven.len() - 1,
        };
        self.jump_to_session(driven[index].clone(), "cycle-implementer", cx);
    }

    pub fn jump_to_next_attention(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let statuses: Vec<crate::session::status::Status> = rows.iter().map(|r| r.status).collect();
        let current = self
            .last_jumped
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id));
        if let Some(id) = crate::ui::nav::next_attention(&statuses, current)
            .and_then(|index| rows.get(index))
            .map(|row| row.id.clone())
        {
            self.jump_to_session(id, "next-attention", cx);
        }
    }
}

impl Focusable for SessionsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
