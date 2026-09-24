use std::cell::Cell;
use std::rc::Rc;

use gpui::*;
use qol_config::contract::resolve_slider_action;

use super::super::components::{
    display_layout_ghost, display_layout_stage, display_layout_tile, settings_action_spinner,
    settings_description, settings_label, settings_label_group, settings_message, ChoiceArt,
    DisplayLayoutTile, RowGround, SettingsChoiceValue, SettingsFeedback, SettingsRow, TileArt,
};
use super::super::display_layout::{mode_label, nudge_step, Display, DisplayLayoutState, Rect};
use super::super::form_nav::adjacent_visible_row;
use super::super::rows::{Row, RowControl, RowSection};
use super::super::SettingsDestination;
use super::choose_card::{label_parts, ChooseOrigin, ChooseState, ChooseTile};
use super::list_card::{
    slider_percent_label, slider_track, slider_value_from_fraction, stepped_slider_value,
    SliderTrackStyle,
};
use super::{slider_fraction, Level, LevelHeader, SettingsPanelView};
use crate::pictures::PictureContext;

const DISPLAY_LAYOUT_STAGE_PAD: f32 = qol_theme::SPACE_INSET;

impl SettingsPanelView {
    pub(super) fn on_display_layout_card_key(
        &mut self,
        key: &str,
        shift: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let selected = self.level().selected;
        let editing = self
            .level()
            .display_layout
            .as_ref()
            .is_some_and(|state| state.editing());
        match display_layout_card_action(selected, editing, key, shift) {
            DisplayLayoutCardAction::Nudge(dx, dy) => {
                self.stage_display_layout_nudge(dx, dy);
                cx.notify();
                true
            }
            DisplayLayoutCardAction::CycleDisplays(step) => {
                if let Some(state) = self.level_mut().display_layout.as_mut() {
                    state.cycle(step);
                }
                cx.notify();
                true
            }
            DisplayLayoutCardAction::ToggleEditing => {
                if let Some(state) = self.level_mut().display_layout.as_mut() {
                    state.set_editing(!state.editing());
                }
                cx.notify();
                true
            }
            DisplayLayoutCardAction::OpenModePicker => {
                self.open_display_mode_card(cx);
                cx.notify();
                true
            }
            DisplayLayoutCardAction::ChoosePrimary => {
                self.stage_display_layout_primary();
                cx.notify();
                true
            }
            DisplayLayoutCardAction::CyclePrimary(step) => {
                self.cycle_display_layout_primary(step);
                cx.notify();
                true
            }
            DisplayLayoutCardAction::MoveSelection(step) => {
                let selected = self.level().selected;
                let rows = &self.level().sections[0].rows;
                let next = adjacent_visible_row(rows, selected, step as isize);
                self.level_mut().selected = next;
                self.sync_scroll();
                cx.notify();
                true
            }
            DisplayLayoutCardAction::StepBrightness(direction) => {
                self.step_display_layout_brightness(direction, cx);
                cx.notify();
                true
            }
            DisplayLayoutCardAction::Consume => true,
            DisplayLayoutCardAction::Apply => {
                self.apply_display_layout(cx);
                true
            }
            DisplayLayoutCardAction::Pop => {
                self.pop_card(cx);
                true
            }
            DisplayLayoutCardAction::LeaveEditing => {
                if let Some(state) = self.level_mut().display_layout.as_mut() {
                    state.set_editing(false);
                }
                cx.notify();
                true
            }
            DisplayLayoutCardAction::Discard => display_layout_escape(self.level_mut()),
            DisplayLayoutCardAction::FallThrough => false,
        }
    }

    fn open_display_mode_card(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.level().display_layout.as_ref() else {
            return;
        };
        if state.modes_for_selected().is_empty() || !state.modes_writable() {
            return;
        }
        let source = self
            .level()
            .sections
            .first()
            .map(|section| section.source)
            .unwrap_or(0);
        let label = RESOLUTION_LABEL;
        let Some(destination) = self.card_destination(label, cx) else {
            return;
        };
        let section = RowSection {
            label: label.to_string(),
            description: Some(RESOLUTION_CARD_DESCRIPTION.to_string()),
            rows: Vec::new(),
            source,
        };
        let child = Level {
            rows: Vec::new(),
            sections: vec![section],
            selected: 0,
            active_section: None,
            selected_section: 0,
            body_scroll: crate::scroll_list::SelectionScroll::new(),
            active_control: None,
            row_bounds: Vec::new(),
            header: LevelHeader::Card(destination.clone()),
            origin_row: Some(DISPLAY_LAYOUT_RESOLUTION_ROW),
            object_array: None,
            display_layout: None,
            list_card: false,
            live_card: false,
            choose: Some(ChooseState {
                origin: ChooseOrigin::DisplayModes,
                highlighted: None,
            }),
            entries: None,
            form: None,
            list_item: None,
        };
        self.push_card(destination, child);
        self.sync_scroll();
    }

    fn stage_display_layout_primary(&mut self) {
        let Some(id) = self
            .level()
            .display_layout
            .as_ref()
            .and_then(|state| state.selected_id().map(str::to_string))
        else {
            return;
        };
        self.make_display_layout_primary(&id);
    }

    fn cycle_display_layout_primary(&mut self, step: i32) {
        if let Some(state) = self.level_mut().display_layout.as_mut() {
            state.cycle(step);
            state.set_primary();
        }
    }

    fn set_display_layout_brightness(
        &mut self,
        display_id: &str,
        fraction: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let Some(slider) = self
            .level()
            .display_layout
            .as_ref()
            .and_then(|state| state.bindings().slider.clone())
        else {
            return;
        };
        let value = slider_value_from_fraction(slider.min, slider.max, slider.step, fraction);
        let value = value.round().clamp(0.0, 255.0) as u8;
        if let Some(state) = self.level_mut().display_layout.as_mut() {
            state.set_brightness(display_id, value);
        }
        self.schedule_slider_dispatch(
            origin_row,
            display_id.to_string(),
            cx,
            |this, row, id, cx| {
                this.dispatch_display_layout_brightness(row, &id, cx);
            },
        );
        cx.notify();
    }

    fn step_display_layout_brightness(&mut self, direction: i32, cx: &mut Context<Self>) {
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let Some((id, current, slider)) = self.level().display_layout.as_ref().and_then(|state| {
            let slider = state.bindings().slider.clone()?;
            let display = state.selected()?;
            Some((display.id.clone(), display.brightness?, slider))
        }) else {
            return;
        };
        let next = stepped_slider_value(
            f64::from(current),
            f64::from(direction),
            slider.min,
            slider.max,
            slider.step,
        );
        let next = next.round().clamp(0.0, 255.0) as u8;
        if let Some(state) = self.level_mut().display_layout.as_mut() {
            state.set_brightness(&id, next);
        }
        self.schedule_slider_dispatch(origin_row, id, cx, |this, row, id, cx| {
            this.dispatch_display_layout_brightness(row, &id, cx);
        });
    }

