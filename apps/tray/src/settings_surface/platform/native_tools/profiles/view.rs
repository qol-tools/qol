use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, App, AsyncApp, ClipboardItem, Context, Div, FocusHandle, Focusable,
    KeyDownEvent, PathPromptOptions, Render, ScrollWheelEvent, WeakEntity, Window,
};
use qol_gpui::key::Key;
use qol_gpui::kit::{kit, Kit};
use qol_gpui::pictures::PictureContext;
use qol_gpui::scroll_list::{wheel_rows, ScrollList};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::settings_panel::components::{
    settings_label, settings_label_group, settings_page, settings_value_group, ChoiceArt,
    RowGround, SettingsChevron, SettingsChoiceValue, SettingsHint, SettingsTextField,
    SettingsToggle,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_affordance, settings_action_spinner,
    settings_busy_message, settings_description, settings_list, settings_value_text, CustomHints,
    CustomPanelCallback, CustomPanelNoticeTone, CustomPanelNotifier, CustomSettingsBreadcrumbs,
    EscapeStep, Intent, SettingsDestination, SettingsGroupHeader, SettingsRow, SettingsValueTone,
};
use qol_gpui::text_edit::{self, TextField};

use super::data::{self, Backup, SignIn, SignInState, Snapshot};
use super::model::{self, Action, Busy, Dot, Level, Page, Row, Value};

const MAX_VISIBLE: usize = 10;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_NAME_BYTES: usize = 64;

pub(super) struct ProfilesView {
    focus: FocusHandle,
    on_back: CustomPanelCallback,
    notify: CustomPanelNotifier,
    snapshot: Option<Snapshot>,
    backups: Option<Vec<Backup>>,
    level: Level,
    return_to: usize,
    sign_in: Option<SignIn>,
    busy: Option<Busy>,
    pending: bool,
    note: Option<String>,
    naming: bool,
    field: TextField,
    selected: usize,
    list: ScrollList,
    polling: bool,
    poller: bool,
}

