mod model;

use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, App, AsyncApp, Context, Div, FocusHandle, Focusable, KeyDownEvent, Render,
    Subscription, Task, WeakEntity, Window,
};
use qol_config::contract::PagesSpec;
use qol_config::pages::{
    Control, Enter, Go, Notice, Page, Pages, Press, PressReply, Row, Stack, Tone,
};

use crate::deck;
use crate::key::Key;
use crate::kit::{kit, Chip};
use crate::scrollbar::{ScrollSource, OVERFLOW_FADE_HEIGHT};

use super::components::{
    settings_busy_message, settings_label_group, settings_message, settings_page,
    settings_value_group, RowGround, SettingsChevron, SettingsFeedback, SettingsHint, SettingsTick,
    SettingsToggle,
};
use super::{
    escape_step, intent, settings_action_affordance, settings_action_spinner, settings_value_text,
    CustomHints, CustomPanelCallback, CustomPanelContext, CustomPanelFactory,
    CustomPanelNoticeTone, CustomPanelNotifier, CustomPanelView, CustomSettingsBreadcrumbs,
    EscapeStep, Intent, SettingsDestination, SettingsGroupHeader, SettingsRow, SettingsRuntime,
    SettingsValueTone,
};
use model::{Item, Moved, Place};

const ANIMATION: &str = "settings-pages-card";
const FRONT_IDS: [&str; 4] = [
    "settings-pages-body",
    "settings-pages-row",
    "settings-pages-spinner",
    "settings-pages-action",
];
const CLOSING_IDS: [&str; 4] = [
    "settings-pages-closing-body",
    "settings-pages-closing-row",
    "settings-pages-closing-spinner",
    "settings-pages-closing-action",
];

pub(super) fn factory(
    heading: String,
    spec: PagesSpec,
    runtime: SettingsRuntime,
) -> CustomPanelFactory {
    Rc::new(move |context: CustomPanelContext, cx: &mut App| {
        let CustomPanelContext {
            on_back,
            notify,
            on_change,
            ..
        } = context;
        let (heading, spec, runtime) = (heading.clone(), spec.clone(), runtime.clone());
        let view = cx.new(|cx| PagesView::new(heading, spec, runtime, on_back, notify, cx));
        CustomPanelView::new(view, on_change, cx)
    })
}

pub(super) struct PagesView {
    focus: FocusHandle,
    heading: String,
    spec: PagesSpec,
    runtime: SettingsRuntime,
    on_back: CustomPanelCallback,
    notify: CustomPanelNotifier,
    pages: Option<Pages>,
    failure: Option<String>,
    stack: Vec<Place>,
    closing: Option<(Place, usize)>,
    step: usize,
    motion: Option<deck::Motion>,
    body_bounds: deck::BodyBounds,
    row_bounds: deck::RowBounds,
    sequence: u64,
    asked: bool,
    loading: bool,
    visiting: bool,
    pressing: Option<(String, String)>,
    poller: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
    opened: Instant,
}

