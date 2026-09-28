use super::super::data::{request_json, REQUEST_TIMEOUT};
use super::model::{self, Action, Row};
use crate::features::linked_computers::settings::{
    self, CatalogOperation, Failure, InvitationInfo, Snapshot,
};
use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, AsyncApp, ClipboardItem, Context, FocusHandle, Focusable, KeyDownEvent,
    Render, WeakEntity, Window,
};
use qol_gpui::kit::kit;
use qol_gpui::scroll_list::SelectionScroll;
use qol_gpui::scrollbar::{ScrollSource, OVERFLOW_FADE_HEIGHT};
use qol_gpui::settings_panel::components::{
    settings_busy_message, settings_label, settings_label_group, settings_page, RowGround,
    SettingsTextField,
};
use qol_gpui::settings_panel::{
    CustomPanelCallback, CustomSettingsBreadcrumbs, SettingsDestination, SettingsRow,
};
use qol_gpui::surface::SurfaceDismisser;
use qol_gpui::text_edit::{self, TextField};
use qol_peers::admin::{Request, Response};
use qol_peers::enrollment::ExportedInvitation;
use qol_runtime::local_http::Method;
use qol_runtime::PlatformStateClient;

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
    name: String,
    source: Option<(ExportedInvitation, InvitationInfo)>,
    invitation: Option<Response>,
    pending: bool,
    selected: usize,
    sequence: u64,
    message: String,
    editing: Editing,
    field: TextField,
    subscriptions: Vec<gpui::Subscription>,
    refresh_on_focus: bool,
    scroll: SelectionScroll,
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
            name: String::new(),
            source: None,
            invitation: None,
            pending: false,
            selected: 0,
            sequence: 0,
            message: "Refresh reads core. Pairing grants no operations.".into(),
            editing: Editing::None,
            field: TextField::new(),
            subscriptions: Vec::new(),
            refresh_on_focus: true,
            scroll: SelectionScroll::new(),
        }
    }

    fn work(&mut self, request: Option<Request>, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.refresh_on_focus = false;
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
        self.message = "Reading or updating core".into();
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
                            view.apply(response, snapshot);
                        }
                        Err(_) => {
                            view.message =
                                "Result unknown. Refresh core before another change.".into()
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn apply(
        &mut self,
        response: Result<Option<Response>, Failure>,
        snapshot: Result<Snapshot, Failure>,
    ) {
        if let Ok(Some(response @ Response::Invitation { .. })) = response {
            self.invitation = Some(response);
        }
        match snapshot {
            Ok(snapshot) => {
                if self.name.is_empty() {
                    if let Some(authority) = &snapshot.status.authority { self.name = authority.name.clone(); }
                }
                self.snapshot = Some(snapshot);
                self.message = "State read from core. Refresh for current connections and pairing progress.".into();
            }
            Err(error) => self.message = match error {
                Failure::OutcomeUnknown => "Result unknown. Refresh core, then recover the original transaction. Never prepare a replacement.".into(),
                Failure::Authority(error) => format!("Core refused the request: {error}. Refresh before changing anything."),
                Failure::Transport(_) | Failure::Inconsistent => "Current state unavailable. Refresh core; previous connection labels are no longer current.".into(),
            },
        }
    }

    fn rows(&self) -> Vec<Row> {
        model::rows(
            self.snapshot.as_ref(),
            self.catalog.as_deref(),
            &self.name,
            self.source.as_ref(),
            self.invitation.as_ref(),
        )
    }

    fn suspend(&mut self, cx: &mut Context<Self>) {
        self.sequence = self.sequence.wrapping_add(1);
        self.refresh_on_focus = true;
        self.snapshot = None;
        self.catalog = None;
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
                self.field.set_text(self.name.clone());
                self.editing = Editing::Name;
            }
            Action::Paste => self.paste(cx),
            Action::Copy => {
                if let Some(Response::Invitation { document, .. }) = &self.invitation {
                    cx.write_to_clipboard(ClipboardItem::new_string(document.expose().to_owned()));
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
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let document = ExportedInvitation::from_owned(zeroize::Zeroizing::new(text));
        let result = document.ok().and_then(|document| {
            settings::invitation_info(&document)
                .ok()
                .map(|info| (document, info))
        });
        match result {
            Some(source) => {
                self.source = Some(source);
                self.message =
                    "Invitation read. Prepare once, or recover its original outgoing transaction."
                        .into();
            }
            None => self.message = "Invalid invitation code. Nothing was submitted.".into(),
        }
    }

    fn finish_edit(&mut self, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.editing, Editing::None) {
            Editing::Name => self.name = self.field.text().trim().to_owned(),
            Editing::Confirm { request, word } => {
                if self.field.text().trim().eq_ignore_ascii_case(word) {
                    self.work(Some(request), cx);
                    return;
                }
                self.message = format!("Type {word} to confirm, or Escape to cancel.");
                self.editing = Editing::Confirm { request, word };
            }
            Editing::None => {}
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        if matches!(key, "escape" | "esc") {
            if matches!(self.editing, Editing::None) {
                self.suspend(cx);
                (self.on_back)(window, cx);
                return;
            }
            self.editing = Editing::None;
            self.field.clear();
            cx.notify();
            return;
        }
        if !matches!(self.editing, Editing::None) {
            if matches!(key, "enter" | "return") {
                self.finish_edit(cx);
            }
            if !matches!(key, "enter" | "return") {
                let mut next = self.field.clone();
                text_edit::apply_edit_key(&mut next, &event.keystroke, || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if next.text().len() <= 128 {
                    self.field = next;
                }
            }
            cx.notify();
            return;
        }
        let rows = self.rows();
        if key == "tab" && event.keystroke.modifiers.shift {
            self.selected = self.selected.saturating_sub(1);
            cx.notify();
            return;
        }
        match key {
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" | "tab" => self.selected = (self.selected + 1).min(rows.len().saturating_sub(1)),
            "enter" | "return" | "space" => {
                if let Some(action) = rows.get(self.selected).and_then(|row| row.action.clone()) {
                    self.activate(action, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn render_row(
        &self,
        index: usize,
        row: Row,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ground = RowGround::of(self.selected == index, focused);
        SettingsRow::setting(("linked-computer-row", index), kit())
            .selected(self.selected == index, focused)
            .on_click(cx.listener(move |view, _, _, cx| {
                if !matches!(view.editing, Editing::None) {
                    return;
                }
                view.selected = index;
                if let Some(action) = row.action.clone() {
                    view.activate(action, cx);
                }
                cx.notify();
            }))
            .child(settings_label_group(
                row.label,
                Some(row.detail.into()),
                ground,
                kit(),
            ))
            .into_any_element()
    }
}

impl CustomSettingsBreadcrumbs for LinkedComputersView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        Vec::new()
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
        if focused && window.is_window_active() && self.refresh_on_focus && !self.pending {
            self.work(None, cx);
        }
        let rows = self.rows();
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let children = rows.len() + 1 + usize::from(!matches!(self.editing, Editing::None)) * 2;
        let selected_child = if matches!(self.editing, Editing::None) {
            self.selected + 1
        } else {
            2
        };
        self.scroll
            .follow(Some(selected_child), None, gpui::px(OVERFLOW_FADE_HEIGHT));
        let busy = self.pending && focused && window.is_window_active() && !self.refresh_on_focus;
        let message = if busy {
            settings_busy_message("linked-computers-loading", self.message.clone(), kit())
        } else {
            settings_label(self.message.clone(), kit())
        };
        let mut page = settings_page().child(message);
        if !matches!(self.editing, Editing::None) {
            let label = match &self.editing {
                Editing::Name => "Computer name".to_string(),
                Editing::Confirm { word, .. } => {
                    format!("Type {word}, then Enter. Escape cancels.")
                }
                Editing::None => String::new(),
            };
            page = page
                .child(settings_label(label, kit()))
                .child(SettingsTextField::live(
                    self.field.clone(),
                    RowGround::of(true, focused),
                    kit(),
                ));
        }
        page = page.children(
            rows.into_iter()
                .enumerate()
                .map(|(index, row)| self.render_row(index, row, focused, cx)),
        );
        div()
            .id("linked-computers-body")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::on_key))
            .child(
                page.id("linked-computers-scroll")
                    .overflow_y_scroll()
                    .track_scroll(self.scroll.handle()),
            )
            .child(kit().scroll_cue(
                ScrollSource::Handle {
                    handle: self.scroll.handle().clone(),
                    children,
                },
                kit().grounds.pane,
            ))
    }
}
