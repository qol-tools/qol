use std::collections::{BTreeSet, HashSet};

use gpui::*;

use super::super::persistence::{panel_base, read_values};
use super::super::rows::{merged_config, row_query_names, row_value_json, rows_from_resolved, Row};
use super::{SettingsPanelView, SourceState};

fn rereads_stored_values(source: &SourceState) -> bool {
    source.pages.is_none() && !source.copy.is_empty()
}

pub(super) fn changed_top_keys(
    merged: &serde_json::Value,
    baseline: &serde_json::Value,
) -> BTreeSet<String> {
    let Some(map) = merged.as_object() else {
        return BTreeSet::new();
    };
    map.iter()
        .filter(|(key, value)| baseline.get(key.as_str()) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect()
}

pub(super) fn subset(values: &serde_json::Value, keys: &BTreeSet<String>) -> serde_json::Value {
    let Some(map) = values.as_object() else {
        return values.clone();
    };
    serde_json::Value::Object(
        map.iter()
            .filter(|(key, _)| keys.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
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
                .filter_map(|(index, base, path)| {
                    read_values(&base, path.as_deref()).map(|values| (index, values))
                })
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

    pub(super) fn saved_baseline(&self, index: usize) -> Option<serde_json::Value> {
        let source = self.sources.get(index)?;
        let snapshot = self.resolved_rows(index, &source.values)?;
        Some(merged_config(&source.values, &snapshot))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{changed_top_keys, subset};

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn changed_top_keys_names_only_the_values_that_moved() {
        let baseline =
            serde_json::json!({ "alpha": false, "capture": { "tags": ["a"] }, "pins": ["old"] });
        let cases = [
            (
                serde_json::json!({ "alpha": true, "capture": { "tags": ["a"] }, "pins": ["old"] }),
                keys(&["alpha"]),
            ),
            (
                serde_json::json!({ "alpha": false, "capture": { "tags": ["b"] }, "pins": ["old"] }),
                keys(&["capture"]),
            ),
            (
                serde_json::json!({ "alpha": false, "capture": { "tags": ["a"] }, "pins": ["old"], "mode": "x" }),
                keys(&["mode"]),
            ),
            (baseline.clone(), keys(&[])),
            (serde_json::json!(null), keys(&[])),
        ];
        for (merged, expected) in cases {
            assert_eq!(changed_top_keys(&merged, &baseline), expected, "{merged}");
        }
    }

    #[test]
    fn subset_keeps_only_named_top_keys() {
        let values = serde_json::json!({ "alpha": 1, "capture": { "tags": [] }, "pins": ["x"] });
        assert_eq!(
            subset(&values, &keys(&["alpha", "capture", "missing"])),
            serde_json::json!({ "alpha": 1, "capture": { "tags": [] } })
        );
        assert_eq!(subset(&values, &keys(&[])), serde_json::json!({}));
    }
}