impl PagesView {
    fn new(
        heading: String,
        spec: PagesSpec,
        runtime: SettingsRuntime,
        on_back: CustomPanelCallback,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                cx.background_executor()
                    .timer(qol_theme::WAIT_BEFORE_BUSY)
                    .await;
                let _ = this.update(&mut cx, |_, cx| cx.notify());
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            heading,
            spec,
            runtime,
            on_back,
            notify,
            pages: None,
            failure: None,
            stack: Vec::new(),
            closing: None,
            step: 0,
            motion: None,
            body_bounds: Default::default(),
            row_bounds: Default::default(),
            sequence: 0,
            asked: false,
            loading: false,
            visiting: false,
            pressing: None,
            poller: None,
            subscriptions: Vec::new(),
            opened: Instant::now(),
        }
    }

    fn open_pages(&self) -> Vec<String> {
        self.stack
            .iter()
            .chain(self.closing.as_ref().map(|(place, _)| place))
            .map(|place| place.page.clone())
            .collect()
    }

    fn fetch(
        &self,
        open: Vec<String>,
    ) -> impl std::future::Future<Output = Result<Pages, String>> + Send + 'static {
        let runtime = self.runtime.clone();
        let query = self.spec.query.clone();
        let input = serde_json::to_value(Stack { open }).unwrap_or_default();
        async move { runtime.query_with(&query, input).and_then(read_pages) }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.visiting || self.pressing.is_some() {
            return;
        }
        self.loading = true;
        self.asked = true;
        let sequence = self.sequence;
        let fetch = self.fetch(self.open_pages());
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx.background_spawn(fetch).await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.loading = false;
                    if view.sequence == sequence {
                        view.take(result);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn visit(&mut self, go: Go, page: String, cx: &mut Context<Self>) {
        if self.visiting || self.pressing.is_some() {
            return;
        }
        self.visiting = true;
        self.sequence = self.sequence.wrapping_add(1);
        let sequence = self.sequence;
        let mut open = self.open_pages();
        open.push(page.clone());
        let fetch = self.fetch(open);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx.background_spawn(fetch).await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.visiting = false;
                    if view.sequence == sequence {
                        view.arrive(result, &go, &page, cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn arrive(
        &mut self,
        answer: Result<Pages, String>,
        go: &Go,
        page: &str,
        cx: &mut Context<Self>,
    ) {
        let found = answer
            .as_ref()
            .is_ok_and(|pages| pages.page(page).is_some());
        self.take(answer);
        if found {
            self.travel(go, cx);
        } else if self.failure.is_none() {
            (self.notify)(
                CustomPanelNoticeTone::Failure,
                "That page went away before it opened".to_string(),
                cx,
            );
        }
    }

    fn take(&mut self, answer: Result<Pages, String>) {
        match answer {
            Ok(pages) => {
                self.failure = None;
                model::reconcile(&mut self.stack, &pages);
                self.pages = Some(pages);
            }
            Err(failure) => self.failure = Some(failure),
        }
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        if self.poller.is_some() {
            return;
        }
        let interval = self.runtime.query_interval(&self.spec.query);
        self.poller = Some(cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor().timer(interval).await;
                    if this.update(&mut cx, |view, cx| view.load(cx)).is_err() {
                        break;
                    }
                }
            }
        }));
    }

    fn suspend(&mut self, cx: &mut Context<Self>) {
        self.poller = None;
        cx.notify();
    }

    fn press(&mut self, page: String, row: String, cx: &mut Context<Self>) {
        if self.pressing.is_some() {
            return;
        }
        self.pressing = Some((page.clone(), row.clone()));
        self.sequence = self.sequence.wrapping_add(1);
        let runtime = self.runtime.clone();
        let action = self.spec.action.clone();
        let input = serde_json::to_value(Press {
            page,
            row,
            open: self.open_pages(),
        })
        .unwrap_or_default();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_spawn(async move { runtime.run_action(&action, input) })
                    .await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.pressing = None;
                    view.answer(result, cx);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn answer(
        &mut self,
        result: Result<Option<serde_json::Value>, String>,
        cx: &mut Context<Self>,
    ) {
        let reply = result.and_then(|value| match value {
            Some(value) => serde_json::from_value::<PressReply>(value).map_err(|error| {
                format!(
                    "{} answered in a way qol cannot read: {error}",
                    self.heading
                )
            }),
            None => Ok(PressReply::default()),
        });
        let reply = match reply {
            Ok(reply) => reply,
            Err(failure) => {
                (self.notify)(CustomPanelNoticeTone::Failure, failure, cx);
                self.load(cx);
                return;
            }
        };
        let fresh = reply.pages.is_some();
        if let Some(pages) = reply.pages {
            self.failure = None;
            self.pages = Some(pages);
        }
        if let Some(go) = reply.go {
            self.navigate(&go, cx);
        }
        if let Some(pages) = self.pages.as_ref() {
            model::reconcile(&mut self.stack, pages);
        }
        if let Some(notice) = reply.notice {
            let (tone, text) = match notice {
                Notice::Success(text) => (CustomPanelNoticeTone::Success, text),
                Notice::Failure(text) => (CustomPanelNoticeTone::Failure, text),
            };
            (self.notify)(tone, text, cx);
        }
        if !fresh {
            self.load(cx);
        }
    }

    fn navigate(&mut self, go: &Go, cx: &mut Context<Self>) {
        if let Go::Open(page) | Go::Replace(page) = go {
            if self
                .pages
                .as_ref()
                .and_then(|pages| pages.page(page))
                .is_none()
            {
                self.visit(go.clone(), page.clone(), cx);
                return;
            }
        }
        self.travel(go, cx);
    }

    fn travel(&mut self, go: &Go, cx: &mut Context<Self>) {
        self.sequence = self.sequence.wrapping_add(1);
        let mark = self.stack.last().and_then(|place| {
            deck::row_mark(&self.row_bounds, place.index, self.body_bounds.get())
        });
        let depth = self.stack.len().saturating_sub(1);
        match model::go(&mut self.stack, go, mark) {
            Moved::Pushed => {
                self.step = self.step.wrapping_add(1);
                self.motion = Some(deck::Motion::Push);
                self.closing = None;
            }
            Moved::Popped(mut gone) => {
                if let Some(top) = gone.pop() {
                    self.leave(top, depth, cx);
                }
            }
            Moved::Stayed => {}
        }
    }

    fn leave(&mut self, place: Place, depth: usize, cx: &mut Context<Self>) {
        self.step = self.step.wrapping_add(1);
        self.motion = None;
        self.closing = Some((place, depth));
        if let Some(front) = self.stack.last() {
            front.scroll.refollow();
        }
        let step = self.step;
        deck::after_transition(cx, move |view, cx| {
            if view.step == step {
                view.closing = None;
                cx.notify();
            }
        });
    }

    fn back_to(&mut self, level: usize, cx: &mut Context<Self>) {
        self.sequence = self.sequence.wrapping_add(1);
        let depth = self.stack.len().saturating_sub(1);
        if let Moved::Popped(mut gone) = model::pop_to(&mut self.stack, level) {
            if let Some(top) = gone.pop() {
                self.leave(top, depth, cx);
            }
        }
    }

    fn front(&self) -> Option<(&Place, &Page)> {
        let place = self.stack.last()?;
        let page = self.pages.as_ref()?.page(&place.page)?;
        Some((place, page))
    }

    fn selected_row(&self) -> Option<(String, Row)> {
        let (place, page) = self.front()?;
        place.selected.as_ref()?;
        let row = model::row_at(page, &model::items(page), place.index)?;
        Some((place.page.clone(), row.clone()))
    }

    fn sync_selection(&mut self) {
        let Some(pages) = self.pages.as_ref() else {
            return;
        };
        for place in &mut self.stack {
            let Some(page) = pages.page(&place.page) else {
                continue;
            };
            match model::settle(page, place) {
                Some(index) => {
                    place.index = index;
                    place.selected =
                        model::row_at(page, &model::items(page), index).map(|row| row.id.clone());
                }
                None => place.selected = None,
            }
        }
    }

    fn select(&mut self, index: usize) {
        let Some(pages) = self.pages.as_ref() else {
            return;
        };
        let Some(place) = self.stack.last_mut() else {
            return;
        };
        let Some(page) = pages.page(&place.page) else {
            return;
        };
        let items = model::items(page);
        if let Some(row) = model::row_at(page, &items, index).filter(|row| !row.info) {
            place.index = index;
            place.selected = Some(row.id.clone());
            place.scroll.follow(
                Some(index),
                model::header_of(&items, index),
                px(OVERFLOW_FADE_HEIGHT),
            );
        }
    }

    fn move_selection(&mut self, direction: isize) {
        let Some((place, page)) = self.front() else {
            return;
        };
        let current = place.index;
        match model::step(page, current, direction) {
            Some(next) if next != current => self.select(next),
            _ => {
                place
                    .scroll
                    .nudge(direction, px(qol_theme::HEIGHT_SETTING_ROW * 3.0));
            }
        }
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        let Some((page, row)) = self.selected_row() else {
            return;
        };
        match row.enter {
            Some(Enter::Open(target)) => self.navigate(&Go::Open(target), cx),
            Some(Enter::Replace(target)) => self.navigate(&Go::Replace(target), cx),
            Some(Enter::Back) => self.navigate(&Go::Back, cx),
            Some(Enter::Press) => self.press(page, row.id, cx),
            None => {}
        }
    }

    fn click(&mut self, index: usize, cx: &mut Context<Self>) {
        self.select(index);
        self.activate(cx);
        cx.notify();
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match escape_step(self.stack.len().saturating_sub(1), false, true) {
            EscapeStep::PopCard => {
                self.navigate(&Go::Back, cx);
                cx.notify();
            }
            EscapeStep::CloseFilter | EscapeStep::AscendRail | EscapeStep::Dismiss => {
                self.suspend(cx);
                (self.on_back)(window, cx);
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        match intent(
            event.keystroke.key.as_str(),
            event.keystroke.key_char.as_deref(),
            false,
        ) {
            Some(Intent::Close) => {
                self.escape(window, cx);
                return;
            }
            Some(Intent::Up) => self.move_selection(-1),
            Some(Intent::Down) => self.move_selection(1),
            Some(Intent::Tab) => self.move_selection(if event.keystroke.modifiers.shift {
                -1
            } else {
                1
            }),
            Some(Intent::Activate) => self.activate(cx),
            _ => return,
        }
        cx.notify();
    }

    fn render_row(
        &self,
        place: &Place,
        index: usize,
        row: &Row,
        focused: bool,
        front: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        let [_, row_id, spinner_id, action_id] = if front { FRONT_IDS } else { CLOSING_IDS };
        let selected = front && place.selected.is_some() && place.index == index;
        let ground = RowGround::of(selected, focused);
        let busy = row.busy
            || self
                .pressing
                .as_ref()
                .is_some_and(|(pressed_page, pressed_row)| {
                    *pressed_page == place.page && *pressed_row == row.id
                });
        let mut values = settings_value_group();
        if busy && !matches!(row.control, Some(Control::Button(_))) {
            values = values.child(settings_action_spinner((spinner_id, index), kit));
        }
        if let Some(tag) = &row.tag {
            values = values.child(kit.chip(Chip::Tag(tag.clone().into()), ground.rest(kit)));
        }
        if let Some(value) = &row.value {
            values = values.child(settings_value_text(
                value.clone(),
                tone(row.tone),
                ground,
                kit,
            ));
        }
        values = match &row.control {
            Some(Control::Toggle(on)) => values.child(SettingsToggle::new(*on, ground, kit)),
            Some(Control::Button(label)) => values.child(settings_action_affordance(
                (action_id, index),
                label.clone(),
                None,
                busy,
                ground,
                kit,
            )),
            Some(Control::Check) => values.child(SettingsTick::new(ground, kit)),
            None if matches!(row.enter, Some(Enter::Open(_) | Enter::Replace(_))) => {
                values.child(SettingsChevron::new(ground, kit))
            }
            None => values,
        };
        let mut element = SettingsRow::setting((row_id, index), kit)
            .selected(selected, focused)
            .attention(row.attention)
            .dimmed(row.dim)
            .child(settings_label_group(
                row.label.clone(),
                row.detail.clone().map(Into::into),
                ground,
                kit,
            ))
            .child(values);
        if front {
            element = element.child(deck::bounds_recorder(Rc::clone(&self.row_bounds), index));
            if !row.info {
                element =
                    element.on_click(cx.listener(move |view, _, _, cx| view.click(index, cx)));
            }
        }
        element.into_any_element()
    }

    fn render_page(
        &self,
        place: &Place,
        front: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let kit = kit();
        let frame = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col();
        let Some(page) = self
            .pages
            .as_ref()
            .and_then(|pages| pages.page(&place.page))
        else {
            return frame;
        };
        let items = model::items(page);
        let selected = place.selected.as_ref().map(|_| place.index);
        let cursor = selected.and_then(|index| model::header_of(&items, index));
        let children: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Header(group) => {
                    let group = page.groups.get(*group)?;
                    Some(
                        SettingsGroupHeader::new(
                            group.title.clone(),
                            group.colophon.clone().map(Into::into),
                            kit,
                        )
                        .current(front && focused && cursor == Some(index))
                        .into_any_element(),
                    )
                }
                Item::Row(..) => {
                    let row = model::row_at(page, &items, index)?;
                    Some(self.render_row(place, index, row, focused, front, cx))
                }
            })
            .collect();
        let count = children.len();
        let ids = if front { FRONT_IDS } else { CLOSING_IDS };
        let feedback = front
            .then(|| self.failure.clone())
            .flatten()
            .map(|failure| {
                div()
                    .px(px(qol_theme::SPACE_PAD))
                    .child(SettingsFeedback::new(failure, true))
            });
        frame
            .children(feedback)
            .child(
                settings_page()
                    .id(ids[0])
                    .track_scroll(place.scroll.handle())
                    .overflow_y_scroll()
                    .children(children),
            )
            .when(front, |frame| {
                frame.child(kit.scroll_cue(
                    ScrollSource::Handle {
                        handle: place.scroll.handle().clone(),
                        children: count,
                    },
                    kit.grounds.pane,
                ))
            })
    }

    fn render_deck(&self, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let kit = kit();
        let width = deck::body_width(self.body_bounds.get());
        let depth = self.stack.len().saturating_sub(1);
        let Some(place) = self.stack.last() else {
            return settings_page().into_any_element();
        };
        let front = self.render_page(place, true, focused, cx);
        let closing = self.closing.as_ref().map(|(place, from)| {
            (
                self.render_page(place, false, false, cx),
                deck::exit(self.step, *from, width),
            )
        });
        if depth == 0 {
            return match closing {
                Some((card, slide)) => deck::shell(deck::reveal(kit, front, card, slide)),
                None => front.into_any_element(),
            };
        }
        let view = cx.weak_entity();
        let on_sliver: deck::SliverClick = Rc::new(move |level, _, cx| {
            let _ = view.update(cx, |view, cx| {
                view.back_to(level, cx);
                cx.notify();
            });
        });
        deck::shell(deck::render(
            kit,
            front,
            deck::DeckFrame {
                depth,
                slide: closing
                    .is_none()
                    .then(|| deck::slide(self.step, self.motion, depth, width))
                    .flatten(),
                closing,
                animation_id: ANIMATION,
                marks: self.stack[..depth].iter().map(|place| place.mark).collect(),
                on_sliver: Some(on_sliver),
            },
        ))
    }
}

