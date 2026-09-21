use std::cell::Cell;
use std::rc::Rc;

use gpui::*;
use qol_config::contract::{is_picture_spec, resolve_slider_action, ResolvedRowAction};

use super::super::components::{
    settings_action_spinner, settings_label, ChoiceArt, RowGround, SettingsChoiceValue, SettingsRow,
};
use super::super::rows::{
    begin_list_item_action, filtered_list_items, list_item_actions, list_slider_value,
    primary_list_item_action, selected_list_item, ListActions, ListItem, ListSlider, Row,
    RowControl, RowSection, SliderHold,
};
use super::super::SettingsDestination;
use super::{
    align_to_step, horizontal_step_direction, round_to_step_precision, slider_fraction,
    status_tone_color, Level, LevelHeader, ListItemCard, SettingsPanelView,
};
use crate::phantom_nav::NavAxis;
use crate::pictures::PictureContext;
use crate::theme::SettingsPanelPalette;

pub(super) const SLIDER_DISPATCH_DEBOUNCE: std::time::Duration =
    std::time::Duration::from_millis(200);
const SLIDER_HOLD_DURATION: std::time::Duration = std::time::Duration::from_secs(10);

impl SettingsPanelView {
    fn slider_rows(&mut self) -> &mut [Row] {
        if list_card_index(&self.stack).is_some() {
            self.root_mut().rows.as_mut_slice()
        } else {
            self.level_mut().rows.as_mut_slice()
        }
    }

    pub(super) fn sync_list_card(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.stack.len() <= 1 {
            return;
        }
        let front = self.stack.len() - 1;
        let Some(list_index) = list_card_index(&self.stack) else {
            return;
        };
        let Some(origin_row) = self.stack[list_index].origin_row else {
            return;
        };
        let Some(parent) = self.root().rows.get(origin_row) else {
            return;
        };
        let RowControl::List {
            query: row_query, ..
        } = &parent.control
        else {
            return;
        };
        if row_query.as_str() != query {
            return;
        }
        let (root, levels) = self.stack.split_at_mut(1);
        list_card_sync(&mut root[0].rows, &mut levels[list_index - 1]);
        let close = list_index < front && item_card_sync(&root[0].rows, &mut levels[front - 1]);
        self.height_revision += 1;
        if close {
            self.pop_card(cx);
        }
    }

    pub(super) fn on_list_card_key(
        &mut self,
        key: &str,
        _key_char: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(direction) = horizontal_step_direction(key) {
            if self.card_list_has_slider() {
                if self.nav_guard.swallow(NavAxis::Horizontal, direction) {
                    return true;
                }
                if self.step_card_list_slider(direction, cx) {
                    cx.notify();
                    return true;
                }
            }
            return false;
        }
        match key {
            "enter" | "return" | "space" => {
                self.open_list_card_item(cx);
                cx.notify();
                true
            }
            "backspace" | "delete" => true,
            _ => false,
        }
    }

    fn card_list_has_slider(&self) -> bool {
        let Some(origin_row) = self.level().origin_row else {
            return false;
        };
        matches!(
            self.root().rows.get(origin_row).map(|row| &row.control),
            Some(RowControl::List {
                slider: Some(_),
                ..
            })
        )
    }

    fn open_list_card_item(&mut self, cx: &mut Context<Self>) {
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let slot = self.level().selected;
        let Some(RowControl::List {
            actions,
            items,
            filter,
            ..
        }) = self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = selected_list_item(actions, items, filter, slot) else {
            return;
        };
        if item.pending || primary_list_item_action(actions, item).is_none() {
            return;
        }
        let item_id = item.id.clone();
        self.open_item_card(origin_row, &item_id, cx);
    }

    fn open_item_card(&mut self, origin_row: usize, item_id: &str, cx: &mut Context<Self>) {
        let (label, description) = {
            let Some(parent) = self.root().rows.get(origin_row) else {
                return;
            };
            let RowControl::List { items, .. } = &parent.control else {
                return;
            };
            let Some(item) = items.iter().find(|item| item.id == item_id) else {
                return;
            };
            (item.label.clone(), item.subtitle.clone())
        };
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let child = {
            let Some(parent) = self.root().rows.get(origin_row) else {
                return;
            };
            let RowControl::List { items, .. } = &parent.control else {
                return;
            };
            let Some(item) = items.iter().find(|item| item.id == item_id) else {
                return;
            };
            let Some(child) = list_item_card_level(
                origin_row,
                item_id,
                &label,
                description,
                destination.clone(),
                parent,
                item,
            ) else {
                return;
            };
            child
        };
        self.push_card(destination, child);
        self.sync_scroll();
    }

