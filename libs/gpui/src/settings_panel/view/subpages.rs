use std::cell::Cell;
use std::rc::Rc;

use gpui::*;
use qol_config::contract::{interpolate_row_template, resolve_row_toggle};

use super::super::components::{
    settings_action_spinner, settings_description, settings_label_group, settings_query_spinner,
    settings_swatch, settings_value_text, RowGround, SettingsFeature, SettingsRow,
    SettingsValueTone,
};
use super::super::rows::{
    begin_list_item_action, inline_empty_text, inline_item_indices, inline_line_count,
    inline_slot_item, is_inline_list, item_control_index, list_item_actions, member_on, page_chain,
    page_sections_for, page_switch_plan, set_member, Row, RowControl, RowQueryState, RowSection,
    ITEM_ROW_MEMBER,
};
use super::super::SettingsDestination;
use super::list_card::{item_card_rows, item_card_sync, value_tone};
use super::{
    action_refresh_payload, horizontal_step_direction, opens_a_card, parsed_color, row_description,
    status_tone_color, Level, LevelHeader, SettingsPanelView,
};
use crate::phantom_nav::NavAxis;
use crate::status_indicator::{StatusIndicator, StatusTone};

const INLINE_ID_STRIDE: usize = 1000;

pub(super) enum SwitchTarget {
    Page(usize),
    Inline { row: usize, item_id: String },
    Member,
    Control,
}

pub(super) fn page_level(
    origin_row: usize,
    destination: SettingsDestination,
    sections: Vec<RowSection>,
    row_count: usize,
) -> Level {
    Level {
        rows: Vec::new(),
        sections,
        selected: 0,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds: (0..row_count).map(|_| Rc::new(Cell::new(None))).collect(),
        header: LevelHeader::Page(destination),
        origin_row: Some(origin_row),
        object_array: None,
        display_layout: None,
        list_card: false,
        live_card: false,
        choose: None,
        entries: None,
        form: None,
        list_item: None,
    }
}

pub(super) fn row_is_selectable(row: &Row, needle: Option<&str>) -> bool {
    !is_inline_list(&row.control) || !inline_item_indices(&row.control, needle).is_empty()
}

pub(super) fn complete_page_switch(
    stack: &mut [Level],
    source: usize,
    id: &str,
    failure: Option<String>,
) {
    let Some(root) = stack.first_mut() else {
        return;
    };
    let Some(RowControl::Subpage {
        switch: Some(switch),
        ..
    }) = root
        .rows
        .iter_mut()
        .find(|row| row.source == source && row.id == id)
        .map(|row| &mut row.control)
    else {
        return;
    };
    switch.pending = false;
    switch.error = failure;
}

impl SettingsPanelView {
    pub(super) fn selectable_rows(&self) -> Vec<usize> {
        let needle = self.filter_needle();
        self.current_visible_rows()
            .into_iter()
            .filter(|index| {
                self.level_rows()
                    .get(*index)
                    .is_some_and(|row| row_is_selectable(row, needle.as_deref()))
            })
            .collect()
    }

    pub(super) fn open_page(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.level_rows().get(row_index) else {
            return;
        };
        let (label, source, field) = (row.label.clone(), row.source, row.id.clone());
        let sections = page_sections_for(&self.root().sections, source, &field);
        if sections.is_empty() {
            return;
        }
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let row_count = self.root().rows.len();
        let child = page_level(row_index, destination.clone(), sections, row_count);
        self.push_card(destination, child);
        let first = self.selectable_rows().into_iter().next().unwrap_or(0);
        self.level_mut().selected = first;
        self.sync_scroll();
    }

    pub(super) fn land_on_paged_hit(&mut self, row: usize, cx: &mut Context<Self>) -> bool {
        if self.stack.len() != 1 {
            return false;
        }
        let chain = page_chain(&self.root().rows, &self.root().sections, row);
        if chain.is_empty() {
            return false;
        }
        self.land_on_search_hit(row, cx);
        for link in chain {
            self.open_page(link, cx);
        }
        self.level_mut().selected = row;
        self.sync_scroll();
        true
    }

    pub(super) fn opens_after_landing(&self, row: usize) -> bool {
        self.level_rows().get(row).is_some_and(|row| {
            opens_a_card(&row.control)
                || matches!(row.control, RowControl::Subpage { .. })
                || is_inline_list(&row.control)
        })
    }

