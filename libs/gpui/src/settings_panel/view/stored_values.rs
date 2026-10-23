use std::collections::{BTreeSet, HashSet};

use gpui::*;

use super::super::persistence::{panel_base, read_values};
use super::super::rows::{row_query_names, row_value_json, rows_from_resolved, Row};
use super::{SettingsPanelView, SourceState};

fn rereads_stored_values(source: &SourceState) -> bool {
    source.pages.is_none() && !source.copy.is_empty()
}

fn top_config_key(row: &Row) -> &str {
    row.config_key
        .split('.')
        .next()
        .unwrap_or(row.config_key.as_str())
}

pub(super) fn edited_top_keys(rows: &[Row], snapshot: &[Row], source: usize) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row.source == source && row_query_names(row).is_empty())
        .filter(|row| {
            snapshot.iter().any(|before| {
                before.source == source
                    && before.id == row.id
                    && row_value_json(&before.control) != row_value_json(&row.control)
            })
        })
        .map(top_config_key)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
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

    pub(super) fn edited_keys_for_save(&self, index: usize) -> Option<BTreeSet<String>> {
        let source = self.sources.get(index)?;
        let snapshot = self.resolved_rows(index, &source.values)?;
        Some(edited_top_keys(&self.root().rows, &snapshot, index))
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

    use super::super::super::rows::{LiveQuery, Row, RowControl};
    use super::super::tests::rows;
    use super::{edited_top_keys, subset};

    fn row(id: &str, config_key: &str, source: usize, control: RowControl) -> Row {
        let mut row = rows(&[false]).remove(0);
        row.id = id.into();
        row.config_key = config_key.into();
        row.source = source;
        row.control = control;
        row
    }

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn edited_row_is_included() {
        let snapshot = vec![row("a", "alpha", 0, RowControl::Toggle(false))];
        let current = vec![row("a", "alpha", 0, RowControl::Toggle(true))];
        assert_eq!(edited_top_keys(&current, &snapshot, 0), keys(&["alpha"]));
    }

    #[test]
    fn unedited_stale_row_is_excluded() {
        let snapshot = vec![
            row("a", "alpha", 0, RowControl::Toggle(false)),
            row("pins", "pins", 0, RowControl::Text("old".into())),
        ];
        let current = vec![
            row("a", "alpha", 0, RowControl::Toggle(true)),
            row("pins", "pins", 0, RowControl::Text("old".into())),
        ];
        assert_eq!(edited_top_keys(&current, &snapshot, 0), keys(&["alpha"]));
    }

    #[test]
    fn nested_key_maps_to_its_top_key() {
        let snapshot = vec![row("c", "capture.tags", 0, RowControl::Text("a".into()))];
        let current = vec![row("c", "capture.tags", 0, RowControl::Text("b".into()))];
        assert_eq!(edited_top_keys(&current, &snapshot, 0), keys(&["capture"]));
    }

    #[test]
    fn query_rows_are_excluded() {
        let live = |value| RowControl::Number {
            value,
            min: None,
            max: None,
            step: None,
            live: Some(LiveQuery {
                query: "level".into(),
                value_from: None,
            }),
            level: None,
        };
        let snapshot = vec![row("n", "volume", 0, live(1.0))];
        let current = vec![row("n", "volume", 0, live(2.0))];
        assert!(edited_top_keys(&current, &snapshot, 0).is_empty());
    }

    #[test]
    fn other_sources_are_ignored() {
        let snapshot = vec![row("a", "alpha", 0, RowControl::Toggle(false))];
        let current = vec![row("a", "alpha", 1, RowControl::Toggle(true))];
        assert!(edited_top_keys(&current, &snapshot, 0).is_empty());
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