    fn dispatch_display_layout_brightness(
        &mut self,
        row: usize,
        display_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self
            .root_source_for(row)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        self.slider_pending.remove(&(row, display_id.to_string()));
        let Some(state) = self.level().display_layout.as_ref() else {
            return;
        };
        let Some(slider) = state.bindings().slider.clone() else {
            return;
        };
        let Some(value) = state
            .displays()
            .iter()
            .find(|display| display.id == display_id)
            .and_then(|display| display.brightness)
        else {
            return;
        };
        let resolved = resolve_slider_action(
            &slider,
            &serde_json::json!({ "id": display_id }),
            f64::from(value),
        );
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                let _ = async_cx
                    .background_spawn(async move {
                        runtime.run_action(&resolved.action, resolved.input)
                    })
                    .await;
            }
        })
        .detach();
    }

    fn stage_display_layout_nudge(&mut self, dx: i32, dy: i32) {
        let (width, height) = self.display_layout_viewport();
        let Some(state) = self.level_mut().display_layout.as_mut() else {
            return;
        };
        state.set_viewport(width, height, DISPLAY_LAYOUT_STAGE_PAD);
        state.nudge(dx, dy);
    }

    pub(super) fn choose_display_mode(&mut self, pick: usize, cx: &mut Context<Self>) {
        let Some(parent) = self.stack.len().checked_sub(2) else {
            return;
        };
        let Some(option) = self.stack[parent]
            .display_layout
            .as_ref()
            .and_then(|state| state.modes_for_selected().get(pick).cloned())
        else {
            return;
        };
        if let Some(state) = self.stack[parent].display_layout.as_mut() {
            state.stage_mode(&option);
        }
        self.pop_card(cx);
    }

    fn make_display_layout_primary(&mut self, id: &str) {
        if let Some(state) = self.level_mut().display_layout.as_mut() {
            state.select(id);
            state.set_primary();
        }
    }

    fn apply_display_layout(&mut self, cx: &mut Context<Self>) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(origin_row) = self.level().origin_row else {
            return;
        };
        let Some(runtime) = self
            .root_source_for(origin_row)
            .map(|source| source.runtime.clone())
        else {
            return;
        };
        let Some(state) = self.level().display_layout.as_ref() else {
            return;
        };
        if state.pending() || !state.has_staged_edits() || !state.is_committable() {
            return;
        }
        let steps = display_layout_dispatch_steps(state);
        if steps.is_empty() {
            return;
        }
        let Some(state) = self.level_mut().display_layout.as_mut() else {
            return;
        };
        state.set_pending(true);
        state.set_error(None);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let outcome = async_cx
                    .background_spawn(async move {
                        for (action, input) in steps {
                            if let Err(message) = runtime.run_action(&action, input) {
                                return Some(message);
                            }
                        }
                        None
                    })
                    .await;
                let _ = this.update(&mut async_cx, |this, cx| {
                    let committed = {
                        let Some(state) = this.level_mut().display_layout.as_mut() else {
                            return;
                        };
                        state.set_pending(false);
                        state.set_error(outcome.clone());
                        if outcome.is_none() {
                            state.finish_commit();
                            true
                        } else {
                            false
                        }
                    };
                    if committed {
                        this.commit_display_layout_to_root();
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn commit_display_layout_to_root(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        display_layout_card_commit(&mut root[0].rows, front.last().expect("front level"));
        self.height_revision += 1;
    }

    fn display_layout_viewport(&self) -> (f32, f32) {
        let (width, height) = self.display_layout_stage.get();
        if width > 0.0 && height > 0.0 {
            return (width, height);
        }
        let fallback_width = super::super::PANEL_WIDE_WIDTH - 2.0 * qol_theme::SPACE_PAD;
        (fallback_width, super::super::PANEL_DISPLAY_LAYOUT_HEIGHT)
    }

    fn drag_display_layout_to(&mut self, position: Point<Pixels>) -> bool {
        let Some((_, origin)) = self.display_layout_press.as_ref() else {
            return false;
        };
        let dx = (position.x - origin.x).to_f64() as f32;
        let dy = (position.y - origin.y).to_f64() as f32;
        let (width, height) = self.display_layout_viewport();
        let Some(state) = self.level_mut().display_layout.as_mut() else {
            return false;
        };
        state.set_viewport(width, height, DISPLAY_LAYOUT_STAGE_PAD);
        state.drag_delta(dx, dy)
    }

    fn finish_display_layout_drag(&mut self) -> bool {
        if self.display_layout_press.take().is_none() {
            return false;
        }
        self.level_mut()
            .display_layout
            .as_mut()
            .is_some_and(DisplayLayoutState::end_drag)
    }

    pub(super) fn sync_display_layout_card(&mut self, query: &str) {
        if self.stack.len() <= 1 {
            return;
        }
        let matches = match self.level().display_layout.as_ref() {
            Some(state) => {
                let bindings = state.bindings();
                bindings.query.as_str() == query || bindings.active_query.as_deref() == Some(query)
            }
            None => false,
        };
        if !matches {
            return;
        }
        let (root, front) = self.stack.split_at_mut(1);
        display_layout_card_sync(&mut root[0].rows, front.last_mut().expect("front level"));
        self.height_revision += 1;
    }

    pub(super) fn open_display_layout_card(&mut self, row_index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.level().rows.get(row_index) else {
            return;
        };
        let RowControl::DisplayLayout(state) = &row.control else {
            return;
        };
        let label = row.label.clone();
        let source = row.source;
        let description = self.sources[row.source]
            .copy
            .get(&row.id)
            .and_then(|copy| copy.card_description.clone());
        let state = state.as_ref().clone();
        let Some(destination) = self.card_destination(&label, cx) else {
            return;
        };
        let child = display_layout_card_level(
            &label,
            source,
            state,
            row_index,
            destination.clone(),
            description,
        );
        self.push_card(destination, child);
        self.sync_scroll();
    }

    pub(super) fn render_display_layout_card(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(state) = self.level().display_layout.as_ref() else {
            return Vec::new();
        };
        let palette = self.palette;
        let mut staged = state.clone();
        let (stage_width, stage_height) = self.display_layout_viewport();
        staged.set_viewport(stage_width, stage_height, DISPLAY_LAYOUT_STAGE_PAD);
        let selected_id = staged.selected_id().map(str::to_string);
        let fit = staged.fit();
        let stage_viewport = Rc::clone(&self.display_layout_stage);
        let stage_entity = cx.weak_entity();
        let drag_entity = stage_entity.clone();
        let stage_bounds = canvas(
            move |bounds, window, _cx| {
                let next = (
                    bounds.size.width.to_f64() as f32,
                    bounds.size.height.to_f64() as f32,
                );
                if stage_viewport.get() == next {
                    return;
                }
                stage_viewport.set(next);
                let entity = stage_entity.clone();
                window.on_next_frame(move |_, cx| {
                    let _ = entity.update(cx, |_, cx| cx.notify());
                });
            },
            move |_, _, window, _| {
                watch_display_layout_drag(window, drag_entity.clone());
            },
        )
        .absolute()
        .inset_0();
        let mut stage = display_layout_stage(palette, super::super::PANEL_DISPLAY_LAYOUT_HEIGHT)
            .child(stage_bounds);
        match fit {
            Some(fit) => {
                let preview = staged.drag_preview();
                let dragged = preview.as_ref().map(|(id, _, _)| id.as_str());
                let tile_at = |index: usize, display: &Display, rect: Rect, conflicted: bool| {
                    let press_id = display.id.clone();
                    let tile = DisplayLayoutTile {
                        connector: display.connector.clone(),
                        resolution: display.resolution(),
                        left: fit.to_client_x(rect.x),
                        top: fit.to_client_y(rect.y),
                        width: fit.to_client_width(rect.width),
                        height: fit.to_client_height(rect.height),
                        selected: selected_id.as_deref() == Some(display.id.as_str()),
                        primary: staged.is_primary(display),
                        conflicted,
                    };
                    display_layout_tile(index, &tile, palette)
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.level_mut().selected = DISPLAY_LAYOUT_STAGE_ROW;
                                let (width, height) = this.display_layout_viewport();
                                if let Some(state) = this.level_mut().display_layout.as_mut() {
                                    state.set_viewport(width, height, DISPLAY_LAYOUT_STAGE_PAD);
                                    if state.begin_drag(&press_id) {
                                        this.display_layout_press =
                                            Some((press_id.clone(), event.position));
                                    }
                                }
                                cx.notify();
                            }),
                        )
                };
                for (index, display) in staged.displays().iter().enumerate() {
                    if dragged == Some(display.id.as_str()) {
                        continue;
                    }
                    let rect = staged.rect_of(display);
                    let conflicted = staged.conflicts(&display.id);
                    let tile = tile_at(index, display, rect, conflicted);
                    stage = stage.child(tile);
                }
                if let Some((dragged_id, pointer, landing)) = preview {
                    if let Some((dragged_index, display)) = staged
                        .displays()
                        .iter()
                        .enumerate()
                        .find(|(_, display)| display.id == dragged_id)
                    {
                        let ghost = display_layout_ghost(
                            fit.to_client_x(landing.x),
                            fit.to_client_y(landing.y),
                            fit.to_client_width(landing.width),
                            fit.to_client_height(landing.height),
                            palette,
                        );
                        stage = stage.child(ghost);
                        let tile = tile_at(dragged_index, display, pointer, false);
                        stage = stage.child(tile);
                    }
                }
            }
            None => {
                stage = stage.child(settings_message("No displays detected", false, palette));
            }
        }
        let mut stage_row = div()
            .id(("settings-display-layout-stage", 0usize))
            .flex()
            .flex_col()
            .w_full()
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.level_mut().selected = DISPLAY_LAYOUT_STAGE_ROW;
                cx.notify();
            }))
            .child(stage);
        stage_row = self.mark_selected(
            stage_row,
            display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_STAGE_ROW),
        );
        let resolution_ground = RowGround::of(
            display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_RESOLUTION_ROW),
            self.body_has_focus(),
        );
        let options = staged.modes_for_selected();
        let modes_available = !options.is_empty() && staged.modes_writable();
        let staged_mode = staged.selected_id().and_then(|id| staged.staged_mode(id));
        let (mode_word, mode_art) = match staged_mode {
            Some(mode) => (
                mode.label(),
                format!("display-mode:{}x{}", mode.width, mode.height),
            ),
            None => match staged.selected() {
                Some(display) => {
                    let width = u32::try_from(display.width).unwrap_or(0);
                    let height = u32::try_from(display.height).unwrap_or(0);
                    (
                        mode_label(width, height, display.refresh_hz),
                        format!("display-mode:{width}x{height}"),
                    )
                }
                None => (String::new(), String::new()),
            },
        };
        let context = PictureContext::for_accent(
            qol_theme::runtime_theme().mode,
            qol_theme::runtime_accent_key(),
        );
        let mut mode_value = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(qol_theme::SPACE_INSET));
        if staged.pending() {
            mode_value = mode_value.child(settings_action_spinner(
                ("settings-display-layout-spinner", 0usize),
                palette,
            ));
        }
        if modes_available {
            mode_value = mode_value.child(SettingsChoiceValue::new(
                mode_word,
                ChoiceArt::Picture(mode_art),
                resolution_ground,
                context,
                palette,
            ));
        } else {
            mode_value = mode_value.child(settings_description(
                "unavailable",
                resolution_ground,
                palette,
            ));
        }
        let mut mode_row = SettingsRow::rule(("settings-display-layout-mode", 0usize), palette)
            .selected(
                display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_RESOLUTION_ROW),
                self.body_has_focus(),
            )
            .child(settings_label_group(
                RESOLUTION_LABEL,
                None,
                resolution_ground,
                palette,
            ))
            .child(mode_value);
        if modes_available {
            mode_row = mode_row.on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.stop_propagation();
                this.open_display_mode_card(cx);
                cx.notify();
            }));
        }
        let mode_control = div().child(mode_row);
        let mut primary_group = self.kit.segmented_group();
        for (index, display) in staged.displays().iter().enumerate() {
            let active = staged.is_primary(display);
            let id = display.id.clone();
            primary_group = primary_group.child(
                self.kit
                    .segment(display.connector.clone(), active)
                    .id(("settings-display-layout-primary", index))
                    .cursor(CursorStyle::PointingHand)
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        if !event.standard_click() {
                            return;
                        }
                        this.make_display_layout_primary(&id);
                        cx.notify();
                    })),
            );
        }
        let primary_ground = RowGround::of(
            display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_PRIMARY_ROW),
            self.body_has_focus(),
        );
        let primary_row =
            SettingsRow::rule(("settings-display-layout-primary-row", 0usize), palette)
                .selected(
                    display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_PRIMARY_ROW),
                    self.body_has_focus(),
                )
                .child(settings_label_group(
                    "Primary display",
                    None,
                    primary_ground,
                    palette,
                ))
                .child(primary_group);
        let apply_row = SettingsRow::rule(("settings-display-layout-apply", 0usize), palette)
            .selected(
                display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_APPLY_ROW),
                self.body_has_focus(),
            )
            .dimmed(!staged.has_staged_edits() || !staged.is_committable())
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(self.palette.state_on))
                    .child("Apply"),
            )
            .child(
                div()
                    .text_size(px(qol_theme::TEXT_CAPTION))
                    .text_color(rgb(self.palette.label_text))
                    .child("enter"),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.apply_display_layout(cx);
                cx.notify();
            }));
        let cancel_row = SettingsRow::rule(("settings-display-layout-cancel", 0usize), palette)
            .selected(
                display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_CANCEL_ROW),
                self.body_has_focus(),
            )
            .child(settings_label("Cancel", palette))
            .child(self.kit.keycap("esc"))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.pop_card(cx);
            }));
        let mut primary_body = div()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .child(primary_row);
        if staged
            .selected()
            .is_some_and(|display| staged.conflicts(&display.id))
        {
            primary_body = primary_body.child(SettingsFeedback::new(
                "This display overlaps another",
                palette.status_danger,
                true,
            ));
        }
        if let Some(error) = staged.error() {
            primary_body = primary_body.child(SettingsFeedback::new(
                error.to_string(),
                palette.status_danger,
                true,
            ));
        }
        let slider = staged.bindings().slider.clone();
        let brightness_ground = RowGround::of(
            display_layout_row_selected(self.level().selected, DISPLAY_LAYOUT_BRIGHTNESS_ROW),
            self.body_has_focus(),
        );
        let brightness_body = match slider.as_ref() {
            Some(spec) => {
                let selected_display = staged.selected();
                let brightness = selected_display.and_then(|display| display.brightness);
                let value_element = match brightness {
                    Some(value) => {
                        let value = f64::from(value);
                        let fraction = slider_fraction(value, Some(spec.min), Some(spec.max));
                        let percent = slider_percent_label(spec.min, spec.max, value);
                        let display_id = selected_display
                            .map(|display| display.id.clone())
                            .unwrap_or_default();
                        let track_id = display_id.clone();
                        let down_id = display_id.clone();
                        let move_id = display_id;
                        let track_row = self
                            .level()
                            .origin_row
                            .unwrap_or(DISPLAY_LAYOUT_BRIGHTNESS_ROW);
                        slider_track(
                            cx,
                            track_row,
                            track_id,
                            SliderTrackStyle {
                                fraction,
                                percent,
                                ground: brightness_ground,
                                palette,
                            },
                            move |panel: &mut SettingsPanelView,
                                  _row: usize,
                                  fraction: f32,
                                  cx: &mut Context<SettingsPanelView>| {
                                panel.set_display_layout_brightness(&down_id, fraction, cx);
                            },
                            move |panel: &mut SettingsPanelView,
                                  _row: usize,
                                  fraction: f32,
                                  cx: &mut Context<SettingsPanelView>| {
                                panel.set_display_layout_brightness(&move_id, fraction, cx);
                                cx.notify();
                            },
                            |_: &mut SettingsPanelView,
                             _: usize,
                             _: &mut Context<SettingsPanelView>| {},
                        )
                        .into_any_element()
                    }
                    None => settings_description("unavailable", brightness_ground, palette)
                        .into_any_element(),
                };
                SettingsRow::rule(("settings-display-layout-brightness", 0usize), palette)
                    .selected(
                        display_layout_row_selected(
                            self.level().selected,
                            DISPLAY_LAYOUT_BRIGHTNESS_ROW,
                        ),
                        self.body_has_focus(),
                    )
                    .child(settings_label_group(
                        "Brightness",
                        None,
                        brightness_ground,
                        palette,
                    ))
                    .child(value_element)
                    .into_any_element()
            }
            None => div().into_any_element(),
        };
        let rows = [
            (DISPLAY_LAYOUT_STAGE_ROW, stage_row.into_any_element()),
            (DISPLAY_LAYOUT_BRIGHTNESS_ROW, brightness_body),
            (
                DISPLAY_LAYOUT_RESOLUTION_ROW,
                mode_control.into_any_element(),
            ),
            (DISPLAY_LAYOUT_PRIMARY_ROW, primary_body.into_any_element()),
            (DISPLAY_LAYOUT_APPLY_ROW, apply_row.into_any_element()),
            (DISPLAY_LAYOUT_CANCEL_ROW, cancel_row.into_any_element()),
        ];
        let visible = self.current_visible_rows();
        rows.into_iter()
            .filter(|(index, _)| visible.contains(index))
            .map(|(_, row)| row)
            .collect()
    }
}