    fn dispatch_resolved_list_action(
        &mut self,
        row_index: usize,
        item_id: &str,
        action: ResolvedRowAction,
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
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        if !list_item_actions(actions, item).contains(&action) {
            return;
        }
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

    pub(super) fn run_item_card_action(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.level().list_item.as_ref() else {
            return;
        };
        let origin_row = card.origin_row;
        let item_id = card.item_id.clone();
        let selected = self.level().selected;
        let Some(action_id) = self.level().rows.get(selected).map(|row| row.id.clone()) else {
            return;
        };
        let Some(RowControl::List { actions, items, .. }) =
            self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return;
        };
        let Some(item) = items.iter().find(|item| item.id == item_id) else {
            return;
        };
        let Some(action) = list_item_actions(actions, item)
            .into_iter()
            .find(|action| action.action == action_id)
        else {
            return;
        };
        self.dispatch_resolved_list_action(origin_row, &item_id, action, cx);
        self.pop_card(cx);
    }

    fn set_list_action_error(&mut self, row_index: usize, item_id: &str, error: String) {
        let Some(RowControl::List { items, .. }) = self
            .root_mut()
            .rows
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(item) = items.iter_mut().find(|item| item.id == item_id) else {
            return;
        };
        item.pending = false;
        item.error = Some(error);
    }

