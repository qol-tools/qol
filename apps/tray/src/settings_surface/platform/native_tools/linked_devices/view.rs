use super::super::data::{request_json, REQUEST_TIMEOUT};
use super::model::{self, Action, Control, Row, NAME_RULE};
use super::snapshot::{self, Snapshot};
use crate::features::linked_devices::settings::{self, CatalogOperation, Failure, InvitationInfo};
use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, AsyncApp, ClipboardItem, Context, Div, FocusHandle, Focusable,
    KeyDownEvent, Render, ScrollWheelEvent, WeakEntity, Window,
};
use qol_gpui::deck;
use qol_gpui::key::Key;
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::settings_panel::components::{
    settings_label_group, settings_page, settings_value_group, RowGround, SettingsFeedback,
    SettingsHint, SettingsToggle,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_affordance, settings_action_spinner,
    settings_busy_message, settings_list, settings_value_text, CustomHints, CustomPanelCallback,
    CustomSettingsBreadcrumbs, EscapeStep, Intent, SettingsDestination, SettingsGroupHeader,
    SettingsRow, SettingsValueTone,
};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::text_edit::{self, TextField};
use qol_peers::admin::{Request, Response};
use qol_peers::enrollment::ExportedInvitation;
use qol_peers::PeerId;
use qol_runtime::local_http::Method;
use qol_runtime::PlatformStateClient;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const MAX_VISIBLE: usize = 9;
const DANGER_VERBS: [&str; 5] = ["unlink", "remove", "abandon", "decline", "reject"];
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
const CARD_DEPTH: usize = 1;
const CARD_ANIMATION: &str = "linked-devices-card-slide";
const PAGE_IDS: [&str; 4] = [
    "linked-devices-list",
    "linked-devices-row",
    "linked-devices-spinner",
    "linked-devices-action",
];
const CARD_IDS: [&str; 4] = [
    "linked-devices-card-list",
    "linked-devices-card-row",
    "linked-devices-card-spinner",
    "linked-devices-card-action",
];

enum Editing {
    None,
    Name,
}

pub(super) struct LinkedDevicesView {
    focus: FocusHandle,
    on_back: CustomPanelCallback,
    snapshot: Option<Snapshot>,
    catalog: Option<Vec<CatalogOperation>>,
    withheld: Vec<crate::plugins::PluginId>,
    open: Option<(PeerId, usize)>,
    closing: Option<PeerId>,
    step: usize,
    motion: Option<deck::Motion>,
    mark: Option<f32>,
    body_bounds: deck::BodyBounds,
    row_bounds: deck::RowBounds,
    stopped: bool,
    enabling: bool,
    source: Option<(ExportedInvitation, InvitationInfo)>,
    invitation: Option<Response>,
    pending: bool,
    polling: bool,
    poller: bool,
    selected: usize,
    list: ScrollList,
    sequence: u64,
    notice: Option<(String, bool)>,
    editing: Editing,
    field: TextField,
    subscriptions: Vec<gpui::Subscription>,
    refresh_on_focus: bool,
}