fn watch_display_layout_drag(window: &mut Window, view: WeakEntity<SettingsPanelView>) {
    let move_view = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let _ = move_view.update(cx, |panel, cx| {
            if panel.display_layout_press.is_none() {
                return;
            }
            if event.dragging() {
                if panel.drag_display_layout_to(event.position) {
                    cx.notify();
                }
            } else {
                panel.finish_display_layout_drag();
                cx.notify();
            }
        });
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        let _ = view.update(cx, |panel, cx| {
            if panel.display_layout_press.is_some() {
                panel.finish_display_layout_drag();
                cx.notify();
            }
        });
    });
}

const DISPLAY_LAYOUT_STAGE_ROW: usize = 0;
const DISPLAY_LAYOUT_BRIGHTNESS_ROW: usize = 1;
const DISPLAY_LAYOUT_RESOLUTION_ROW: usize = 2;
const DISPLAY_LAYOUT_PRIMARY_ROW: usize = 3;
const DISPLAY_LAYOUT_APPLY_ROW: usize = 4;
const DISPLAY_LAYOUT_CANCEL_ROW: usize = 5;
const RESOLUTION_LABEL: &str = "Resolution and refresh";
const RESOLUTION_CARD_DESCRIPTION: &str = "Size and refresh for this display.";