    fn step_card_list_slider(&mut self, direction: f64, cx: &mut Context<Self>) -> bool {
        let Some(origin_row) = self.level().origin_row else {
            return false;
        };
        let slot = self.level().selected;
        let Some(RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        }) = self
            .root_mut()
            .rows
            .get_mut(origin_row)
            .map(|row| &mut row.control)
        else {
            return false;
        };
        let Some(slider) = slider else {
            return false;
        };
        let Some(item) = selected_list_item(actions, items, filter, slot) else {
            return false;
        };
        step_list_slider(slider, item, direction);
        let item_id = item.id.clone();
        self.schedule_slider_dispatch(origin_row, &item_id, cx);
        true
    }

    fn set_list_slider_value(
        &mut self,
        row_index: usize,
        item_id: &str,
        fraction: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(RowControl::List { slider, items, .. }) = self
            .slider_rows()
            .get_mut(row_index)
            .map(|row| &mut row.control)
        else {
            return;
        };
        let Some(slider) = slider else {
            return;
        };
        if !items.iter().any(|item| item.id == item_id) {
            return;
        }
        let value = slider_value_from_fraction(
            slider.spec.min,
            slider.spec.max,
            slider.spec.step,
            fraction,
        );
        slider.values.insert(
            item_id.to_string(),
            SliderHold {
                value,
                dispatched: None,
                until: std::time::Instant::now() + SLIDER_HOLD_DURATION,
            },
        );
        self.schedule_slider_dispatch(row_index, item_id, cx);
    }

    fn schedule_slider_dispatch(
        &mut self,
        row_index: usize,
        item_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.slider_dispatch_generation += 1;
        let generation = self.slider_dispatch_generation;
        self.slider_pending.clear();
        self.slider_pending.insert((row_index, item_id.to_string()));
        let item_id = item_id.to_string();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(SLIDER_DISPATCH_DEBOUNCE)
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    if this.slider_dispatch_generation != generation {
                        return;
                    }
                    this.dispatch_list_slider(row_index, &item_id, cx);
                });
            }
        })
        .detach();
    }

    fn dispatch_list_slider(&mut self, row_index: usize, item_id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self
            .root_source_for(row_index)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        self.slider_pending
            .remove(&(row_index, item_id.to_string()));
        let Some((action, item_id)) =
            resolve_list_slider_dispatch(self.slider_rows(), row_index, item_id)
        else {
            return;
        };
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
                        super::super::rows::clear_slider_hold(
                            this.slider_rows(),
                            row_index,
                            &item_id,
                        );
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn open_list_card(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let (label, config_key, source, description) = match self.level().rows.get(row_index) {
            Some(row) => (
                row.label.clone(),
                row.config_key.clone(),
                row.source,
                self.sources[row.source]
                    .copy
                    .get(&row.id)
                    .and_then(|copy| copy.card_description.clone()),
            ),
            None => return,
        };
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &row.control
        else {
            return;
        };
        let child = list_card_level(
            ListCardOrigin {
                label: &label,
                config_key: &config_key,
                source,
                row: row_index,
            },
            actions,
            items,
            filter,
            list.selected,
            destination.clone(),
            description,
        );
        self.push_card(destination, child);
        self.sync_scroll();
    }

    fn render_list_slider_element(
        &self,
        row_index: usize,
        slider: &ListSlider,
        item: &super::super::rows::ListItem,
        value: f64,
        row: RowGround,
        cx: &mut Context<Self>,
    ) -> Div {
        let fill = slider_fraction(value, Some(slider.spec.min), Some(slider.spec.max)) * 72.0;
        let percent = slider_percent_label(slider.spec.min, slider.spec.max, value);
        let ground = row.rest(self.palette);
        let item_id = item.id.clone();
        let track_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let bounds_for_down = track_bounds.clone();
        let bounds_for_move = track_bounds.clone();
        let id_for_down = item_id.clone();
        let id_for_move = item_id.clone();
        let id_for_up = item_id.clone();
        let id_for_up_out = item_id;
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET))
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .relative()
                    .w(px(72.))
                    .h(px(4.))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(rgba(ground.well.packed()))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(fill))
                            .rounded_full()
                            .bg(rgb(ground.mark)),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| track_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let Some(bounds) = bounds_for_down.get() else {
                                return;
                            };
                            let fraction = (((event.position.x - bounds.left()).to_f64()
                                / bounds.size.width.to_f64())
                            .clamp(0.0, 1.0)) as f32;
                            this.slider_drag = Some((row_index, id_for_down.clone()));
                            this.set_list_slider_value(row_index, &id_for_down, fraction, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if !event.dragging()
                            || this.slider_drag.as_ref() != Some(&(row_index, id_for_move.clone()))
                        {
                            return;
                        }
                        let Some(bounds) = bounds_for_move.get() else {
                            return;
                        };
                        let fraction = (((event.position.x - bounds.left()).to_f64()
                            / bounds.size.width.to_f64())
                        .clamp(0.0, 1.0)) as f32;
                        this.set_list_slider_value(row_index, &id_for_move, fraction, cx);
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, _| {
                            if this.slider_drag.as_ref() == Some(&(row_index, id_for_up.clone())) {
                                this.slider_drag = None;
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, _| {
                            if this.slider_drag.as_ref()
                                == Some(&(row_index, id_for_up_out.clone()))
                            {
                                this.slider_drag = None;
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child(percent),
            )
    }

    pub(super) fn render_list_card_item(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(origin_row) = self.level().origin_row else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let Some(RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        }) = self.root().rows.get(origin_row).map(|row| &row.control)
        else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let Some(item) = selected_list_item(actions, items, filter, index) else {
            return div()
                .id(("settings-list-card-empty", index))
                .into_any_element();
        };
        let selected = index == self.level().selected;
        let row = RowGround::of(selected, self.body_has_focus());
        let visible = filtered_list_items(actions, items, filter);
        let art = list_item_art(items, &visible, index);
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        let word_color = (row == RowGround::Pane).then(|| list_card_word_color(item, self.palette));
        let mut line = SettingsRow::rule(("settings-list-card-item", index), self.palette)
            .selected(selected, self.body_has_focus())
            .child(
                settings_label(item.label.clone(), self.palette)
                    .flex_1()
                    .min_w(px(0.)),
            );
        if item.pending {
            line = line.child(settings_action_spinner(
                ("settings-list-card-spinner", index),
                self.palette,
            ));
        }
        line = line.child(
            SettingsChoiceValue::new(list_item_value_text(item), art, row, context, self.palette)
                .word_color(word_color),
        );
        if let Some((slider, value)) = slider.as_deref().and_then(|slider| {
            list_card_slider_value(slider, actions, items, filter, index)
                .map(|value| (slider, value))
        }) {
            line = line
                .child(self.render_list_slider_element(origin_row, slider, item, value, row, cx));
        }
        line.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.standard_click() {
                return;
            }
            let already_selected = this.level().selected == index;
            this.level_mut().selected = index;
            if already_selected {
                this.open_list_card_item(cx);
            }
            cx.notify();
        }))
        .into_any_element()
    }
}

fn list_item_art(items: &[ListItem], visible: &[usize], slot: usize) -> ChoiceArt {
    let labels = visible
        .iter()
        .map(|index| items[*index].label.as_str())
        .collect::<Vec<_>>();
    let letters = crate::pictures::letters_for(&labels);
    let spec = format!(
        "letters:{}",
        letters.get(slot).map(String::as_str).unwrap_or_default()
    );
    if is_picture_spec(&spec) {
        ChoiceArt::Picture(spec)
    } else {
        ChoiceArt::Picture("letters:?".to_string())
    }
}

fn list_card_word_color(item: &ListItem, palette: SettingsPanelPalette) -> u32 {
    if item.error.is_some() {
        return palette.state_off;
    }
    if item.badge.is_some() {
        return status_tone_color(palette, item.effective_badge_tone());
    }
    if list_item_value_text(item).is_empty() {
        palette.grounds.pane.faint
    } else {
        palette.grounds.pane.soft
    }
}

fn stepped_slider_value(current: f64, direction: f64, min: f64, max: f64, step: f64) -> f64 {
    let next = round_to_step_precision(current + direction * step, step);
    next.clamp(min, max)
}

