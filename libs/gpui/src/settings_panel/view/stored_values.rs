use gpui::*;

use super::super::persistence::{load_values, panel_base};
use super::super::rows::{row_query_names, row_value_json, rows_from_resolved};
use super::SettingsPanelView;

impl SettingsPanelView {
    pub(in crate::settings_panel) fn reload_stored_values(&mut self, cx: &mut Context<Self>) {
        let generation = self.values_generation;
        let reads: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| source.pages.is_none() && !source.copy.is_empty())
            .map(|(index, source)| (index, panel_base(&source.plugin_id), source.path.clone()))
            .collect();
        if reads.is_empty() {
            return;
        }
        let (sender, answer) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let values: Vec<_> = reads
                .into_iter()
                .map(|(index, base, path)| (index, load_values(&base, path.as_deref())))
                .collect();
            let _ = sender.send(values);
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let Ok(values) = answer.await else {
                    return;
                };
                let _ = this.update(&mut async_cx, |this, cx| {
                    if this.values_generation != generation
                        || this.stack.len() > 1
                        || this.root().active_control.is_some()
                    {
                        return;
                    }
                    let changed = values.into_iter().fold(false, |changed, (index, values)| {
                        this.take_stored_values(index, values) || changed
                    });
                    if changed {
                        this.height_revision += 1;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn take_stored_values(&mut self, index: usize, values: serde_json::Value) -> bool {
        let Some(source) = self.sources.get(index) else {
            return false;
        };
        if source.values == values {
            return false;
        }
        let Some(resolved) = self
            .panel
            .sources
            .iter()
            .find(|panel| panel.plugin_id == source.plugin_id)
            .and_then(|panel| qol_config::contract::parse_spec_str(&panel.contract).ok())
            .and_then(|spec| qol_config::normalized::resolve_config(&spec, &values).ok())
        else {
            return false;
        };
        let mut changed = false;
        for fresh in rows_from_resolved(&resolved, index) {
            if !row_query_names(&fresh).is_empty() {
                continue;
            }
            let Some(row) = self
                .root_mut()
                .rows
                .iter_mut()
                .find(|row| row.source == index && row.id == fresh.id)
            else {
                continue;
            };
            if row_value_json(&row.control) != row_value_json(&fresh.control) {
                row.control = fresh.control;
                changed = true;
            }
        }
        if let Some(source) = self.sources.get_mut(index) {
            source.values = values;
        }
        changed
    }
}