pub(super) fn display_mode_tiles(state: &DisplayLayoutState) -> Vec<ChooseTile> {
    let staged_token = state
        .selected_id()
        .and_then(|id| state.staged_mode(id))
        .map(|mode| mode.token);
    state
        .modes_for_selected()
        .into_iter()
        .map(|option| {
            let (name, detail) = label_parts(&option.label);
            ChooseTile {
                value: Some(option.token.to_string()),
                name: name.to_string(),
                detail: detail.map(str::to_string),
                art: TileArt::Picture(format!("display-mode:{}x{}", option.width, option.height)),
                saved: staged_token.map_or(option.current, |token| token == option.token),
            }
        })
        .collect()
}

fn display_layout_row_selected(selected: usize, index: usize) -> bool {
    selected == index
}

fn display_layout_card_row_indices(has_slider: bool) -> Vec<usize> {
    let mut rows = vec![DISPLAY_LAYOUT_STAGE_ROW];
    if has_slider {
        rows.push(DISPLAY_LAYOUT_BRIGHTNESS_ROW);
    }
    rows.extend([
        DISPLAY_LAYOUT_RESOLUTION_ROW,
        DISPLAY_LAYOUT_PRIMARY_ROW,
        DISPLAY_LAYOUT_APPLY_ROW,
        DISPLAY_LAYOUT_CANCEL_ROW,
    ]);
    rows
}