pub(super) fn slider_value_from_fraction(min: f64, max: f64, step: f64, fraction: f32) -> f64 {
    let value = min + f64::from(fraction) * (max - min);
    align_to_step(value, Some(min), Some(max), step)
}

fn slider_percent_label(min: f64, max: f64, value: f64) -> String {
    let fraction = if max > min {
        (value - min) / (max - min)
    } else {
        0.0
    };
    format!("{:.0}%", fraction * 100.0)
}

fn step_list_slider(slider: &mut ListSlider, item: &ListItem, direction: f64) {
    let current = list_slider_value(&slider.spec, &slider.values, item);
    let next = stepped_slider_value(
        current,
        direction,
        slider.spec.min,
        slider.spec.max,
        slider.spec.step,
    );
    slider.values.insert(
        item.id.clone(),
        SliderHold {
            value: next,
            dispatched: None,
            until: std::time::Instant::now() + SLIDER_HOLD_DURATION,
        },
    );
}

fn list_card_slider_value(
    slider: &ListSlider,
    actions: &ListActions,
    items: &[ListItem],
    filter: &str,
    slot: usize,
) -> Option<f64> {
    let item = selected_list_item(actions, items, filter, slot)?;
    Some(list_slider_value(&slider.spec, &slider.values, item))
}

fn resolve_list_slider_dispatch(
    rows: &mut [Row],
    row_index: usize,
    item_id: &str,
) -> Option<(ResolvedRowAction, String)> {
    let RowControl::List { slider, items, .. } =
        rows.get_mut(row_index).map(|row| &mut row.control)?
    else {
        return None;
    };
    let slider = slider.as_mut()?;
    let item = items.iter().find(|item| item.id == item_id)?;
    let value = list_slider_value(&slider.spec, &slider.values, item);
    if let Some(hold) = slider.values.get_mut(item_id) {
        hold.dispatched = Some(value);
    }
    Some((
        resolve_slider_action(&slider.spec, &item.data, value),
        item.id.clone(),
    ))
}

fn list_item_value_text(item: &ListItem) -> String {
    item.error
        .clone()
        .or_else(|| item.badge.clone())
        .or_else(|| item.subtitle.clone())
        .unwrap_or_default()
}

fn list_card_child_rows(
    _label: &str,
    key: &str,
    source: usize,
    items: &[ListItem],
    visible: &[usize],
) -> Vec<Row> {
    visible
        .iter()
        .map(|item_index| {
            let item = &items[*item_index];
            Row {
                id: item.id.clone(),
                section_id: None,
                section_label: None,
                label: item.label.clone(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: key.to_string(),
                default: qol_config::contract::FieldDefault::String(String::new()),
                stream: None,
                action: None,
                visibility: None,
                source,
                control: RowControl::Text(list_item_value_text(item)),
            }
        })
        .collect()
}

struct ListCardOrigin<'a> {
    label: &'a str,
    config_key: &'a str,
    source: usize,
    row: usize,
}