fn read_pages(value: serde_json::Value) -> Result<Pages, String> {
    let pages: Pages = serde_json::from_value(value)
        .map_err(|error| format!("the plugin sent pages qol cannot read: {error}"))?;
    match pages.problems().into_iter().next() {
        Some(problem) => Err(format!("the plugin sent broken pages: {problem}")),
        None => Ok(pages),
    }
}

fn tone(tone: Tone) -> SettingsValueTone {
    match tone {
        Tone::Normal => SettingsValueTone::Normal,
        Tone::Muted => SettingsValueTone::Muted,
        Tone::Attention => SettingsValueTone::Attention,
        Tone::Danger => SettingsValueTone::Danger,
        Tone::Success => SettingsValueTone::Success,
    }
}

impl CustomSettingsBreadcrumbs for PagesView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        let Some(pages) = self.pages.as_ref() else {
            return Vec::new();
        };
        self.stack
            .iter()
            .skip(1)
            .filter_map(|place| pages.page(&place.page))
            .filter_map(|page| SettingsDestination::new(page.title.clone()).ok())
            .collect()
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        let mut left = Vec::new();
        if let Some(verb) = self
            .selected_row()
            .and_then(|(_, row)| model::enter_verb(&row))
        {
            left.push(SettingsHint::new(Key::ENTER, verb));
        }
        left.push(SettingsHint::new(Key::UP_DOWN, "move"));
        Some(CustomHints {
            question: None,
            left,
            right: Vec::new(),
        })
    }
}