fn display_layout_card_row(id: &str, label: &str, source: usize) -> Row {
    Row {
        id: id.to_string(),
        section_id: None,
        section_label: None,
        label: label.to_string(),
        description: None,
        placeholder: None,
        variant: None,
        config_key: String::new(),
        default: qol_config::contract::FieldDefault::String(String::new()),
        stream: None,
        action: None,
        visibility: None,
        source,
        control: RowControl::Text(String::new()),
    }
}

fn display_layout_card_level(
    label: &str,
    source: usize,
    state: DisplayLayoutState,
    origin_row: usize,
    destination: SettingsDestination,
    description: Option<String>,
) -> Level {
    let has_slider = state.bindings().slider.is_some();
    let mut rows = vec![display_layout_card_row(
        "display_layout_stage",
        label,
        source,
    )];
    rows.push(display_layout_card_row(
        "display_layout_brightness",
        "Brightness",
        source,
    ));
    rows.push(display_layout_card_row(
        "display_layout_mode",
        RESOLUTION_LABEL,
        source,
    ));
    rows.push(display_layout_card_row(
        "display_layout_primary",
        "Primary display",
        source,
    ));
    rows.push(display_layout_card_row(
        "display_layout_apply",
        "Apply",
        source,
    ));
    rows.push(display_layout_card_row(
        "display_layout_cancel",
        "Cancel",
        source,
    ));
    let section = RowSection {
        label: label.to_string(),
        description,
        rows: display_layout_card_row_indices(has_slider),
        source,
    };
    let row_bounds = (0..rows.len()).map(|_| Rc::new(Cell::new(None))).collect();
    Level {
        rows,
        sections: vec![section],
        selected: DISPLAY_LAYOUT_STAGE_ROW,
        active_section: None,
        selected_section: 0,
        body_scroll: crate::scroll_list::SelectionScroll::new(),
        active_control: None,
        row_bounds,
        header: LevelHeader::Card(destination),
        origin_row: Some(origin_row),
        object_array: None,
        display_layout: Some(state),
        list_card: false,
        live_card: false,
        choose: None,
        entries: None,
        form: None,
        list_item: None,
    }
}

fn display_layout_card_sync(root_rows: &mut [Row], level: &mut Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(RowControl::DisplayLayout(fresh)) = root_rows.get(origin_row).map(|row| &row.control)
    else {
        return;
    };
    let Some(card) = level.display_layout.as_mut() else {
        return;
    };
    card.adopt_fresh(fresh);
}

fn display_layout_card_commit(root_rows: &mut [Row], level: &Level) {
    let Some(origin_row) = level.origin_row else {
        return;
    };
    let Some(card) = level.display_layout.as_ref() else {
        return;
    };
    let Some(RowControl::DisplayLayout(root)) =
        root_rows.get_mut(origin_row).map(|row| &mut row.control)
    else {
        return;
    };
    **root = card.clone();
}

fn display_layout_dispatch_steps(state: &DisplayLayoutState) -> Vec<(String, serde_json::Value)> {
    let bindings = state.bindings();
    let mut steps = Vec::new();
    for intent in state.pending_mode_intents() {
        let action = match intent.action() {
            "set_mode" => bindings
                .active_action
                .clone()
                .unwrap_or_else(|| intent.action().to_string()),
            _ => bindings.action.clone(),
        };
        steps.push((action, intent.input()));
    }
    if let Some(intent) = state.arrange_intent() {
        steps.push((bindings.action.clone(), intent.input()));
    }
    steps
}