fn list_card_level(
    origin: ListCardOrigin,
    actions: &ListActions,
    items: &[ListItem],
    filter: &str,
    selected_slot: usize,
    destination: SettingsDestination,
    description: Option<String>,
) -> Level {
    let ListCardOrigin {
        label,
        config_key,
        source,
        row: origin_row,
    } = origin;
    let visible = filtered_list_items(actions, items, filter);
    let rows = list_card_child_rows(label, config_key, source, items, &visible);
    let section = RowSection {
        label: label.to_string(),
        description,
        rows: (0..rows.len()).collect(),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    let selected = selected_slot.min(rows.len().saturating_sub(1));
    Level {
        rows,
        sections: vec![section],
        selected,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        header: LevelHeader::Card(destination),
        origin_row: Some(origin_row),
        object_array: None,
        display_layout: None,
        list_card: true,
        live_card: false,
        choose: None,
        entries: None,
        form: None,
        list_item: None,
    }
}

fn item_card_rows(
    actions: &ListActions,
    item: &ListItem,
    config_key: &str,
    source: usize,
) -> Vec<Row> {
    list_item_actions(actions, item)
        .into_iter()
        .map(|action| Row {
            id: action.action.clone(),
            section_id: None,
            section_label: None,
            label: action.label,
            description: action.description,
            placeholder: None,
            variant: None,
            config_key: config_key.to_string(),
            default: qol_config::contract::FieldDefault::String(String::new()),
            stream: None,
            action: None,
            visibility: None,
            source,
            control: RowControl::Text(String::new()),
        })
        .collect()
}

fn list_item_card_level(
    origin_row: usize,
    item_id: &str,
    label: &str,
    description: Option<String>,
    destination: SettingsDestination,
    row: &Row,
    item: &ListItem,
) -> Option<Level> {
    let RowControl::List { actions, .. } = &row.control else {
        return None;
    };
    let rows = item_card_rows(actions, item, &row.config_key, row.source);
    let section = RowSection {
        label: label.to_string(),
        description,
        rows: (0..rows.len()).collect(),
        source: row.source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    Some(Level {
        rows,
        sections: vec![section],
        selected: 0,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        header: LevelHeader::Card(destination),
        origin_row: Some(origin_row),
        object_array: None,
        display_layout: None,
        list_card: false,
        live_card: false,
        choose: None,
        entries: None,
        form: None,
        list_item: Some(ListItemCard {
            origin_row,
            item_id: item_id.to_string(),
        }),
    })
}

fn list_card_index(stack: &[Level]) -> Option<usize> {
    let front = stack.len().checked_sub(1)?;
    if stack[front].list_card {
        return Some(front);
    }
    let list = front.checked_sub(1)?;
    (stack[front].list_item.is_some() && stack[list].list_card).then_some(list)
}

fn item_card_sync(root_rows: &[Row], level: &mut Level) -> bool {
    let Some(card) = level.list_item.as_ref() else {
        return false;
    };
    let origin_row = card.origin_row;
    let item_id = card.item_id.clone();
    let selected_id = level.rows.get(level.selected).map(|row| row.id.clone());
    let Some(row) = root_rows.get(origin_row) else {
        return true;
    };
    let RowControl::List { actions, items, .. } = &row.control else {
        return true;
    };
    let Some(item) = items.iter().find(|item| item.id == item_id) else {
        return true;
    };
    let rows = item_card_rows(actions, item, &row.config_key, row.source);
    if rows.is_empty() {
        return true;
    }
    let selected = selected_id
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .unwrap_or(0);
    level.rows = rows;
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = selected;
    false
}

fn list_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let selected = level.selected;
    let selected_id = level.rows.get(selected).map(|row| row.id.clone());
    let description = level
        .sections
        .first()
        .and_then(|section| section.description.clone());
    let (label, config_key, source) = {
        let Some(parent) = root_rows.get(origin_row) else {
            return;
        };
        (
            parent.label.clone(),
            parent.config_key.clone(),
            parent.source,
        )
    };
    let Some(RowControl::List {
        actions,
        items,
        filter,
        list,
        ..
    }) = root_rows.get_mut(origin_row).map(|row| &mut row.control)
    else {
        return;
    };
    let visible = filtered_list_items(actions, items, filter);
    let visible_ids = visible
        .iter()
        .map(|slot| items[*slot].id.clone())
        .collect::<Vec<_>>();
    let next = match selected_id {
        Some(id) => visible_ids
            .iter()
            .position(|candidate| candidate == &id)
            .unwrap_or_else(|| selected.min(visible.len().saturating_sub(1))),
        None => selected.min(visible.len().saturating_sub(1)),
    };
    list.selected = next;
    list.sync(visible.len());
    let rows = list_card_child_rows(&label, &config_key, source, items, &visible);
    let section = RowSection {
        label: label.clone(),
        description,
        rows: (0..rows.len()).collect(),
        source,
    };
    level.rows = rows;
    level.sections = vec![section];
    level.row_bounds = (0..level.rows.len())
        .map(|_| Rc::new(Cell::new(None)))
        .collect();
    level.selected = next;
}

#[cfg(test)]
mod tests {
    use super::super::super::rows::{
        list_slider_value, selected_list_item, Row, RowControl, SliderHold,
    };
    use super::super::super::SettingsDestination;
    use super::super::tests::{level, list_row};
    use super::super::Level;
    use super::{
        item_card_sync, list_card_slider_value, list_item_art, list_item_card_level,
        slider_percent_label, slider_value_from_fraction, step_list_slider, stepped_slider_value,
    };
    use crate::phantom_nav::{NavAxis, PhantomNavGuard};
    use crate::settings_panel::components::ChoiceArt;
    use qol_config::contract::RowActionSpec;

    #[test]
    fn row_slider_steps_align_and_clamp_to_the_contract_range() {
        let cases = [
            (50.0, 1.0, 0.0, 100.0, 10.0, 60.0),
            (50.0, -1.0, 0.0, 100.0, 10.0, 40.0),
            (95.0, 1.0, 0.0, 100.0, 10.0, 100.0),
            (5.0, -1.0, 0.0, 100.0, 10.0, 0.0),
            (0.0, -1.0, 0.0, 100.0, 10.0, 0.0),
            (0.25, 1.0, 0.0, 1.0, 0.1, 0.4),
            (0.99, 1.0, 0.0, 1.0, 0.1, 1.0),
        ];
        for (current, direction, min, max, step, expected) in cases {
            assert_eq!(
                stepped_slider_value(current, direction, min, max, step),
                expected,
                "current={current} direction={direction} range={min}..{max} step={step}"
            );
        }
    }

    #[test]
    fn row_slider_click_fractions_map_to_aligned_values_and_percents() {
        let value_cases = [
            (0.0, 100.0, 10.0, 0.5, 50.0),
            (0.0, 100.0, 10.0, 0.33, 30.0),
            (0.0, 100.0, 10.0, 0.67, 70.0),
            (0.0, 100.0, 10.0, 0.0, 0.0),
            (0.0, 100.0, 10.0, 1.0, 100.0),
            (5.0, 250.0, 5.0, 0.4, 105.0),
            (5.0, 250.0, 5.0, 0.0, 5.0),
        ];
        for (min, max, step, fraction, expected) in value_cases {
            assert_eq!(
                slider_value_from_fraction(min, max, step, fraction),
                expected,
                "min={min} max={max} step={step} fraction={fraction}"
            );
        }

        let percent_cases = [
            (0.0, 100.0, 42.0, "42%"),
            (0.0, 100.0, 0.0, "0%"),
            (0.0, 100.0, 100.0, "100%"),
            (5.0, 250.0, 105.0, "41%"),
            (5.0, 250.0, 5.0, "0%"),
            (0.0, 1.0, 0.25, "25%"),
        ];
        for (min, max, value, expected) in percent_cases {
            assert_eq!(
                slider_percent_label(min, max, value),
                expected,
                "min={min} max={max} value={value}"
            );
        }
    }

    #[test]
    fn right_then_left_within_phantom_window_steps_slider_once() {
        let mut guard = PhantomNavGuard::new();
        let (min, max, step) = (0.0, 100.0, 10.0);
        let mut value = 50.0;

        assert!(!guard.swallow(NavAxis::Horizontal, 1.0));
        value = stepped_slider_value(value, 1.0, min, max, step);
        assert_eq!(value, 60.0);

        assert!(
            guard.swallow(NavAxis::Horizontal, -1.0),
            "a left arriving within the phantom window must be swallowed"
        );
        assert_eq!(
            value, 60.0,
            "the slider holds at one step after a real right and a phantom left"
        );
    }

    fn listed_row(items: Vec<super::super::super::rows::ListItem>) -> Row {
        let mut row = list_row();
        let RowControl::List { items: stored, .. } = &mut row.control else {
            unreachable!();
        };
        *stored = items;
        row
    }

    fn list_item(id: &str, name: &str) -> super::super::super::rows::ListItem {
        super::super::super::rows::ListItem {
            id: id.into(),
            label: name.into(),
            subtitle: None,
            accent: None,
            badge: None,
            badge_tone: None,
            data: serde_json::Value::Null,
            pending: false,
            error: None,
        }
    }

    fn list_card_fixture() -> (Vec<Row>, Level) {
        let mut root = level(0);
        root.rows = vec![listed_row(vec![
            list_item("a", "Alpha"),
            list_item("b", "Beta"),
            list_item("c", "Gamma"),
        ])];
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &root.rows[0].control
        else {
            unreachable!();
        };
        let child = super::list_card_level(
            super::ListCardOrigin {
                label: "Devices",
                config_key: "devices",
                source: 0,
                row: 0,
            },
            actions,
            items,
            filter,
            list.selected,
            SettingsDestination::from_static("Devices"),
            None,
        );
        (root.rows, child)
    }

    fn action_spec(
        action: &str,
        label: &str,
        description: Option<&str>,
        when: Option<&str>,
    ) -> RowActionSpec {
        RowActionSpec {
            action: action.into(),
            input: None,
            label: Some(label.into()),
            description: description.map(str::to_string),
            key: None,
            when: when.map(str::to_string),
        }
    }

    fn item_card_fixture() -> (Vec<Row>, Level) {
        let mut root = level(0);
        let mut row = listed_row(vec![list_item("a", "Alpha")]);
        let RowControl::List { actions, .. } = &mut row.control else {
            unreachable!();
        };
        actions.primary = Some(action_spec(
            "connect",
            "Connect",
            Some("Connects it."),
            None,
        ));
        actions.additional = vec![
            action_spec("disconnect", "Disconnect", Some("Drops it."), None),
            action_spec("remove", "Remove", None, None),
        ];
        root.rows = vec![row];
        let item = {
            let RowControl::List { items, .. } = &root.rows[0].control else {
                unreachable!();
            };
            items.iter().find(|item| item.id == "a").expect("item")
        };
        let child = list_item_card_level(
            0,
            "a",
            "Alpha",
            None,
            SettingsDestination::from_static("Alpha"),
            &root.rows[0],
            item,
        )
        .expect("item card level");
        (root.rows, child)
    }

    #[test]
    fn an_item_card_lists_actions_in_order_with_descriptions() {
        let (_, child) = item_card_fixture();
        assert_eq!(child.rows.len(), 3);
        assert_eq!(child.rows[0].label, "Connect");
        assert_eq!(child.rows[0].description.as_deref(), Some("Connects it."));
        assert_eq!(child.rows[1].label, "Disconnect");
        assert_eq!(child.rows[1].description.as_deref(), Some("Drops it."));
        assert_eq!(child.rows[2].label, "Remove");
        assert_eq!(child.rows[2].description, None);
        assert!(matches!(&child.rows[0].control, RowControl::Text(value) if value.is_empty()));
        assert_eq!(child.sections[0].label, "Alpha");
        assert_eq!(
            child.header,
            super::super::LevelHeader::Card(SettingsDestination::from_static("Alpha"))
        );
    }

    #[test]
    fn an_item_card_highlight_starts_on_the_primary_action() {
        let (_, child) = item_card_fixture();
        assert_eq!(child.selected, 0);
        assert_eq!(child.rows[child.selected].id, "connect");
        assert_eq!(child.rows[child.selected].label, "Connect");
    }

    #[test]
    fn an_item_card_resync_keeps_the_highlight_on_the_same_action() {
        let (mut rows, mut child) = item_card_fixture();
        child.selected = 2;
        let RowControl::List { actions, .. } = &mut rows[0].control else {
            unreachable!();
        };
        actions.additional = vec![
            action_spec("remove", "Remove", None, None),
            action_spec("disconnect", "Disconnect", Some("Drops it."), None),
        ];
        assert!(!item_card_sync(&rows, &mut child));
        assert_eq!(child.selected, 1);
        assert_eq!(child.rows[child.selected].id, "remove");
        assert_eq!(child.rows.len(), 3);
    }

    #[test]
    fn an_item_card_resync_reports_when_the_card_must_close() {
        let (mut rows, mut child) = item_card_fixture();
        let RowControl::List { items, .. } = &mut rows[0].control else {
            unreachable!();
        };
        items.clear();
        assert!(item_card_sync(&rows, &mut child));

        let (mut rows, mut child) = item_card_fixture();
        let RowControl::List { actions, .. } = &mut rows[0].control else {
            unreachable!();
        };
        actions.primary = None;
        actions.additional.clear();
        assert!(item_card_sync(&rows, &mut child));
    }

    #[test]
    fn card_enter_hints_open_list_cards_and_run_item_cards() {
        let (_, list) = list_card_fixture();
        assert_eq!(super::super::card_enter_hint(&list), Some("open"));
        let (_, item) = item_card_fixture();
        assert_eq!(super::super::card_enter_hint(&item), Some("run"));
    }

    #[test]
    fn list_item_art_letters_every_visible_label() {
        let items = vec![list_item("a", "Alpha"), list_item("b", "Beta")];
        assert_eq!(
            list_item_art(&items, &[0, 1], 0),
            ChoiceArt::Picture("letters:A".to_string())
        );
        assert_eq!(
            list_item_art(&items, &[0, 1], 1),
            ChoiceArt::Picture("letters:B".to_string())
        );
        let empty = vec![list_item("c", "")];
        assert_eq!(
            list_item_art(&empty, &[0], 0),
            ChoiceArt::Picture("letters:?".to_string())
        );
    }

    #[test]
    fn list_card_child_rows_builds_one_row_per_item() {
        let items = vec![
            super::super::super::rows::ListItem {
                id: "aa:bb".into(),
                label: "Headphones".into(),
                subtitle: Some("Sony WH-1000XM4".into()),
                accent: None,
                badge: Some("Connected".into()),
                badge_tone: None,
                data: serde_json::Value::Null,
                pending: false,
                error: None,
            },
            list_item("cc:dd", "Keyboard"),
        ];
        let rows = super::list_card_child_rows("Devices", "devices", 2, &items, &[0, 1]);
        assert_eq!(rows.len(), 2);
        for (slot, row) in rows.iter().enumerate() {
            assert_eq!(row.label, items[slot].label);
            assert_eq!(row.id, items[slot].id);
            assert_eq!(row.config_key, "devices");
            assert_eq!(row.source, 2);
        }
        assert!(matches!(
            &rows[0].control,
            RowControl::Text(value) if value == "Connected"
        ));
        assert!(matches!(
            &rows[1].control,
            RowControl::Text(value) if value.is_empty()
        ));
    }

    #[test]
    fn a_list_card_rebuild_keeps_the_selection_on_the_same_item() {
        let (mut rows, mut child) = list_card_fixture();
        child.selected = 1;
        super::super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": [
                {"id": "c", "name": "Gamma"},
                {"id": "a", "name": "Alpha"},
                {"id": "b", "name": "Beta"},
            ]})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 3);
        assert_eq!(child.selected, 2);
        assert_eq!(child.rows[child.selected].id, "b");
        let RowControl::List {
            list: root_list, ..
        } = &rows[0].control
        else {
            unreachable!();
        };
        assert_eq!(root_list.selected, 2);
    }

    #[test]
    fn a_list_card_rebuild_clamps_when_the_selected_item_disappears() {
        let (mut rows, mut child) = list_card_fixture();
        child.selected = 1;
        super::super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": [
                {"id": "a", "name": "Alpha"},
                {"id": "c", "name": "Gamma"},
            ]})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 2);
        assert_eq!(child.selected, 1);
        assert_eq!(child.rows[child.selected].id, "c");
        super::super::super::rows::apply_runtime_query(
            &mut rows,
            "items",
            Ok(serde_json::json!({"items": []})),
            &|_, _| false,
        );
        super::list_card_sync(&mut rows, &mut child);
        assert_eq!(child.rows.len(), 0);
        assert_eq!(child.selected, 0);
        let RowControl::List {
            list: root_list, ..
        } = &rows[0].control
        else {
            unreachable!();
        };
        assert_eq!(root_list.selected, 0);
    }

    fn list_item_with_value(
        id: &str,
        name: &str,
        level: f64,
    ) -> super::super::super::rows::ListItem {
        let mut item = list_item(id, name);
        item.data = serde_json::json!({ "level": level });
        item
    }

    fn slider_list_row() -> Row {
        let mut row = listed_row(vec![
            list_item_with_value("a", "Alpha", 0.3),
            list_item_with_value("b", "Beta", 0.8),
        ]);
        let RowControl::List { slider, .. } = &mut row.control else {
            unreachable!();
        };
        *slider = Some(Box::new(super::super::super::rows::ListSlider {
            spec: qol_config::contract::RowSliderSpec {
                value_from: "level".into(),
                min: 0.0,
                max: 1.0,
                step: 0.1,
                action: "set_level".into(),
                input: None,
            },
            values: std::collections::HashMap::new(),
        }));
        row
    }

    fn slider_card_fixture() -> (Vec<Row>, Level) {
        let mut root = level(0);
        root.rows = vec![slider_list_row()];
        let RowControl::List {
            actions,
            items,
            filter,
            list,
            ..
        } = &root.rows[0].control
        else {
            unreachable!();
        };
        let child = super::list_card_level(
            super::ListCardOrigin {
                label: "Devices",
                config_key: "devices",
                source: 0,
                row: 0,
            },
            actions,
            items,
            filter,
            list.selected,
            SettingsDestination::from_static("Devices"),
            None,
        );
        (root.rows, child)
    }

    #[test]
    fn a_card_slider_row_exposes_the_same_value_list_slider_value_returns() {
        let (mut rows, mut child) = slider_card_fixture();
        child.selected = 1;
        let RowControl::List {
            actions,
            slider,
            items,
            filter,
            ..
        } = &mut rows[0].control
        else {
            unreachable!();
        };
        slider.as_mut().unwrap().values.insert(
            "b".into(),
            SliderHold {
                value: 0.55,
                dispatched: Some(0.55),
                until: std::time::Instant::now() + super::SLIDER_HOLD_DURATION,
            },
        );
        let slider = slider.as_deref().unwrap();
        let held = list_slider_value(&slider.spec, &slider.values, &items[1]);
        assert_eq!(
            held, 0.55,
            "the hold wins over the item data while it lasts"
        );
        assert_eq!(
            list_card_slider_value(slider, actions, items, filter, child.selected),
            Some(held),
            "the card row for the selected slot shows the root list value for its item"
        );
        assert_eq!(
            list_card_slider_value(slider, actions, items, filter, 0),
            Some(list_slider_value(&slider.spec, &slider.values, &items[0])),
        );
    }

    #[test]
    fn card_slider_stepping_lands_on_the_item_the_well_would_step() {
        let (mut rows, mut child) = slider_card_fixture();
        let RowControl::List {
            actions,
            slider,
            items,
            filter,
            list,
            ..
        } = &mut rows[0].control
        else {
            unreachable!();
        };
        child.selected = 1;
        list.selected = child.selected;
        let card_item = selected_list_item(actions, items, filter, child.selected).unwrap();
        let well_item = selected_list_item(actions, items, filter, list.selected).unwrap();
        assert_eq!(card_item.id, "b");
        assert_eq!(well_item.id, card_item.id);
        let slider = slider.as_mut().unwrap();
        step_list_slider(slider, card_item, 1.0);
        let hold = slider
            .values
            .get(&well_item.id)
            .expect("the step records the hold under the id the well would step");
        assert_eq!(hold.value, 0.9);
    }
}