impl ProfilesView {
    pub(super) fn new(
        on_back: CustomPanelCallback,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            on_back,
            notify,
            snapshot: None,
            backups: None,
            level: Level::Main,
            return_to: 1,
            sign_in: None,
            busy: None,
            pending: false,
            note: None,
            naming: false,
            field: TextField::new(),
            selected: 1,
            list: ScrollList::new(MAX_VISIBLE),
            polling: false,
            poller: false,
        }
    }

    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let page = Page {
            snapshot,
            sign_in: self.sign_in.as_ref(),
            busy: self.busy,
            note: self.note.as_deref(),
            naming: self.naming,
            now_secs: chrono::Utc::now().timestamp(),
        };
        match self.level {
            Level::Main => model::main_rows(&page),
            Level::Profiles => model::profile_rows(&page),
            Level::Backups => model::backup_rows(self.backups.as_deref()),
        }
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
                    let alive = this.update(&mut cx, |view, cx| {
                        if !view.pending && !view.polling && !view.naming {
                            view.reload(cx);
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                    cx.background_executor().timer(POLL_INTERVAL).await;
                }
            }
        })
        .detach();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.polling = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx.background_spawn(async { data::load() }).await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.polling = false;
                    if let Ok(snapshot) = result {
                        view.snapshot = Some(snapshot);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn run<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx.background_spawn(async move { work() }).await;
                let snapshot = cx.background_spawn(async { data::load() }).await;
                let _ = this.update(&mut cx, |view, cx| {
                    view.pending = false;
                    view.busy = None;
                    if let Ok(snapshot) = snapshot {
                        view.snapshot = Some(snapshot);
                    }
                    match result {
                        Ok(value) => done(view, value, cx),
                        Err(error) => {
                            view.tell(CustomPanelNoticeTone::Failure, format!("{error:#}"), cx)
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn tell(&self, tone: CustomPanelNoticeTone, message: String, cx: &mut Context<Self>) {
        (self.notify)(tone, message, cx);
    }

    fn active_name(&self) -> Option<String> {
        self.snapshot
            .as_ref()?
            .profiles
            .iter()
            .find(|profile| profile.active)
            .map(|profile| profile.name.clone())
    }

    fn enter(&mut self, level: Level, selected: usize) {
        if self.level == Level::Main {
            self.return_to = self.selected;
        }
        self.level = level;
        self.selected = selected;
        self.list.reset();
    }

    fn leave(&mut self) {
        self.level = Level::Main;
        self.naming = false;
        self.field.clear();
        self.selected = self.return_to;
        self.list.reset();
    }

    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        match action {
            Action::OpenProfiles => {
                let active = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot.profiles.iter().position(|profile| profile.active)
                    })
                    .unwrap_or(0);
                self.enter(Level::Profiles, active + 1);
            }
            Action::Use(name) => {
                let was = self.active_name().unwrap_or_default();
                let target = name.clone();
                self.run(
                    move || data::use_profile(&target),
                    move |view, (), _| {
                        view.note = Some(format!("Switched from {was} just now"));
                        view.leave();
                        view.selected = 1;
                    },
                    cx,
                );
            }
            Action::NewProfile => self.new_profile(cx),
            Action::Sync => {
                self.busy = Some(Busy::Sync);
                self.run(data::sync_now, |_, (), _| {}, cx);
            }
            Action::Connect => self.connect(cx),
            Action::OpenGitHub => {
                if let Some(sign_in) = &self.sign_in {
                    cx.write_to_clipboard(ClipboardItem::new_string(sign_in.user_code.clone()));
                    let uri = sign_in.verification_uri.clone();
                    cx.background_spawn(async move { crate::paths::open_url(&uri) })
                        .detach();
                }
            }
            Action::Auto(on) => self.run(move || data::set_auto_sync(on), |_, (), _| {}, cx),
            Action::Disconnect => self.run(
                data::disconnect,
                |view, (), cx| {
                    view.tell(
                        CustomPanelNoticeTone::Success,
                        "This computer stopped syncing. Your settings stay here and on GitHub."
                            .to_string(),
                        cx,
                    )
                },
                cx,
            ),
            Action::OpenBackups => {
                self.backups = None;
                self.enter(Level::Backups, 1);
                self.run(
                    data::backups,
                    |view, backups, _| view.backups = Some(backups),
                    cx,
                );
            }
            Action::OpenBackup(file_name) => {
                self.run(move || data::open_backup(&file_name), |_, (), _| {}, cx)
            }
            Action::Export => self.export(window, cx),
            Action::Import => self.import(cx),
        }
        cx.notify();
    }

    fn new_profile(&mut self, cx: &mut Context<Self>) {
        if !self.naming {
            self.naming = true;
            self.field.clear();
            return;
        }
        let name = self.field.text().trim().to_owned();
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if let Err(message) = model::valid_name(&name, &snapshot.profiles) {
            self.tell(CustomPanelNoticeTone::Failure, message, cx);
            return;
        }
        let from = self.active_name().unwrap_or_else(|| "default".to_string());
        let created = name.clone();
        self.run(
            move || data::create_profile(&name, &from),
            move |view, (), cx| {
                view.naming = false;
                view.field.clear();
                if let Some(index) = view.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .profiles
                        .iter()
                        .position(|profile| profile.name == created)
                }) {
                    view.selected = index + 1;
                }
                view.tell(
                    CustomPanelNoticeTone::Success,
                    format!("Made {created}. Press Enter on it to use it on this computer."),
                    cx,
                );
            },
            cx,
        );
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let has_token = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.sync.has_github_token);
        if has_token {
            self.busy = Some(Busy::Connect);
            self.run(data::connect, |_, (), _| {}, cx);
            return;
        }
        self.run(
            data::start_sign_in,
            |view, sign_in, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(sign_in.user_code.clone()));
                let session = sign_in.session_id.clone();
                let interval = Duration::from_secs(sign_in.interval.max(1));
                view.sign_in = Some(sign_in);
                view.watch_sign_in(session, interval, cx);
            },
            cx,
        );
    }

    fn watch_sign_in(&mut self, session: String, interval: Duration, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor().timer(interval).await;
                    let watching = this
                        .update(&mut cx, |view, _| {
                            view.sign_in
                                .as_ref()
                                .is_some_and(|sign_in| sign_in.session_id == session)
                        })
                        .unwrap_or(false);
                    if !watching {
                        break;
                    }
                    let id = session.clone();
                    let poll = cx
                        .background_spawn(async move { data::poll_sign_in(&id) })
                        .await;
                    let state = match &poll {
                        Ok(poll) => poll.state,
                        Err(_) => continue,
                    };
                    if state == SignInState::Pending {
                        continue;
                    }
                    let error = poll.ok().and_then(|poll| poll.error);
                    let _ = this.update(&mut cx, |view, cx| {
                        view.sign_in = None;
                        if state == SignInState::Authorized {
                            view.busy = Some(Busy::Connect);
                            view.run(data::connect, |_, (), _| {}, cx);
                        } else {
                            view.tell(
                                CustomPanelNoticeTone::Failure,
                                error.unwrap_or_else(|| {
                                    "GitHub did not accept the sign-in.".to_string()
                                }),
                                cx,
                            );
                        }
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
    }

    fn export(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let suggested = format!(
            "qol-profile-{}.json",
            chrono::Local::now().format("%Y-%m-%d")
        );
        let prompt = cx.prompt_for_new_path(&folder, Some(&suggested));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let Ok(Ok(Some(path))) = prompt.await else {
                    return;
                };
                let _ = this.update(&mut cx, |view, cx| {
                    let shown = path.display().to_string();
                    view.run(
                        move || data::export_to(&path),
                        move |view, (), cx| {
                            view.tell(CustomPanelNoticeTone::Success, format!("Saved {shown}"), cx)
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let Ok(Ok(Some(paths))) = prompt.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let _ = this.update(&mut cx, |view, cx| {
                    view.run(
                        move || data::import_from(&path),
                        |view, (), cx| {
                            view.tell(
                                CustomPanelNoticeTone::Success,
                                "Imported. This setup now matches the file.".to_string(),
                                cx,
                            )
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
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
        match escape_step(usize::from(self.level != Level::Main), false, true) {
            EscapeStep::CloseFilter => {}
            EscapeStep::PopCard => self.leave(),
            EscapeStep::AscendRail | EscapeStep::Dismiss => (self.on_back)(window, cx),
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        match intent(key, event.keystroke.key_char.as_deref(), self.naming) {
            Some(Intent::CancelEdit) => {
                self.naming = false;
                self.field.clear();
            }
            Some(Intent::CommitEdit) => self.new_profile(cx),
            _ if self.naming => {
                let mut next = self.field.clone();
                text_edit::apply_edit_key(&mut next, &event.keystroke, || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if next.text().len() <= MAX_NAME_BYTES {
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
                    self.activate(action, window, cx);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn render_label(&self, index: usize, row: &Row, ground: RowGround, kit: Kit) -> Div {
        if row.dot.is_none() && !row.spinner {
            return settings_label_group(
                row.label.clone(),
                (!row.detail.is_empty()).then(|| row.detail.clone().into()),
                ground,
                kit,
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_STACK))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(qol_theme::SPACE_INSET))
                    .children(
                        row.spinner.then(|| {
                            settings_action_spinner(("profiles-status-spinner", index), kit)
                        }),
                    )
                    .children(row.dot.map(|dot| status_dot(dot, kit)))
                    .child(settings_label(row.label.clone(), kit)),
            )
            .child(settings_description(row.detail.clone(), ground, kit))
    }

    fn render_values(
        &self,
        index: usize,
        row: &Row,
        selected: bool,
        ground: RowGround,
        kit: Kit,
    ) -> Div {
        let busy = selected && self.pending && !row.spinner;
        let mut values = settings_value_group();
        if busy && row.value != Value::Chip {
            values = values.child(settings_action_spinner(("profiles-spinner", index), kit));
        }
        match &row.value {
            Value::None => values,
            Value::Text(text, tone) => {
                values.child(settings_value_text(text.clone(), *tone, ground, kit))
            }
            Value::Choice { word, art, chevron } => values
                .child(
                    SettingsChoiceValue::new(
                        word.clone(),
                        ChoiceArt::Picture(art.clone()),
                        ground,
                        PictureContext::for_accent(
                            qol_theme::runtime_theme().mode,
                            qol_theme::runtime_accent_key(),
                        ),
                        kit,
                    )
                    .chevron(*chevron),
                )
                .children(
                    (row.chip_when_selected && selected)
                        .then_some(row.verb)
                        .flatten()
                        .map(|verb| {
                            settings_action_affordance(
                                ("profiles-action", index),
                                verb,
                                None,
                                busy,
                                ground,
                                kit,
                            )
                        }),
                ),
            Value::Open(text) => values
                .child(settings_value_text(
                    text.clone(),
                    SettingsValueTone::Normal,
                    ground,
                    kit,
                ))
                .child(SettingsChevron::new(ground, kit)),
            Value::Toggle(on) => values.child(SettingsToggle::new(*on, ground, kit)),
            Value::Chip => values.children(
                (!row.chip_when_selected || selected)
                    .then_some(row.verb)
                    .flatten()
                    .map(|verb| {
                        settings_action_affordance(
                            ("profiles-action", index),
                            verb,
                            None,
                            busy,
                            ground,
                            kit,
                        )
                    }),
            ),
            Value::Name => values.child(SettingsTextField::live(self.field.clone(), ground, kit)),
        }
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
        let action = row.action.clone();
        SettingsRow::setting(("profiles-row", index), kit)
            .selected(selected, focused)
            .on_click(cx.listener(move |view, _, window, cx| {
                if view.naming && action != Some(Action::NewProfile) {
                    return;
                }
                view.selected = index;
                if let Some(action) = action.clone() {
                    view.activate(action, window, cx);
                }
                cx.notify();
            }))
            .child(self.render_label(index, row, ground, kit))
            .child(self.render_values(index, row, selected, ground, kit))
            .into_any_element()
    }

    fn render_list(&self, rows: &[Row], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let total = rows.len();
        let cursor_group = rows
            .get(..=self.selected.min(total.saturating_sub(1)))
            .and_then(|seen| seen.iter().rposition(|row| row.header));
        let range = self.list.visible_range(total);
        let mut list = settings_list()
            .id("profiles-list")
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

impl CustomSettingsBreadcrumbs for ProfilesView {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        let title = match self.level {
            Level::Main => return Vec::new(),
            Level::Profiles => "profile",
            Level::Backups => "backups",
        };
        SettingsDestination::new(title).ok().into_iter().collect()
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        if self.naming {
            return Some(CustomHints {
                question: None,
                left: vec![
                    SettingsHint::new(Key::ENTER, "create"),
                    SettingsHint::new(Key::TYPE, "name"),
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

impl Focusable for ProfilesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ProfilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_polling(cx);
        let focused = self.focus.is_focused(window);
        let rows = self.rows();
        self.sync_selection(&rows);
        let page = settings_page();
        let page = if self.snapshot.is_none() {
            page.child(settings_busy_message(
                "profiles-loading",
                "Reading your profiles",
                kit(),
            ))
        } else {
            page.child(self.render_list(&rows, focused, cx))
        };
        page.id("profiles-body")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
    }
}

fn status_dot(dot: Dot, kit: Kit) -> Div {
    let (tone, halo) = match dot {
        Dot::Idle => (kit.grounds.pane.faint, 0),
        Dot::Success => (kit.palette.success, kit.washes.halo_success.packed()),
        Dot::Warning => (kit.palette.warning, kit.washes.halo_attention.packed()),
        Dot::Danger => (kit.palette.danger, kit.washes.halo_invalid.packed()),
    };
    kit.status_dot(tone, halo)
}