impl LinkedDevicesView {
    pub(super) fn new(
        on_back: CustomPanelCallback,
        _dismisser: SurfaceDismisser,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            on_back,
            snapshot: None,
            catalog: None,
            withheld: Vec::new(),
            open: None,
            closing: None,
            step: 0,
            motion: None,
            mark: None,
            body_bounds: Rc::new(Cell::new(None)),
            row_bounds: Rc::new(RefCell::new(HashMap::new())),
            stopped: false,
            enabling: false,
            source: None,
            invitation: None,
            pending: false,
            polling: false,
            poller: false,
            selected: 1,
            list: ScrollList::new(MAX_VISIBLE),
            sequence: 0,
            notice: None,
            editing: Editing::None,
            field: TextField::new(),
            subscriptions: Vec::new(),
            refresh_on_focus: true,
        }
    }

    fn work(&mut self, request: Option<Request>, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.refresh_on_focus = false;
        if matches!(&request, Some(Request::Stop { .. })) {
            self.stopped = true;
        }
        if matches!(
            &request,
            Some(Request::Enrollment {
                request: qol_peers::admin::EnrollmentRequest::CancelInvitation { .. },
            })
        ) {
            self.invitation = None;
        }
        self.pending = true;
        self.sequence = self.sequence.wrapping_add(1);
        let sequence = self.sequence;
        self.notice = None;
        let receiver = load(request);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = receiver.await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.pending = false;
                    if view.sequence != sequence {
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok((response, snapshot, catalog)) => {
                            view.catalog = catalog;
                            if let Some(next) = view.apply(response, snapshot) {
                                view.work(Some(next), cx);
                            }
                        }
                        Err(_) => {
                            view.notice = Some((
                                "Result unknown. Refresh before another change.".into(),
                                true,
                            ))
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        self.polling = true;
        let sequence = self.sequence;
        let receiver = load(None);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = receiver.await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.polling = false;
                    if view.sequence != sequence || view.pending {
                        return;
                    }
                    if let Ok((_, Ok(snapshot), catalog)) = result {
                        view.catalog = catalog.or(view.catalog.take());
                        if let Some(next) = view.apply(Ok(None), Ok(snapshot)) {
                            view.work(Some(next), cx);
                        }
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        if self.poller {
            return;
        }
        self.poller = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor().timer(POLL_INTERVAL).await;
                    let alive = this.update(&mut cx, |view, cx| {
                        if !view.pending && !view.polling && matches!(view.editing, Editing::None) {
                            view.poll(cx);
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn apply(
        &mut self,
        response: Result<Option<Response>, Failure>,
        snapshot: Result<Snapshot, Failure>,
    ) -> Option<Request> {
        let mut next = None;
        if let Ok(Some(response @ Response::Invitation { .. })) = &response {
            self.invitation = Some(response.clone());
        }
        if let (Ok(Some(Response::JoinPrepared { transaction, .. })), Some((document, _))) =
            (&response, &self.source)
        {
            if let Ok(current) = &snapshot {
                if let Some(authority) = &current.status.authority {
                    next = Some(Request::Enrollment {
                        request: qol_peers::admin::EnrollmentRequest::Redeem {
                            expected: authority.expected(),
                            transaction: *transaction,
                            document: document.clone(),
                        },
                    });
                }
            }
        }
        match snapshot {
            Ok(snapshot) => {
                if snapshot.status.lifecycle == qol_peers::admin::Lifecycle::Inactive
                    && !self.stopped
                    && !self.enabling
                {
                    self.enabling = true;
                    next = Some(Request::Enable);
                }
                self.snapshot = Some(snapshot);
            }
            Err(error) => {
                self.notice = Some((
                    match error {
                        Failure::OutcomeUnknown => "Result unknown. Refresh, then recover the original transaction. Never prepare a replacement.".into(),
                        Failure::Authority(qol_peers::admin::Error::Authority { error: qol_peers::AuthorityError::InvalidName }) => NAME_RULE.into(),
                        Failure::Authority(error) => format!("Linking refused the change: {error}. Refresh before changing anything."),
                        Failure::Transport(_) | Failure::Inconsistent => "Linking state is unavailable. Refresh to read it again.".into(),
                    },
                    true,
                ))
            }
        }
        next
    }

    fn card(&self) -> Option<(String, Vec<Row>)> {
        let (peer_id, _) = self.open?;
        self.card_for(peer_id)
    }

    fn card_for(&self, peer_id: PeerId) -> Option<(String, Vec<Row>)> {
        model::card(
            self.snapshot.as_ref(),
            self.catalog.as_deref(),
            &self.withheld,
            peer_id,
        )
    }

    fn open_card(&mut self, peer_id: PeerId) {
        self.mark = deck::row_mark(&self.row_bounds, self.selected, self.body_bounds.get());
        self.closing = None;
        self.open = Some((peer_id, self.selected));
        self.selected = 0;
        self.list.reset();
        self.step = self.step.wrapping_add(1);
        self.motion = Some(deck::Motion::Push);
    }

    fn close_card(&mut self) {
        if let Some((_, selected)) = self.open.take() {
            self.selected = selected;
            self.list.selected = selected;
        }
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        let Some((peer_id, _)) = self.open else {
            return;
        };
        self.closing = Some(peer_id);
        self.close_card();
        self.step = self.step.wrapping_add(1);
        self.motion = None;
        deck::after_transition(cx, |view, cx| {
            view.closing = None;
            cx.notify();
        });
    }

    fn sliver_click(cx: &mut Context<Self>) -> deck::SliverClick {
        let view = cx.weak_entity();
        Rc::new(move |_, _, cx| {
            let _ = view.update(cx, |view, cx| {
                view.leave(cx);
                cx.notify();
            });
        })
    }

    fn rows(&self) -> Vec<Row> {
        if let Some((_, rows)) = self.card() {
            return rows;
        }
        model::rows(
            self.snapshot.as_ref(),
            self.source.as_ref(),
            self.invitation.as_ref(),
            self.catalog.as_deref(),
            &self.withheld,
        )
    }

    fn suspend(&mut self, cx: &mut Context<Self>) {
        self.sequence = self.sequence.wrapping_add(1);
        self.refresh_on_focus = true;
        self.source = None;
        self.invitation = None;
        self.editing = Editing::None;
        self.field.clear();
        cx.notify();
    }

    fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        match action {
            Action::Refresh => self.work(None, cx),
            Action::Name => {
                let name = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.status.authority.as_ref())
                    .map(|authority| authority.name.clone())
                    .unwrap_or_default();
                self.field.set_text(name);
                self.editing = Editing::Name;
            }
            Action::Toggle(plugin) => {
                match self
                    .withheld
                    .iter()
                    .position(|withheld| *withheld == plugin)
                {
                    Some(index) => {
                        self.withheld.remove(index);
                    }
                    None => self.withheld.push(plugin),
                }
            }
            Action::Open(peer_id) => self.open_card(peer_id),
            Action::Paste => self.paste(cx),
            Action::Copy => {
                if let Some(Response::Invitation { document, .. }) = &self.invitation {
                    cx.write_to_clipboard(ClipboardItem::new_string(document.expose().to_owned()));
                    self.notice = Some((
                        "Invitation copied. Paste it on the other device.".into(),
                        false,
                    ));
                }
            }
            Action::Send(request) => {
                if matches!(request, Request::Nearby { .. } | Request::Revoke { .. }) {
                    self.leave(cx);
                }
                self.work(Some(request), cx)
            }
        }
        cx.notify();
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        self.source = None;
        let Some(item) = cx.read_from_clipboard() else {
            self.notice = Some((
                "The clipboard is empty. Copy the invitation on the other device first.".into(),
                true,
            ));
            return;
        };
        let Some(text) = item.text() else {
            self.notice = Some((
                "The clipboard holds something other than text, so there is no invitation to read."
                    .into(),
                true,
            ));
            return;
        };
        if text.trim().is_empty() {
            self.notice = Some((
                "The clipboard is empty. Copy the invitation on the other device first.".into(),
                true,
            ));
            return;
        }
        let document = ExportedInvitation::from_owned(zeroize::Zeroizing::new(text));
        let result = document.ok().and_then(|document| {
            settings::invitation_info(&document)
                .ok()
                .map(|info| (document, info))
        });
        let Some(source) = result else {
            self.notice = Some((
                "The clipboard text is not a linked devices invitation. Nothing was sent.".into(),
                true,
            ));
            return;
        };
        let expected = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.status.authority.as_ref())
            .map(|authority| authority.expected());
        let document = source.0.clone();
        self.source = Some(source);
        if let Some(expected) = expected {
            self.work(
                Some(Request::Enrollment {
                    request: qol_peers::admin::EnrollmentRequest::Prepare { expected, document },
                }),
                cx,
            );
        }
    }

    fn finish_edit(&mut self, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.editing, Editing::None) {
            Editing::Name => {
                let name = self.field.text().trim().to_owned();
                if !qol_peers::is_valid_name(&name) {
                    self.notice = Some((NAME_RULE.into(), true));
                    return;
                }
                let expected = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.status.authority.as_ref())
                    .map(|authority| authority.expected());
                if let Some(expected) = expected {
                    self.work(Some(Request::Rename { expected, name }), cx);
                }
            }
            Editing::None => {}
        }
    }

    fn navigable(rows: &[Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| !row.header)
            .map(|(index, _)| index)
            .collect()
    }

    fn move_selection(&mut self, direction: isize) {
        let rows = self.rows();
        let navigable = Self::navigable(&rows);
        if navigable.is_empty() {
            return;
        }
        self.selected = adjacent_visible_row(&navigable, self.selected, direction);
        self.list.selected = self.selected;
        self.list.sync(rows.len());
    }

    fn sync_selection(&mut self, rows: &[Row]) {
        let navigable = Self::navigable(rows);
        self.selected = navigable
            .iter()
            .copied()
            .find(|index| *index >= self.selected)
            .or_else(|| navigable.last().copied())
            .unwrap_or(0);
        self.list.selected = self.selected;
        self.list.sync(rows.len());
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match escape_step(usize::from(self.open.is_some()), false, true) {
            EscapeStep::CloseFilter => {}
            EscapeStep::PopCard => {
                self.leave(cx);
                cx.notify();
            }
            EscapeStep::AscendRail | EscapeStep::Dismiss => {
                self.suspend(cx);
                (self.on_back)(window, cx);
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        let editing = !matches!(self.editing, Editing::None);
        if !editing && matches!(key, "backspace" | "delete") {
            if let Some((_, action)) = self
                .rows()
                .get(self.selected)
                .and_then(|row| row.remove.clone())
            {
                self.activate(action, cx);
                cx.notify();
            }
            return;
        }
        match intent(key, event.keystroke.key_char.as_deref(), editing) {
            Some(Intent::CancelEdit) => {
                self.editing = Editing::None;
                self.field.clear();
            }
            Some(Intent::CommitEdit) => self.finish_edit(cx),
            _ if editing => {
                let mut next = self.field.clone();
                text_edit::apply_edit_key(&mut next, &event.keystroke, || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if next.text().len() <= qol_peers::MAX_NAME_BYTES {
                    self.field = next;
                }
            }
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
            Some(Intent::Activate) => {
                if let Some(action) = self
                    .rows()
                    .get(self.selected)
                    .and_then(|row| row.action.clone())
                {
                    self.activate(action, cx);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn render_row(
        &self,
        card: bool,
        index: usize,
        row: &Row,
        current_group: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let [_, row_id, spinner_id, action_id] = ids(card);
        let kit = kit();
        if row.header {
            return SettingsGroupHeader::new(
                row.label.clone(),
                Some(row.detail.clone().into()),
                kit,
            )
            .current(current_group && focused)
            .into_any_element();
        }
        let selected = self.selected == index;
        let ground = RowGround::of(selected, focused);
        let editing_name = selected && matches!(self.editing, Editing::Name);
        let action = row.action.clone();
        let busy = selected && self.pending;
        let mut values = settings_value_group();
        if busy && row.control != Control::Chip {
            values = values.child(settings_action_spinner((spinner_id, index), kit));
        }
        values = match &row.control {
            _ if editing_name => values.child(settings_value_text(
                format!("{}_", self.field.text()),
                SettingsValueTone::Normal,
                ground,
                kit,
            )),
            Control::Value(text, tone) => {
                values.child(settings_value_text(text.clone(), *tone, ground, kit))
            }
            Control::Toggle(on) => values.child(SettingsToggle::new(*on, ground, kit)),
            Control::Chip => values.children(row.verb.map(|verb| {
                settings_action_affordance(
                    (action_id, index),
                    verb,
                    DANGER_VERBS.contains(&verb).then_some("danger"),
                    busy,
                    ground,
                    kit,
                )
            })),
            Control::None => values,
        };
        SettingsRow::setting((row_id, index), kit)
            .selected(selected, focused)
            .on_click(cx.listener(move |view, _, _, cx| {
                if !matches!(view.editing, Editing::None) {
                    return;
                }
                view.selected = index;
                if let Some(action) = action.clone() {
                    view.activate(action, cx);
                }
                cx.notify();
            }))
            .child(settings_label_group(
                row.label.clone(),
                Some(row.detail.clone().into()),
                ground,
                kit,
            ))
            .child(values)
            .children((!card).then(|| deck::bounds_recorder(Rc::clone(&self.row_bounds), index)))
            .into_any_element()
    }

    fn render_list(
        &self,
        card: bool,
        rows: &[Row],
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total = rows.len();
        let cursor_group = rows
            .get(..=self.selected.min(total.saturating_sub(1)))
            .and_then(|seen| seen.iter().rposition(|row| row.header));
        let range = self.list.visible_range(total);
        let mut list = settings_list()
            .id(ids(card)[0])
            .on_scroll_wheel(
                cx.listener(|view: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    let steps = wheel_rows(&event.delta, qol_theme::HEIGHT_SETTING_ROW);
                    for _ in 0..steps.unsigned_abs() {
                        view.move_selection(steps.signum());
                    }
                    cx.notify();
                }),
            );
        for index in range.clone() {
            list = list.child(self.render_row(
                card,
                index,
                &rows[index],
                cursor_group == Some(index),
                focused,
                cx,
            ));
        }
        list.child(kit().scroll_cue(
            ScrollSource::Window {
                first: range.start,
                shown: range.len(),
                total,
            },
            kit().grounds.pane,
        ))
        .into_any_element()
    }

    fn render_page(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> Div {
        let mut page = settings_page();
        if let Some((message, danger)) = &self.notice {
            page = page.child(SettingsFeedback::new(message.clone(), *danger));
        }
        page.child(self.render_list(self.open.is_some(), rows, focused, cx))
    }

    fn render_deck(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let kit = kit();
        let width = deck::body_width(self.body_bounds.get());
        let front = self.render_page(rows, focused, cx);
        if self.open.is_some() {
            return deck::shell(deck::render(
                kit,
                front,
                deck::DeckFrame {
                    depth: CARD_DEPTH,
                    slide: deck::slide(self.step, self.motion, CARD_DEPTH, width),
                    closing: None,
                    animation_id: CARD_ANIMATION,
                    marks: vec![self.mark],
                    on_sliver: Some(Self::sliver_click(cx)),
                },
            ));
        }
        let Some((_, leaving)) = self.closing.and_then(|peer_id| self.card_for(peer_id)) else {
            return front.into_any_element();
        };
        let card = settings_page().child(self.render_list(true, &leaving, false, cx));
        deck::shell(deck::reveal(
            kit,
            front,
            card,
            deck::exit(self.step, CARD_DEPTH, width),
        ))
    }
}

impl CustomSettingsBreadcrumbs for LinkedDevicesView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        self.card()
            .and_then(|(title, _)| SettingsDestination::new(title).ok())
            .into_iter()
            .collect()
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        if !matches!(self.editing, Editing::None) {
            return Some(CustomHints {
                question: None,
                left: vec![
                    SettingsHint::new(Key::ENTER, "save"),
                    SettingsHint::new(Key::TYPE, "edit"),
                ],
                right: vec![SettingsHint::new(Key::ESC, "cancel")],
            });
        }
        let mut left = Vec::new();
        if let Some(verb) = self.rows().get(self.selected).and_then(|row| row.verb) {
            left.push(SettingsHint::new(Key::ENTER, verb));
        }
        if let Some((verb, _)) = self
            .rows()
            .get(self.selected)
            .and_then(|row| row.remove.as_ref())
        {
            left.push(SettingsHint::new(Key::BACKSPACE, *verb));
        }
        left.push(SettingsHint::new(Key::UP_DOWN, "move"));
        Some(CustomHints {
            question: None,
            left,
            right: Vec::new(),
        })
    }
}

impl Focusable for LinkedDevicesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LinkedDevicesView {
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
        if self.sequence == 0 && !self.pending {
            self.work(None, cx);
        } else if focused && window.is_window_active() && self.refresh_on_focus && !self.pending {
            self.refresh_on_focus = false;
            self.poll(cx);
        }
        self.start_polling(cx);
        if self.open.is_some() && self.card().is_none() {
            self.close_card();
        }
        let rows = self.rows();
        self.sync_selection(&rows);
        let loading = self.snapshot.is_none() && (self.pending || self.polling);
        let body = if loading {
            let mut page = settings_page();
            if let Some((message, danger)) = &self.notice {
                page = page.child(SettingsFeedback::new(message.clone(), *danger));
            }
            page.child(settings_busy_message(
                "linked-devices-loading",
                "Reading linking state",
                kit(),
            ))
            .into_any_element()
        } else {
            self.render_deck(&rows, focused, cx)
        };
        div()
            .id("linked-devices-body")
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

fn ids(card: bool) -> [&'static str; 4] {
    if card {
        CARD_IDS
    } else {
        PAGE_IDS
    }
}

type Loaded = (
    Result<Option<Response>, Failure>,
    Result<Snapshot, Failure>,
    Option<Vec<CatalogOperation>>,
);

fn load(request: Option<Request>) -> tokio::sync::oneshot::Receiver<Loaded> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let client = PlatformStateClient::from_env();
        let response = request
            .map(|request| snapshot::request(&client, request))
            .transpose();
        let snapshot = match &response {
            Ok(_) => snapshot::load(&client),
            Err(error) => Err(error.clone()),
        };
        let catalog = request_json::<Vec<CatalogOperation>>(
            Method::Get,
            "/api/peers/catalog",
            None,
            REQUEST_TIMEOUT,
        );
        let _ = sender.send((response, snapshot, catalog.ok()));
    });
    receiver
}
