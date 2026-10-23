use std::collections::HashSet;

use gpui::*;

use super::super::persistence::{load_values, panel_base, read_values};
use super::super::rows::{row_query_names, row_value_json, rows_from_resolved, Row};
use super::{SettingsPanelView, SourceState};

fn rereads_stored_values(source: &SourceState) -> bool {
    source.pages.is_none() && !source.copy.is_empty()
}

impl SettingsPanelView {
    pub(in crate::settings_panel) fn reload_stored_values(&mut self, cx: &mut Context<Self>) {
        let generation = self.values_generation;
        let reads: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| rereads_stored_values(source))
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

    pub(super) fn fresh_values_for_save(&mut self, index: usize) -> Option<serde_json::Value> {
        let source = self
            .sources
            .get(index)
            .filter(|source| rereads_stored_values(source))?;
        let fresh = read_values(&panel_base(&source.plugin_id), source.path.as_deref())?;
        let snapshot_rows = self.resolved_rows(index, &source.values)?;
        let fresh_rows = self.resolved_rows(index, &fresh)?;
        let edited = self.edited_row_ids(index, &snapshot_rows);
        self.apply_fresh_rows(index, fresh_rows, &edited);
        Some(fresh)
    }

    fn resolved_rows(&self, index: usize, values: &serde_json::Value) -> Option<Vec<Row>> {
        let plugin_id = &self.sources.get(index)?.plugin_id;
        let resolved = self
            .panel
            .sources
            .iter()
            .find(|panel| &panel.plugin_id == plugin_id)
            .and_then(|panel| qol_config::contract::parse_spec_str(&panel.contract).ok())
            .and_then(|spec| qol_config::normalized::resolve_config(&spec, values).ok())?;
        Some(rows_from_resolved(&resolved, index))
    }

    fn edited_row_ids(&self, index: usize, snapshot_rows: &[Row]) -> HashSet<String> {
        snapshot_rows
            .iter()
            .filter(|snapshot| row_query_names(snapshot).is_empty())
            .filter(|snapshot| {
                self.root().rows.iter().any(|row| {
                    row.source == index
                        && row.id == snapshot.id
                        && row_value_json(&row.control) != row_value_json(&snapshot.control)
                })
            })
            .map(|snapshot| snapshot.id.clone())
            .collect()
    }

    fn apply_fresh_rows(
        &mut self,
        index: usize,
        fresh_rows: Vec<Row>,
        skip: &HashSet<String>,
    ) -> bool {
        let mut changed = false;
        for fresh in fresh_rows {
            if !row_query_names(&fresh).is_empty() || skip.contains(&fresh.id) {
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
        changed
    }

    fn take_stored_values(&mut self, index: usize, values: serde_json::Value) -> bool {
        let Some(source) = self.sources.get(index) else {
            return false;
        };
        if source.values == values {
            return false;
        }
        let Some(fresh_rows) = self.resolved_rows(index, &values) else {
            return false;
        };
        let changed = self.apply_fresh_rows(index, fresh_rows, &HashSet::new());
        if let Some(source) = self.sources.get_mut(index) {
            source.values = values;
        }
        changed
    }
}