fn display_layout_escape(level: &mut Level) -> bool {
    let Some(state) = level.display_layout.as_mut() else {
        return false;
    };
    state.discard_staged()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayLayoutCardAction {
    Nudge(i32, i32),
    CycleDisplays(i32),
    ToggleEditing,
    OpenModePicker,
    ChoosePrimary,
    CyclePrimary(i32),
    MoveSelection(i32),
    StepBrightness(i32),
    Consume,
    Apply,
    Pop,
    LeaveEditing,
    Discard,
    FallThrough,
}

fn display_layout_card_action(
    selected: usize,
    editing: bool,
    key: &str,
    shift: bool,
) -> DisplayLayoutCardAction {
    if key == "escape" && editing {
        return DisplayLayoutCardAction::LeaveEditing;
    }
    if editing && selected == DISPLAY_LAYOUT_STAGE_ROW {
        let step = nudge_step(shift);
        match key {
            "up" => return DisplayLayoutCardAction::Nudge(0, -step),
            "down" => return DisplayLayoutCardAction::Nudge(0, step),
            "left" => return DisplayLayoutCardAction::Nudge(-step, 0),
            "right" => return DisplayLayoutCardAction::Nudge(step, 0),
            "tab" => return DisplayLayoutCardAction::CycleDisplays(if shift { -1 } else { 1 }),
            _ => {}
        }
    }
    match key {
        "up" => DisplayLayoutCardAction::MoveSelection(-1),
        "down" => DisplayLayoutCardAction::MoveSelection(1),
        "enter" | "return" | "space" => match selected {
            DISPLAY_LAYOUT_STAGE_ROW => DisplayLayoutCardAction::ToggleEditing,
            DISPLAY_LAYOUT_BRIGHTNESS_ROW => DisplayLayoutCardAction::Consume,
            DISPLAY_LAYOUT_RESOLUTION_ROW => DisplayLayoutCardAction::OpenModePicker,
            DISPLAY_LAYOUT_PRIMARY_ROW => DisplayLayoutCardAction::ChoosePrimary,
            DISPLAY_LAYOUT_APPLY_ROW => DisplayLayoutCardAction::Apply,
            DISPLAY_LAYOUT_CANCEL_ROW => DisplayLayoutCardAction::Pop,
            _ => DisplayLayoutCardAction::FallThrough,
        },
        "left" | "right" if selected == DISPLAY_LAYOUT_BRIGHTNESS_ROW => {
            DisplayLayoutCardAction::StepBrightness(if key == "left" { -1 } else { 1 })
        }
        "left" | "right" if selected == DISPLAY_LAYOUT_PRIMARY_ROW => {
            DisplayLayoutCardAction::CyclePrimary(if key == "left" { -1 } else { 1 })
        }
        "escape" => DisplayLayoutCardAction::Discard,
        _ => DisplayLayoutCardAction::FallThrough,
    }
}

pub(super) fn enter_hint(selected: usize) -> Option<&'static str> {
    match selected {
        DISPLAY_LAYOUT_STAGE_ROW => Some("edit"),
        DISPLAY_LAYOUT_RESOLUTION_ROW => Some("choose"),
        DISPLAY_LAYOUT_PRIMARY_ROW => Some("choose"),
        DISPLAY_LAYOUT_APPLY_ROW => Some("apply"),
        DISPLAY_LAYOUT_CANCEL_ROW => Some("back"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::display_layout::{DisplayLayoutBindings, DisplayLayoutState};
    use super::super::super::form_nav::adjacent_visible_row;
    use super::super::super::rows::{Row, RowControl};
    use super::super::super::SettingsDestination;
    use super::super::tests::{level, rows, source_section};
    use crate::settings_panel::components::TileArt;

    fn brightness_slider() -> qol_config::contract::RowSliderSpec {
        serde_json::from_value(serde_json::json!({
            "value_from": "brightness",
            "min": 0,
            "max": 100,
            "step": 5,
            "action": "set_brightness",
            "input": { "id": "{id}", "value": "{value}" },
        }))
        .expect("brightness slider spec")
    }

    fn display_layout_state() -> DisplayLayoutState {
        let mut state = DisplayLayoutState::new(DisplayLayoutBindings::new(
            "layout",
            Some("modes".to_string()),
            "arrange",
            Some("set_mode".to_string()),
            Some(brightness_slider()),
        ));
        state.set_viewport(
            624.0,
            super::super::super::PANEL_DISPLAY_LAYOUT_HEIGHT,
            super::DISPLAY_LAYOUT_STAGE_PAD,
        );
        state.load_layout(&serde_json::json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "refresh_hz": 60,
                "primary": true,
                "brightness": 80,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 3840,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": false,
                "brightness": 40,
            }
        ]));
        state
    }

    fn display_layout_state_without_slider() -> DisplayLayoutState {
        let mut state = DisplayLayoutState::new(DisplayLayoutBindings::new(
            "layout",
            Some("modes".to_string()),
            "arrange",
            Some("set_mode".to_string()),
            None,
        ));
        state.set_viewport(
            624.0,
            super::super::super::PANEL_DISPLAY_LAYOUT_HEIGHT,
            super::DISPLAY_LAYOUT_STAGE_PAD,
        );
        state.load_layout(&serde_json::json!([{
            "id": "alpha",
            "connector": "card0-DP-1",
            "x": 0,
            "y": 0,
            "width": 3840,
            "height": 2160,
            "refresh_hz": 60,
            "primary": true,
        }]));
        state
    }

    fn display_layout_row() -> Row {
        let mut row = rows(&[false]).remove(0);
        row.id = "arrangement".into();
        row.label = "Arrangement".into();
        row.control = RowControl::DisplayLayout(Box::new(display_layout_state()));
        row
    }

    fn display_layout_mode_rows() -> serde_json::Value {
        serde_json::json!([{
            "id": "beta#7",
            "display_id": "beta",
            "connector": "card0-DP-2",
            "token": 7,
            "width": 2560,
            "height": 1440,
            "refresh_hz": 60,
            "label": "2560x1440@60",
            "detail": "available mode",
            "current": false,
            "writable": true,
            "selectable": true,
        }])
    }

    #[test]
    fn the_slider_binding_decides_whether_the_brightness_row_exists() {
        let with_slider = super::display_layout_card_level(
            "Displays",
            0,
            display_layout_state(),
            0,
            SettingsDestination::from_static("Displays"),
            None,
        );
        assert_eq!(with_slider.sections[0].rows.len(), 6);
        assert_eq!(with_slider.sections[0].rows, vec![0, 1, 2, 3, 4, 5]);
        let mut selected = super::DISPLAY_LAYOUT_STAGE_ROW;
        let mut walked = vec![selected];
        for _ in 0..5 {
            selected = adjacent_visible_row(&with_slider.sections[0].rows, selected, 1);
            walked.push(selected);
        }
        assert_eq!(
            walked,
            vec![0, 1, 2, 3, 4, 5],
            "navigation walks every row in order"
        );

        let without_slider = super::display_layout_card_level(
            "Displays",
            0,
            display_layout_state_without_slider(),
            0,
            SettingsDestination::from_static("Displays"),
            None,
        );
        assert_eq!(without_slider.sections[0].rows.len(), 5);
        assert_eq!(
            without_slider.sections[0].rows,
            vec![0, 2, 3, 4, 5],
            "the brightness row is skipped without a slider binding"
        );
        assert_eq!(
            adjacent_visible_row(
                &without_slider.sections[0].rows,
                super::DISPLAY_LAYOUT_STAGE_ROW,
                1
            ),
            super::DISPLAY_LAYOUT_RESOLUTION_ROW
        );
    }

    #[test]
    fn a_display_layout_card_level_exposes_six_rows_and_starts_on_the_stage() {
        let mut state = display_layout_state();
        state.load_modes(&display_layout_mode_rows());
        let card = super::display_layout_card_level(
            "Displays",
            2,
            state,
            4,
            SettingsDestination::from_static("Displays"),
            None,
        );
        assert_eq!(card.origin_row, Some(4));
        assert_eq!(card.selected, super::DISPLAY_LAYOUT_STAGE_ROW);
        assert_eq!(card.rows.len(), 6);
        let labels: Vec<&str> = card.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Displays",
                "Brightness",
                "Resolution and refresh",
                "Primary display",
                "Apply",
                "Cancel"
            ]
        );
        let ids: Vec<&str> = card.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "display_layout_stage",
                "display_layout_brightness",
                "display_layout_mode",
                "display_layout_primary",
                "display_layout_apply",
                "display_layout_cancel",
            ]
        );
        assert!(card.rows.iter().all(|row| row.source == 2));
        for row in &card.rows {
            assert!(matches!(&row.control, RowControl::Text(value) if value.is_empty()));
        }
        assert_eq!(card.sections.len(), 1);
        assert_eq!(card.sections[0].rows, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(card.row_bounds.len(), 6);
        let state = card.display_layout.as_ref().expect("card state");
        assert_eq!(state.displays().len(), 2);
        assert_eq!(state.modes_for("beta").len(), 1);
        assert_eq!(super::super::card_enter_hint(&card), Some("edit"));
    }

    #[test]
    fn only_the_selected_card_row_paints_selected() {
        let card = super::display_layout_card_level(
            "Arrangement",
            0,
            display_layout_state(),
            0,
            SettingsDestination::from_static("Arrangement"),
            None,
        );
        for selected in 0..card.rows.len() {
            let painted: Vec<usize> = (0..card.rows.len())
                .filter(|index| super::display_layout_row_selected(selected, *index))
                .collect();
            assert_eq!(painted, vec![selected]);
        }
    }

    #[test]
    fn up_and_down_move_the_card_selection_until_the_stage_is_editing() {
        let card = super::display_layout_card_level(
            "Arrangement",
            0,
            display_layout_state(),
            0,
            SettingsDestination::from_static("Arrangement"),
            None,
        );
        let visible = card.sections[0].rows.clone();
        assert_eq!(
            adjacent_visible_row(&visible, card.selected, 1),
            super::DISPLAY_LAYOUT_BRIGHTNESS_ROW
        );
        assert_eq!(
            adjacent_visible_row(&visible, card.selected, -1),
            super::DISPLAY_LAYOUT_STAGE_ROW
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, false, "down", false),
            super::DisplayLayoutCardAction::MoveSelection(1)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, false, "up", false),
            super::DisplayLayoutCardAction::MoveSelection(-1)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "down", false),
            super::DisplayLayoutCardAction::Nudge(0, 10)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "down", true),
            super::DisplayLayoutCardAction::Nudge(0, 1)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "up", false),
            super::DisplayLayoutCardAction::Nudge(0, -10)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "left", false),
            super::DisplayLayoutCardAction::Nudge(-10, 0)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "right", true),
            super::DisplayLayoutCardAction::Nudge(1, 0)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "tab", false),
            super::DisplayLayoutCardAction::CycleDisplays(1)
        );
        assert_eq!(
            super::display_layout_card_action(card.selected, true, "tab", true),
            super::DisplayLayoutCardAction::CycleDisplays(-1)
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_RESOLUTION_ROW,
                true,
                "down",
                false,
            ),
            super::DisplayLayoutCardAction::MoveSelection(1),
            "edit mode only nudges while the stage row is selected"
        );
    }

    #[test]
    fn the_display_layout_card_walks_the_six_rows_without_wrapping() {
        let mut card = super::display_layout_card_level(
            "Displays",
            0,
            display_layout_state(),
            0,
            SettingsDestination::from_static("Displays"),
            None,
        );
        let down = super::DisplayLayoutCardAction::MoveSelection(1);
        let rows = card.sections[0].rows.clone();
        for expected in [
            super::DISPLAY_LAYOUT_BRIGHTNESS_ROW,
            super::DISPLAY_LAYOUT_RESOLUTION_ROW,
            super::DISPLAY_LAYOUT_PRIMARY_ROW,
            super::DISPLAY_LAYOUT_APPLY_ROW,
            super::DISPLAY_LAYOUT_CANCEL_ROW,
        ] {
            assert_eq!(
                super::display_layout_card_action(card.selected, false, "down", false),
                down
            );
            card.selected = adjacent_visible_row(&rows, card.selected, 1);
            assert_eq!(card.selected, expected);
            let painted: Vec<usize> = (0..card.rows.len())
                .filter(|index| super::display_layout_row_selected(card.selected, *index))
                .collect();
            assert_eq!(painted, vec![expected]);
        }
        assert_eq!(
            super::display_layout_card_action(card.selected, false, "down", false),
            down
        );
        assert_eq!(
            adjacent_visible_row(&rows, card.selected, 1),
            super::DISPLAY_LAYOUT_CANCEL_ROW,
            "down at the cancel row stays"
        );
        assert_eq!(
            super::display_layout_card_action(super::DISPLAY_LAYOUT_STAGE_ROW, false, "up", false,),
            super::DisplayLayoutCardAction::MoveSelection(-1)
        );
        assert_eq!(
            adjacent_visible_row(&rows, super::DISPLAY_LAYOUT_STAGE_ROW, -1),
            super::DISPLAY_LAYOUT_STAGE_ROW,
            "up at the stage row stays"
        );
    }

    #[test]
    fn enter_on_a_card_row_targets_that_row() {
        let cases = [
            (
                super::DISPLAY_LAYOUT_STAGE_ROW,
                super::DisplayLayoutCardAction::ToggleEditing,
            ),
            (
                super::DISPLAY_LAYOUT_BRIGHTNESS_ROW,
                super::DisplayLayoutCardAction::Consume,
            ),
            (
                super::DISPLAY_LAYOUT_RESOLUTION_ROW,
                super::DisplayLayoutCardAction::OpenModePicker,
            ),
            (
                super::DISPLAY_LAYOUT_PRIMARY_ROW,
                super::DisplayLayoutCardAction::ChoosePrimary,
            ),
            (
                super::DISPLAY_LAYOUT_APPLY_ROW,
                super::DisplayLayoutCardAction::Apply,
            ),
            (
                super::DISPLAY_LAYOUT_CANCEL_ROW,
                super::DisplayLayoutCardAction::Pop,
            ),
        ];
        for (selected, expected) in cases {
            for key in ["enter", "return", "space"] {
                assert_eq!(
                    super::display_layout_card_action(selected, false, key, false),
                    expected,
                    "key: {key} row: {selected}"
                );
            }
        }
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_STAGE_ROW,
                true,
                "enter",
                false,
            ),
            super::DisplayLayoutCardAction::ToggleEditing
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_BRIGHTNESS_ROW,
                false,
                "left",
                false,
            ),
            super::DisplayLayoutCardAction::StepBrightness(-1)
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_BRIGHTNESS_ROW,
                false,
                "right",
                false,
            ),
            super::DisplayLayoutCardAction::StepBrightness(1)
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_PRIMARY_ROW,
                false,
                "left",
                false,
            ),
            super::DisplayLayoutCardAction::CyclePrimary(-1)
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_PRIMARY_ROW,
                false,
                "right",
                false,
            ),
            super::DisplayLayoutCardAction::CyclePrimary(1)
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_RESOLUTION_ROW,
                false,
                "right",
                false,
            ),
            super::DisplayLayoutCardAction::FallThrough,
            "the panel has no horizontal select convention to follow"
        );
    }

    #[test]
    fn escape_leaves_card_edit_mode_before_discarding() {
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_STAGE_ROW,
                true,
                "escape",
                false,
            ),
            super::DisplayLayoutCardAction::LeaveEditing
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_RESOLUTION_ROW,
                true,
                "escape",
                false,
            ),
            super::DisplayLayoutCardAction::LeaveEditing
        );
        assert_eq!(
            super::display_layout_card_action(
                super::DISPLAY_LAYOUT_STAGE_ROW,
                false,
                "escape",
                false,
            ),
            super::DisplayLayoutCardAction::Discard
        );
        let mut state = display_layout_state();
        assert!(!state.editing());
        state.set_editing(true);
        assert!(state.editing());
        state.set_editing(false);
        assert!(!state.editing());
    }

    #[test]
    fn the_card_enter_hint_follows_the_selected_row() {
        let cases = [
            (super::DISPLAY_LAYOUT_STAGE_ROW, Some("edit")),
            (super::DISPLAY_LAYOUT_BRIGHTNESS_ROW, None),
            (super::DISPLAY_LAYOUT_RESOLUTION_ROW, Some("choose")),
            (super::DISPLAY_LAYOUT_PRIMARY_ROW, Some("choose")),
            (super::DISPLAY_LAYOUT_APPLY_ROW, Some("apply")),
            (super::DISPLAY_LAYOUT_CANCEL_ROW, Some("back")),
        ];
        for (selected, expected) in cases {
            let mut card = super::display_layout_card_level(
                "Displays",
                0,
                display_layout_state(),
                0,
                SettingsDestination::from_static("Displays"),
                None,
            );
            card.selected = selected;
            assert_eq!(
                super::super::card_enter_hint(&card),
                expected,
                "row: {selected}"
            );
        }
    }

    #[test]
    fn escape_discards_staged_edits_before_popping() {
        let mut card = level(0);
        let mut state = display_layout_state();
        assert!(state.select("beta"));
        assert!(state.nudge(0, 12));
        card.display_layout = Some(state);
        assert!(
            super::display_layout_escape(&mut card),
            "the first escape discards the staged edits"
        );
        let card_state = card.display_layout.as_ref().expect("card state");
        assert!(!card_state.has_staged_edits());
        assert!(
            !super::display_layout_escape(&mut card),
            "the second escape falls through to the pop"
        );
    }

    #[test]
    fn apply_dispatches_arrange_and_set_mode_payloads() {
        let mut state = display_layout_state();
        state.load_modes(&display_layout_mode_rows());
        let option = state.modes_for("beta").remove(0);
        assert!(state.stage_mode(&option).is_some());
        assert!(state.select("beta"));
        assert!(state.nudge(0, -10));
        let steps = super::display_layout_dispatch_steps(&state);
        assert_eq!(
            steps,
            vec![
                (
                    "set_mode".to_string(),
                    serde_json::json!({
                        "id": "beta",
                        "token": 7,
                        "width": 2560,
                        "height": 1440,
                        "refresh": 60,
                    })
                ),
                (
                    "arrange".to_string(),
                    serde_json::json!({
                        "placements": [
                            { "id": "alpha", "x": 0, "y": 10 },
                            { "id": "beta", "x": 3840, "y": 0 },
                        ],
                        "primary": "alpha",
                    })
                ),
            ]
        );
    }

    #[test]
    fn cancel_leaves_the_root_state_untouched() {
        let mut root = level(0);
        root.rows = vec![display_layout_row()];
        root.sections = vec![source_section("Arrangement", 0, vec![0])];
        let mut card_state = display_layout_state();
        assert!(card_state.select("beta"));
        assert!(card_state.nudge(0, 33));
        let card = super::display_layout_card_level(
            "Arrangement",
            0,
            card_state,
            0,
            SettingsDestination::from_static("Arrangement"),
            None,
        );
        let mut stack = vec![root, card];
        assert!(super::super::pop_level(&mut stack).is_some());
        let RowControl::DisplayLayout(root_state) = &stack[0].rows[0].control else {
            panic!("expected the root display layout row");
        };
        let beta = root_state
            .displays()
            .iter()
            .find(|display| display.id == "beta")
            .expect("beta");
        assert_eq!(beta.y, 0, "cancel never writes the staged edit");
        assert!(root_state.staged_mode("beta").is_none());
    }

    #[test]
    fn a_successful_apply_replaces_the_committed_root_baseline() {
        let mut root = vec![display_layout_row()];
        let mut card_state = display_layout_state();
        assert!(card_state.select("beta"));
        assert!(card_state.nudge(0, 33));
        card_state.finish_commit();
        let card = super::display_layout_card_level(
            "Arrangement",
            0,
            card_state,
            0,
            SettingsDestination::from_static("Arrangement"),
            None,
        );
        super::display_layout_card_commit(&mut root, &card);
        let RowControl::DisplayLayout(root_state) = &root[0].control else {
            panic!("expected the root display layout row");
        };
        let beta = root_state
            .displays()
            .iter()
            .find(|display| display.id == "beta")
            .expect("beta");
        assert_eq!(beta.y, 33);
        assert!(!root_state.has_staged_edits());
    }

    #[test]
    fn a_layout_poll_merges_fresh_geometry_into_the_open_card() {
        let mut root = vec![display_layout_row()];
        let mut card_state = display_layout_state();
        assert!(card_state.select("beta"));
        assert!(card_state.nudge(0, 21));
        let mut card = super::display_layout_card_level(
            "Arrangement",
            0,
            card_state,
            0,
            SettingsDestination::from_static("Arrangement"),
            None,
        );
        let RowControl::DisplayLayout(fresh) = &mut root[0].control else {
            panic!("expected the root display layout row");
        };
        fresh.load_layout(&serde_json::json!([
            {
                "id": "alpha",
                "connector": "card0-DP-1",
                "x": 0,
                "y": 0,
                "width": 3840,
                "height": 2160,
                "refresh_hz": 60,
                "primary": true,
            },
            {
                "id": "beta",
                "connector": "card0-DP-2",
                "x": 2000,
                "y": 0,
                "width": 1920,
                "height": 1080,
                "refresh_hz": 60,
                "primary": false,
                "brightness": 55,
            }
        ]));
        super::display_layout_card_sync(&mut root, &mut card);
        let state = card.display_layout.as_ref().expect("card state");
        let beta = state
            .displays()
            .iter()
            .find(|display| display.id == "beta")
            .expect("beta");
        assert_eq!(beta.x, 2000, "fresh geometry lands");
        assert_eq!(state.rect_of(beta).y, 21, "the staged edit survives");
        assert_eq!(beta.brightness, Some(55), "brightness follows the poll");
    }

    #[test]
    fn display_mode_tiles_tick_the_staged_mode_else_the_current_one() {
        let mut state = display_layout_state();
        state.load_modes(&serde_json::json!([
            {
                "id": "alpha#11",
                "display_id": "alpha",
                "connector": "card0-DP-1",
                "token": 11,
                "width": 3840,
                "height": 2160,
                "refresh_hz": 60,
                "label": "3840x2160 \u{00b7} 60 Hz",
                "detail": "current mode",
                "current": true,
                "writable": true,
                "selectable": false,
            },
            {
                "id": "alpha#12",
                "display_id": "alpha",
                "connector": "card0-DP-1",
                "token": 12,
                "width": 2560,
                "height": 1440,
                "refresh_hz": 165,
                "label": "2560x1440 \u{00b7} 165 Hz",
                "detail": "available mode",
                "current": false,
                "writable": true,
                "selectable": true,
            }
        ]));
        let tiles = super::display_mode_tiles(&state);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].value.as_deref(), Some("11"));
        assert_eq!(tiles[0].name, "3840x2160");
        assert_eq!(tiles[0].detail.as_deref(), Some("60 Hz"));
        assert_eq!(
            tiles[0].art,
            TileArt::Picture("display-mode:3840x2160".to_string())
        );
        assert!(tiles[0].saved);
        assert!(!tiles[1].saved, "the current mode opens ticked");
        let option = state.modes_for("alpha").remove(1);
        assert!(state.stage_mode(&option).is_some());
        let tiles = super::display_mode_tiles(&state);
        assert!(!tiles[0].saved);
        assert!(tiles[1].saved, "the staged mode takes the tick");
    }
}
