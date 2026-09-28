use super::super::data::{request_json, REQUEST_TIMEOUT};
use super::model::{self, Action, Row, NAME_RULE};
use crate::features::linked_computers::settings::{
    self, CatalogOperation, Failure, InvitationInfo, Snapshot,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, AsyncApp, ClipboardItem, Context, FocusHandle, Focusable, KeyDownEvent,
    Render, ScrollWheelEvent, WeakEntity, Window,
};
use qol_gpui::key::Key;
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::settings_panel::components::{
    settings_label_group, settings_page, settings_value_group, RowGround, SettingsFeedback,
    SettingsHint, SettingsTextField,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_affordance, settings_busy_message,
    settings_list, settings_value_text, CustomHints, CustomPanelCallback,
    CustomSettingsBreadcrumbs, EscapeStep, Intent, SettingsDestination, SettingsGroupHeader,
    SettingsRow,
};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::text_edit::{self, TextField};
use qol_peers::admin::{Request, Response};
use qol_peers::enrollment::ExportedInvitation;
use qol_runtime::local_http::Method;
use qol_runtime::PlatformStateClient;

const MAX_VISIBLE: usize = 9;
const DANGER_VERBS: [&str; 5] = ["stop", "revoke", "remove", "abandon", "decline"];
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

enum Editing {
    None,
    Name,
    Confirm {
        request: Request,
        word: &'static str,
    },
}

pub(super) struct LinkedComputersView {
    focus: FocusHandle,
    on_back: CustomPanelCallback,
    snapshot: Option<Snapshot>,
    catalog: Option<Vec<CatalogOperation>>,
    withheld: Vec<crate::plugins::PluginId>,
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

impl LinkedComputersView {
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
        self.snapshot = None;
        self.catalog = None;
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

    fn rows(&self) -> Vec<Row> {
        model::rows(
            self.snapshot.as_ref(),
            self.catalog.as_deref(),
            self.source.as_ref(),
            self.invitation.as_ref(),
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
            Action::Paste => self.paste(cx),
            Action::Copy => {
                if let Some(Response::Invitation { document, .. }) = &self.invitation {
                    cx.write_to_clipboard(ClipboardItem::new_string(document.expose().to_owned()));
                    self.notice = Some((
                        "Invitation copied. Paste it on the other computer.".into(),
                        false,
                    ));
                }
            }
            Action::Send(request, Some(word)) => {
                self.field.clear();
                self.editing = Editing::Confirm { request, word };
            }
            Action::Send(request, None) => self.work(Some(request), cx),
        }
        cx.notify();
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        self.source = None;
        let Some(item) = cx.read_from_clipboard() else {
            self.notice = Some((
                "The clipboard is empty. Copy the invitation on the other computer first.".into(),
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
                "The clipboard is empty. Copy the invitation on the other computer first.".into(),
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
                "The clipboard text is not a linked computers invitation. Nothing was sent.".into(),
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
            Editing::Confirm { request, word } => {
                if self.field.text().trim().eq_ignore_ascii_case(word) {
                    self.work(Some(request), cx);
                    return;
                }
                self.notice = Some((
                    format!("Type {word} to confirm, or Escape to cancel."),
                    true,
                ));
                self.editing = Editing::Confirm { request, word };
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
        match escape_step(0, false, true) {
            EscapeStep::CloseFilter | EscapeStep::PopCard => {}
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
        index: usize,
        row: &Row,
        current_group: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
        let mut values = settings_value_group();
        if editing_name {
            values = values.child(SettingsTextField::live(self.field.clone(), ground, kit));
        } else {
            values = values.children(
                row.value
                    .clone()
                    .map(|(text, tone)| settings_value_text(text, tone, ground, kit)),
            );
            values = values.children(row.verb.map(|verb| {
                settings_action_affordance(
                    ("linked-computers-action", index),
                    verb,
                    DANGER_VERBS.contains(&verb).then_some("danger"),
                    selected && self.pending,
                    ground,
                    kit,
                )
            }));
        }
        SettingsRow::setting(("linked-computers-row", index), kit)
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
            .into_any_element()
    }

    fn render_confirm(&self, word: &str, focused: bool) -> AnyElement {
        let kit = kit();
        let ground = RowGround::of(true, focused);
        SettingsRow::setting("linked-computers-confirm", kit)
            .selected(true, focused)
            .child(settings_label_group(
                format!("Type {word} to confirm"),
                Some("Enter confirms. Escape cancels.".into()),
                ground,
                kit,
            ))
            .child(settings_value_group().child(SettingsTextField::live(
                self.field.clone(),
                ground,
                kit,
            )))
            .into_any_element()
    }

    fn render_list(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let total = rows.len();
        let cursor_group = rows
            .get(..=self.selected.min(total.saturating_sub(1)))
            .and_then(|seen| seen.iter().rposition(|row| row.header));
        let range = self.list.visible_range(total);
        let mut list = settings_list()
            .id("linked-computers-list")
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
}

impl CustomSettingsBreadcrumbs for LinkedComputersView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        Vec::new()
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        if !matches!(self.editing, Editing::None) {
            let commit = match self.editing {
                Editing::Name => "save",
                _ => "confirm",
            };
            return Some(CustomHints {
                question: None,
                left: vec![
                    SettingsHint::new(Key::ENTER, commit),
                    SettingsHint::new(Key::TYPE, "edit"),
                ],
                right: vec![SettingsHint::new(Key::ESC, "cancel")],
            });
        }
        let mut left = Vec::new();
        if let Some(verb) = self.rows().get(self.selected).and_then(|row| row.verb) {
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

impl Focusable for LinkedComputersView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LinkedComputersView {
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
        let rows = self.rows();
        self.sync_selection(&rows);
        let busy = self.pending && focused && window.is_window_active() && !self.refresh_on_focus;
        let mut page = settings_page();
        if let Some((message, danger)) = &self.notice {
            page = page.child(SettingsFeedback::new(message.clone(), *danger));
        }
        if let Editing::Confirm { word, .. } = &self.editing {
            page = page.child(self.render_confirm(word, focused));
        }
        page = page.child(self.render_list(&rows, focused, cx));
        if busy && self.snapshot.is_none() {
            page = page.child(settings_busy_message(
                "linked-computers-loading",
                "Reading linking state",
                kit(),
            ));
        }
        page.id("linked-computers-body")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
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
            .map(|request| settings::request(&client, request))
            .transpose();
        let snapshot = match &response {
            Ok(_) => settings::load(&client),
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
