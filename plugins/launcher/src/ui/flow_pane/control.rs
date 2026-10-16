use gpui::{AppContext as _, AsyncApp, ClipboardItem, Context, WeakEntity, Window};
use qol_config::contract::RowActionSpec;
use qol_plugin_api::launcher_flows::FlowEnter;

use super::super::trace;
use super::super::LauncherView;
use super::{clipboard_item, finishes_flow};
use crate::flow::FlowEntry;

const COPY_ACTION: &str = "copy";

impl LauncherView {
    pub(in crate::ui) fn open_flow(&mut self, entry: FlowEntry, cx: &mut Context<Self>) {
        self.state.enter_flow(entry);
        trace::flow(self, "entered");
        if self.state.flow_preview() {
            if let Some(session) = self.state.flow.as_mut() {
                session.pending = true;
            }
            self.start_flow_fetch(cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn open_flow_for(&mut self, plugin_id: &str, cx: &mut Context<Self>) {
        match self.store.flow_entry(plugin_id) {
            Some(entry) => self.open_flow(entry, cx),
            None => log::warn!("no launcher flow is installed for {plugin_id}"),
        }
    }

    pub(in crate::ui) fn request_flow_detail(&mut self, cx: &mut Context<Self>) {
        if !self.is_showing {
            return;
        }
        let Some(id) = self.state.want_flow_detail() else {
            return;
        };
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        let Some(query) = flow.entry.detail.clone() else {
            return;
        };
        let entry = flow.entry.clone();
        let epoch = flow.epoch;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let asked = id.clone();
                let outcome = async_cx
                    .background_spawn(
                        async move { crate::flow::fetch_detail(&entry, &query, &asked) },
                    )
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let detail = match outcome {
                        Ok(detail) => Some(detail),
                        Err(message) => {
                            log::warn!("flow detail for row {id} failed: {message}");
                            None
                        }
                    };
                    if view.state.accept_flow_detail(epoch, &id, detail) {
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(in crate::ui) fn toggle_flow_sheet(&mut self, cx: &mut Context<Self>) {
        if self.state.toggle_flow_sheet() {
            trace::flow(self, "sheet");
            cx.notify();
        }
    }

    pub(in crate::ui) fn run_flow_row_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let Some(action) = flow.entry.row_actions.get(index) else {
            return;
        };
        let finished = finishes_flow(action);
        if let Err(message) = crate::flow::run_row_action(&flow.entry, action, row) {
            self.state.set_launch_error(message);
            cx.notify();
            return;
        }
        trace::flow(self, "row_action");
        if finished {
            self.hide_to_ghost("flow", window);
            return;
        }
        self.schedule_flow_query(cx);
        cx.notify();
    }

    pub(in crate::ui) fn enter_flow_row(
        &mut self,
        enter: FlowEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match enter {
            FlowEnter::Copy | FlowEnter::Paste => self.copy_flow_row(window, cx),
            FlowEnter::Action(id) => self.run_enter_action(&id, window, cx),
            FlowEnter::Detail => {}
        }
    }

    fn copy_flow_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let copied = match (clipboard_item(row), copy_action(&flow.entry)) {
            (Some(item), _) => {
                cx.write_to_clipboard(item);
                Ok(())
            }
            (None, Some(action)) => crate::flow::run_row_action(&flow.entry, action, row),
            (None, None) if row.spec.masked => Err(format!(
                "{} declares no copy row action for a hidden row",
                flow.entry.title
            )),
            (None, None) => {
                cx.write_to_clipboard(ClipboardItem::new_string(row.title.clone()));
                Ok(())
            }
        };
        if let Err(message) = copied {
            self.state.set_launch_error(message);
            cx.notify();
            return;
        }
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        flow.sheet = false;
        trace::flow(self, "copied");
        self.hide_to_ghost("copy", window);
    }

    fn run_enter_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let action = enter_action(&flow.entry, id);
        if let Err(message) = crate::flow::run_row_action(&flow.entry, &action, row) {
            self.state.set_launch_error(message);
            cx.notify();
            return;
        }
        trace::flow(self, "activated");
        self.hide_to_ghost("flow", window);
    }
}

fn copy_action(entry: &FlowEntry) -> Option<&RowActionSpec> {
    entry
        .row_actions
        .iter()
        .find(|action| action.action == COPY_ACTION)
}

fn enter_action(entry: &FlowEntry, id: &str) -> RowActionSpec {
    entry
        .row_actions
        .iter()
        .find(|action| action.action == id)
        .cloned()
        .unwrap_or_else(|| RowActionSpec {
            action: id.to_string(),
            input: None,
            label: None,
            description: None,
            key: None,
            when: None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_plugin_api::launcher_flows::FlowPanel;

    #[test]
    fn an_enter_action_uses_the_declared_row_action_or_a_bare_one() {
        let declared = RowActionSpec {
            action: "paste".to_string(),
            input: None,
            label: Some("Paste".to_string()),
            description: None,
            key: Some("shift+enter".to_string()),
            when: None,
        };
        let entry = FlowEntry {
            plugin_id: "qol-clipboard".to_string(),
            title: "Clipboard".to_string(),
            prompt: "Search clipboard".to_string(),
            query: "rows".to_string(),
            row_actions: vec![declared.clone()],
            icon: None,
            enter: FlowEnter::Action("paste".to_string()),
            panel: FlowPanel::Preview,
            detail: None,
        };
        assert_eq!(enter_action(&entry, "paste"), declared);
        let bare = enter_action(&entry, "forget");
        assert_eq!(bare.action, "forget");
        assert_eq!(bare.label, None);
        assert_eq!(bare.key, None);
    }

    #[test]
    fn a_row_without_copy_text_copies_through_the_declared_row_action() {
        let copy = RowActionSpec {
            action: "copy".to_string(),
            input: Some(qol_config::contract::IndexMap::from([(
                "id".to_string(),
                "{id}".to_string(),
            )])),
            label: Some("Copy".to_string()),
            description: None,
            key: None,
            when: None,
        };
        let pin = RowActionSpec {
            action: "pin".to_string(),
            key: Some("ctrl+p".to_string()),
            ..copy.clone()
        };
        let mut entry = FlowEntry {
            plugin_id: "qol-clipboard".to_string(),
            title: "Clipboard".to_string(),
            prompt: "Search clipboard".to_string(),
            query: "rows".to_string(),
            row_actions: vec![pin, copy.clone()],
            icon: None,
            enter: FlowEnter::Copy,
            panel: FlowPanel::Preview,
            detail: Some("detail".to_string()),
        };
        let row = crate::flow::FlowRow {
            title: "••••••••".to_string(),
            subtitle: None,
            copy: None,
            raw: serde_json::json!({ "id": "c7", "masked": true }),
            spec: Default::default(),
        };

        assert_eq!(copy_action(&entry), Some(&copy));
        let input = crate::flow::render_action_input(copy_action(&entry).unwrap(), &row);
        assert_eq!(input, serde_json::json!({ "id": "c7" }));
        entry.row_actions.pop();
        assert_eq!(copy_action(&entry), None);
    }
}