impl Focusable for PagesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PagesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.subscriptions.is_empty() {
            self.subscriptions
                .push(cx.on_blur(&self.focus, window, |view, _, cx| view.suspend(cx)));
            self.subscriptions
                .push(cx.observe_window_activation(window, |view, window, cx| {
                    if !window.is_window_active() {
                        view.suspend(cx);
                    }
                    cx.notify();
                }));
        }
        let focused = self.focus.is_focused(window);
        if !self.asked {
            self.load(cx);
        }
        if focused && window.is_window_active() {
            self.start_polling(cx);
        }
        self.sync_selection();
        if let Some((place, page)) = self.front() {
            let items = model::items(page);
            place.scroll.follow(
                place.selected.as_ref().map(|_| place.index),
                model::header_of(&items, place.index),
                px(OVERFLOW_FADE_HEIGHT),
            );
        }
        let body = match (&self.pages, &self.failure) {
            (Some(_), _) => self.render_deck(focused, cx),
            (None, Some(failure)) => settings_page()
                .child(settings_message(failure.clone(), true, kit()))
                .into_any_element(),
            (None, None) if self.opened.elapsed() >= qol_theme::WAIT_BEFORE_BUSY => settings_page()
                .child(settings_busy_message(
                    "settings-pages-loading",
                    format!("Reading {}", self.heading),
                    kit(),
                ))
                .into_any_element(),
            (None, None) => settings_page().into_any_element(),
        };
        div()
            .id("settings-pages")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(deck::body_recorder(Rc::clone(&self.body_bounds)))
            .child(body)
    }
}