    pub(super) fn render_page_link(&self, index: usize, cx: &mut Context<Self>) -> SettingsRow {
        let row = &self.level_rows()[index];
        let selected = index == self.level().selected;
        let focused = self.body_has_focus();
        let ground = RowGround::of(selected, focused);
        let (status, switch) = match &row.control {
            RowControl::Subpage { status, switch } => (status.as_deref(), switch.as_deref()),
            _ => (None, None),
        };
        let text = match status {
            Some(status) if status.error.is_some() => "unavailable".to_string(),
            Some(status) => status
                .label
                .clone()
                .unwrap_or_else(|| "\u{2013}".to_string()),
            None => String::new(),
        };
        let state = self.query_state_of(row);
        let description = row_description(row, &state, &self.applied_query_payloads);
        let label_group = settings_label_group(row.label.clone(), description, ground, self.kit);
        let mut line = SettingsRow::setting(("settings-page-link", index), self.kit)
            .selected(selected, focused)
            .child(label_group)
            .child(self.row_bounds_canvas(index));
        if matches!(state, RowQueryState::Loading) {
            line = line.opens(true).child(settings_query_spinner(
                ("settings-page-spinner", index),
                ground,
                self.kit,
            ));
        } else if let Some(switch) = switch {
            let on = switch.active && switch.error.is_none();
            if switch.pending {
                line = line.child(settings_action_spinner(
                    ("settings-page-working", index),
                    self.kit,
                ));
            }
            line = line.child(SettingsFeature::new(on, text, ground, self.kit));
        } else {
            let tone = status
                .filter(|status| status.error.is_none())
                .map_or(StatusTone::Muted, |status| status.tone);
            line = line.opens(true);
            if !text.is_empty() {
                line = line.child(StatusIndicator::new(
                    ("settings-page-status", index),
                    text,
                    rgb(status_tone_color(self.kit, tone)),
                ));
            }
        }
        line.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if !event.standard_click() {
                return;
            }
            this.click_row(index, window, cx);
        }))
    }

    pub(super) fn dispatch_page_switch(
        &mut self,
        row_index: usize,
        turn_on: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let ready = matches!(self.row_query_state(row_index), RowQueryState::Ready);
        let Some(row) = self.level_rows().get(row_index) else {
            return;
        };
        let RowControl::Subpage {
            switch: Some(switch),
            ..
        } = &row.control
        else {
            return;
        };
        let Some(action) = page_switch_plan(switch, ready, turn_on).map(str::to_owned) else {
            return;
        };
        let source = row.source;
        let row_id = row.id.clone();
        let refresh_query = switch.active_query.clone();
        let refresh_value_from = switch.active_value_from.clone();
        let Some(runtime) = self
            .source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        if let Some(RowControl::Subpage {
            switch: Some(switch),
            ..
        }) = self
            .level_rows_mut()
            .get_mut(row_index)
            .map(|row| &mut row.control)
        {
            switch.pending = true;
            switch.error = None;
        }
        self.pause_runtime_poll();
        let rearm_generation = self.runtime_poll_generation;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let (result, refreshed) = async_cx
                    .background_spawn(async move {
                        let result = runtime.run_action(&action, serde_json::Value::Null);
                        let refreshed = result.is_ok().then(|| {
                            let payload =
                                action_refresh_payload(&result, refresh_value_from.as_deref());
                            let answer = payload
                                .map(Ok)
                                .unwrap_or_else(|| runtime.query(&refresh_query));
                            (refresh_query, answer)
                        });
                        (result, refreshed)
                    })
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    complete_page_switch(&mut this.stack, source, &row_id, result.err());
                    if let Some((query, answer)) = refreshed {
                        this.apply_query(source, &query, answer, cx);
                    }
                    if rearm_generation == this.runtime_poll_generation {
                        this.start_runtime_poll(Some(this.runtime.poll_interval), cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn inline_slot(&self, row_index: usize) -> usize {
        let needle = self.filter_needle();
        let Some(row) = self.level_rows().get(row_index) else {
            return 0;
        };
        let RowControl::List { list, .. } = &row.control else {
            return 0;
        };
        let count = inline_item_indices(&row.control, needle.as_deref()).len();
        list.selected.min(count.saturating_sub(1))
    }

    pub(super) fn selected_inline_offset(&self, row_index: usize) -> usize {
        match self.level_rows().get(row_index) {
            Some(row) if is_inline_list(&row.control) => self.inline_slot(row_index),
            _ => 0,
        }
    }

    pub(super) fn row_weights(&self) -> impl Fn(usize) -> usize + '_ {
        let needle = self.filter_needle();
        move |index| match self.level_rows().get(index) {
            Some(row) if is_inline_list(&row.control) => {
                inline_line_count(&row.control, needle.as_deref())
            }
            _ => 1,
        }
    }

    fn set_inline_slot(&mut self, row_index: usize, slot: usize) {
        if let Some(RowControl::List { list, .. }) = self
            .level_rows_mut()
            .get_mut(row_index)
            .map(|row| &mut row.control)
        {
            list.selected = slot;
        }
    }

    pub(super) fn step_inline_slot(&mut self, direction: isize) -> bool {
        let selected = self.level().selected;
        let needle = self.filter_needle();
        let count = match self.level_rows().get(selected) {
            Some(row) if is_inline_list(&row.control) => {
                inline_item_indices(&row.control, needle.as_deref()).len()
            }
            _ => return false,
        };
        let slot = self.inline_slot(selected) as isize;
        let next = slot + direction;
        if count == 0 || next < 0 || next >= count as isize {
            return false;
        }
        self.set_inline_slot(selected, next as usize);
        self.sync_scroll();
        true
    }

    pub(super) fn enter_inline_row(&mut self, direction: isize) {
        let selected = self.level().selected;
        let needle = self.filter_needle();
        let count = match self.level_rows().get(selected) {
            Some(row) if is_inline_list(&row.control) => {
                inline_item_indices(&row.control, needle.as_deref()).len()
            }
            _ => return,
        };
        let slot = if direction < 0 {
            count.saturating_sub(1)
        } else {
            0
        };
        self.set_inline_slot(selected, slot);
    }

    pub(super) fn open_inline_item(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let needle = self.filter_needle();
        let slot = self.inline_slot(row_index);
        let Some(row) = self.root().rows.get(row_index) else {
            return;
        };
        let Some(item) = inline_slot_item(&row.control, needle.as_deref(), slot) else {
            return;
        };
        if item.pending || item_card_rows(&self.root().rows, row_index, item).is_empty() {
            return;
        }
        let item_id = item.id.clone();
        self.open_item_card(row_index, &item_id, cx);
    }

    fn click_inline_item(&mut self, row_index: usize, slot: usize, cx: &mut Context<Self>) {
        if row_index != self.level().selected {
            self.commit_edit(cx);
        }
        let already = self.level().selected == row_index && self.inline_slot(row_index) == slot;
        self.level_mut().selected = row_index;
        self.set_inline_slot(row_index, slot);
        if already {
            self.open_inline_item(row_index, cx);
        }
        self.sync_scroll();
        self.take_pointer_keys();
        cx.notify();
    }

    pub(super) fn render_inline_lines(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let row = &self.level_rows()[index];
        let needle = self.filter_needle();
        let indices = inline_item_indices(&row.control, needle.as_deref());
        let RowControl::List { actions, items, .. } = &row.control else {
            return Vec::new();
        };
        if indices.is_empty() {
            return vec![div()
                .id(("settings-inline-empty", index))
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(px(qol_theme::HEIGHT_SETTING_ROW))
                .px(px(qol_theme::SPACE_INSET))
                .child(settings_description(
                    inline_empty_text(&row.control),
                    RowGround::Pane,
                    self.kit,
                ))
                .into_any_element()];
        }
        let current = self.inline_slot(index);
        let focused = self.body_has_focus();
        indices
            .iter()
            .enumerate()
            .map(|(slot, item_index)| {
                let item = &items[*item_index];
                let selected = index == self.level().selected && slot == current;
                let ground = RowGround::of(selected, focused);
                let toggle = actions
                    .row_toggle
                    .as_ref()
                    .map(|spec| resolve_row_toggle(spec, &item.data));
                let value = item
                    .error
                    .clone()
                    .or_else(|| item.badge.clone())
                    .unwrap_or_default();
                let tone = if item.error.is_some() {
                    SettingsValueTone::Danger
                } else {
                    value_tone(item.effective_badge_tone())
                };
                let swatch = actions.swatch.as_deref().and_then(|template| {
                    parsed_color(&interpolate_row_template(template, &item.data))
                });
                let key = index * INLINE_ID_STRIDE + slot;
                let label_group = settings_label_group(
                    item.label.clone(),
                    item.subtitle.clone().map(SharedString::from),
                    ground,
                    self.kit,
                );
                let mut line = SettingsRow::setting(("settings-inline-item", key), self.kit)
                    .selected(selected, focused)
                    .opens(toggle.is_none())
                    .child(label_group);
                if item.pending {
                    line = line.child(settings_action_spinner(
                        ("settings-inline-spinner", key),
                        self.kit,
                    ));
                }
                line = match (toggle, swatch) {
                    (Some(toggle), swatch) => line.child(
                        SettingsFeature::new(toggle.on, value, ground, self.kit).swatch(swatch),
                    ),
                    (None, swatch) => {
                        let mut line = line;
                        if !value.is_empty() {
                            line = line.child(settings_value_text(value, tone, ground, self.kit));
                        }
                        if let Some(color) = swatch {
                            line = line.child(settings_swatch(color));
                        }
                        line
                    }
                };
                line.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if !event.standard_click() {
                        return;
                    }
                    this.click_inline_item(index, slot, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn dispatch_inline_toggle(
        &mut self,
        row_index: usize,
        item_id: &str,
        turn_on: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self
            .root_source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        let Some(RowControl::List { actions, items, .. }) = self
            .root_mut()
            .rows
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(spec) = actions.row_toggle.clone() else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        let toggle = resolve_row_toggle(&spec, &item.data);
        if toggle.on == turn_on {
            return;
        }
        let action = if turn_on {
            toggle.turn_on
        } else {
            toggle.turn_off
        };
        let Some(action) = begin_list_item_action(item, action) else {
            return;
        };
        let item_id = item.id.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(
                        async move { runtime.run_action(&action.action, action.input) },
                    )
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if let Err(error) = result {
                        this.set_list_action_error(row_index, &item_id, error);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn switch_target(&self) -> Option<SwitchTarget> {
        let level = self.level();
        if level.list_item.is_some() {
            let row = level.rows.get(level.selected)?;
            if row.variant.as_deref() == Some(ITEM_ROW_MEMBER) {
                return Some(SwitchTarget::Member);
            }
            return (item_control_index(row).is_some()
                && matches!(row.control, RowControl::Toggle(_)))
            .then_some(SwitchTarget::Control);
        }
        if level.list_card
            || level.live_card
            || level.choose.is_some()
            || level.entries.is_some()
            || level.form.is_some()
            || level.display_layout.is_some()
        {
            return None;
        }
        let selected = level.selected;
        let row = self.level_rows().get(selected)?;
        match &row.control {
            RowControl::Subpage {
                switch: Some(_), ..
            } => Some(SwitchTarget::Page(selected)),
            RowControl::List { actions, .. } if actions.inline && actions.row_toggle.is_some() => {
                let needle = self.filter_needle();
                let item =
                    inline_slot_item(&row.control, needle.as_deref(), self.inline_slot(selected))?;
                (!item.pending).then(|| SwitchTarget::Inline {
                    row: selected,
                    item_id: item.id.clone(),
                })
            }
            _ => None,
        }
    }

    pub(super) fn on_switch_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(direction) = horizontal_step_direction(key) else {
            return false;
        };
        let Some(target) = self.switch_target() else {
            return false;
        };
        if self.nav_guard.swallow(NavAxis::Horizontal, direction) {
            return true;
        }
        let turn_on = direction > 0.0;
        match target {
            SwitchTarget::Page(row) => self.dispatch_page_switch(row, Some(turn_on), cx),
            SwitchTarget::Inline { row, item_id } => {
                self.dispatch_inline_toggle(row, &item_id, turn_on, cx)
            }
            SwitchTarget::Member => {
                self.flip_item_member(Some(turn_on), cx);
            }
            SwitchTarget::Control => {
                self.flip_item_control(Some(turn_on), cx);
            }
        }
        true
    }

    pub(super) fn flip_item_control(
        &mut self,
        turn_on: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.level().selected;
        let Some(row) = self.level_rows_mut().get_mut(selected) else {
            return false;
        };
        if item_control_index(row).is_none() {
            return false;
        }
        let RowControl::Toggle(on) = &mut row.control else {
            return false;
        };
        let target = turn_on.unwrap_or(!*on);
        if target == *on {
            return true;
        }
        *on = target;
        let Some(action) = row.action.clone() else {
            return true;
        };
        self.dispatch_row_action(selected, &action, serde_json::Value::Null, cx);
        cx.notify();
        true
    }

    pub(super) fn flip_item_member(
        &mut self,
        turn_on: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(card) = self.level().list_item.as_ref() else {
            return false;
        };
        let origin_row = card.origin_row;
        let item_id = card.item_id.clone();
        let selected = self.level().selected;
        let Some(RowControl::List { actions, items, .. }) =
            self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return false;
        };
        let Some(item) = items.iter().find(|item| item.id == item_id) else {
            return false;
        };
        let Some(action) = list_item_actions(actions, item).into_iter().nth(selected) else {
            return false;
        };
        let Some((field, member)) = action.member_of.zip(action.member) else {
            return false;
        };
        let now = member_on(&self.root().rows, origin_row, &field, &member);
        let target = turn_on.unwrap_or(!now);
        if target == now {
            return true;
        }
        if !set_member(
            self.root_mut().rows.as_mut_slice(),
            origin_row,
            &field,
            &member,
            target,
        ) {
            return true;
        }
        self.persist();
        let (root, levels) = self.stack.split_at_mut(1);
        if let Some(level) = levels.last_mut() {
            item_card_sync(&root[0].rows, level);
        }
        self.height_revision += 1;
        cx.notify();
        true
    }

    pub(super) fn sync_inline_item_card(&mut self, query: &str, cx: &mut Context<Self>) -> bool {
        let front = self.stack.len() - 1;
        if front == 0 {
            return false;
        }
        let Some(card) = self.stack[front].list_item.as_ref() else {
            return false;
        };
        if self.stack[front - 1].list_card {
            return false;
        }
        let Some(RowControl::List {
            query: row_query, ..
        }) = self
            .root()
            .rows
            .get(card.origin_row)
            .map(|row| &row.control)
        else {
            return true;
        };
        if row_query != query {
            return true;
        }
        let (root, levels) = self.stack.split_at_mut(1);
        let close = item_card_sync(&root[0].rows, &mut levels[front - 1]);
        self.height_revision += 1;
        if close {
            self.pop_card(cx);
        }
        true
    }

    pub(super) fn switch_hint(&self) -> bool {
        self.switch_target().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::rows::{ListItem, PageSwitch};
    use super::super::super::SettingsDestination;
    use super::super::tests::{level, list_row};
    use super::{complete_page_switch, page_level, row_is_selectable};
    use super::{RowControl, RowSection};

    fn section(rows: Vec<usize>) -> RowSection {
        RowSection {
            label: "Paired".into(),
            description: None,
            rows,
            source: 0,
            device_kind: None,
            page: Some("bluetooth_page".into()),
        }
    }

    fn item(id: &str) -> ListItem {
        ListItem {
            id: id.into(),
            label: id.into(),
            subtitle: None,
            accent: None,
            badge: None,
            badge_tone: None,
            data: serde_json::Value::Null,
            pending: false,
            error: None,
        }
    }

    #[test]
    fn a_page_level_shares_the_root_rows_and_holds_only_its_sections() {
        let sections = vec![section(vec![1, 2])];
        let child = page_level(
            3,
            SettingsDestination::from_static("Bluetooth"),
            sections.clone(),
            5,
        );
        assert!(child.is_page());
        assert!(child.rows.is_empty());
        assert_eq!(child.sections, sections);
        assert_eq!(child.row_bounds.len(), 5);
        assert_eq!(child.origin_row, Some(3));
        assert!(!level(0).is_page());
    }

    #[test]
    fn an_inline_list_is_selectable_only_while_it_has_items_to_show() {
        let mut row = list_row();
        let RowControl::List { actions, .. } = &mut row.control else {
            unreachable!();
        };
        actions.inline = true;
        assert!(!row_is_selectable(&row, None));
        let RowControl::List { items, .. } = &mut row.control else {
            unreachable!();
        };
        items.push(item("a"));
        assert!(row_is_selectable(&row, None));
        assert!(!row_is_selectable(&row, Some("zzz")));
        assert!(row_is_selectable(&list_row(), None));
    }

    #[test]
    fn weighted_offsets_count_every_line_of_an_inline_list() {
        let sections = vec![vec![0, 1, 2]];
        let weight = |index: usize| if index == 1 { 3 } else { 1 };
        let offset =
            |row: usize| super::super::body_child_offset_with(&sections, &[true], row, &weight);
        assert_eq!(offset(0), Some(1));
        assert_eq!(offset(1), Some(2));
        assert_eq!(offset(2), Some(5));
    }

    #[test]
    fn finishing_a_page_switch_clears_the_wait_and_keeps_the_failure() {
        let mut row = list_row();
        row.id = "bluetooth_page".into();
        row.control = RowControl::Subpage {
            status: None,
            switch: Some(Box::new(PageSwitch {
                action: "enable".into(),
                active_action: Some("disable".into()),
                active_query: "status".into(),
                active_value_from: None,
                active: false,
                pending: true,
                error: None,
            })),
        };
        let mut root = level(0);
        root.rows = vec![row];
        let mut stack = vec![root];
        complete_page_switch(&mut stack, 0, "bluetooth_page", Some("no adapter".into()));
        let RowControl::Subpage {
            switch: Some(switch),
            ..
        } = &stack[0].rows[0].control
        else {
            unreachable!();
        };
        assert!(!switch.pending);
        assert_eq!(switch.error.as_deref(), Some("no adapter"));
    }
}
