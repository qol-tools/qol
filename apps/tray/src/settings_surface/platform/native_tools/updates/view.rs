use qol_gpui::key::Key;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use qol_connection_health::Level;
use qol_gpui::activity_animation::ActivityAnimation;
use qol_gpui::deck;
use qol_gpui::kit::kit;
use qol_gpui::kit::Kit;
use qol_gpui::scroll_list::{wheel_steps, ListHeight, ScrollList};
use qol_gpui::settings_panel::components::{
    settings_label, settings_label_group, settings_page, settings_value_group, HintTone, RowGround,
    SettingsFeedback, SettingsHint, DIMMED_OPACITY,
};
use qol_gpui::settings_panel::{
    adjacent_visible_row, escape_step, intent, settings_action_word, settings_busy_message,
    settings_connection_spinner, settings_description, settings_list, settings_tag, CustomHints,
    CustomPanelCallback, CustomPanelNoticeTone, CustomPanelNotifier, CustomSettingsBreadcrumbs,
    EscapeStep, Intent, SettingsDestination, SettingsGroupHeader, SettingsRow, SettingsValueTone,
    SETTINGS_LIST_GAP,
};
use qol_gpui::surface::SurfaceDismisser;

use super::data;
use super::model::{
    can_check, page, RowAction, Summary, SummaryDot, TargetRow, TargetState, UpdatesSnapshot,
    ValueTone,
};
use super::pull_request_build::{self, BuildRow, CardRow, Job};

const MAX_VISIBLE: usize = 10;
const CARD_DEPTH: usize = 1;
const CARD_ANIMATION: &str = "updates-card-slide";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    None,
    Summary,
    Check,
    Host,
    Plugin(String),
    PullRequestBuild,
}

enum PageRow {
    Header { title: String, detail: &'static str },
    Summary(Summary),
    Check(TargetRow),
    Host(TargetRow),
    Plugin(TargetRow),
    PullRequestBuild(BuildRow),
}

type Fold = Option<(usize, String)>;

struct Card {
    builds: Option<Result<Vec<qol_conventions::pr_try::ListedBuild>, String>>,
    selected: usize,
    list: ScrollList,
    list_height: ListHeight,
}

impl Card {
    fn new() -> Self {
        Self {
            builds: None,
            selected: 0,
            list: ScrollList::new(MAX_VISIBLE),
            list_height: ListHeight::default(),
        }
    }
}

fn rows_for(snapshot: &UpdatesSnapshot, shown: &BTreeSet<String>) -> (Vec<PageRow>, Fold) {
    let model = page(snapshot, shown);
    let mut rows = vec![
        PageRow::Header {
            title: "Status".to_string(),
            detail: "What the last check found.",
        },
        PageRow::Summary(model.summary),
        PageRow::Check(model.check),
        PageRow::Header {
            title: qol_conventions::artifact::TRAY_PACKAGE_NAME.to_string(),
            detail: "The tray itself.",
        },
        PageRow::Host(model.host),
        PageRow::Header {
            title: "Plugins".to_string(),
            detail: "Everything you installed.",
        },
    ];
    rows.extend(model.plugins.into_iter().map(PageRow::Plugin));
    let fold = model.fold.map(|text| (rows.len() - 1, text));
    if snapshot.can_try_pull_requests {
        rows.push(PageRow::Header {
            title: pull_request_build::GROUP.to_string(),
            detail: pull_request_build::GROUP_DETAIL,
        });
        rows.push(PageRow::PullRequestBuild(pull_request_build::row(snapshot)));
    }
    (rows, fold)
}

impl PageRow {
    fn action(&self) -> Option<&RowAction> {
        match self {
            PageRow::Header { .. } | PageRow::PullRequestBuild(_) => None,
            PageRow::Summary(summary) => summary.action.as_ref(),
            PageRow::Check(target) | PageRow::Host(target) | PageRow::Plugin(target) => {
                target.action.as_ref()
            }
        }
    }

    fn selection(&self) -> Option<Selection> {
        Some(match self {
            PageRow::Header { .. } => return None,
            PageRow::Summary(_) => Selection::Summary,
            PageRow::Check(_) => Selection::Check,
            PageRow::Host(_) => Selection::Host,
            PageRow::Plugin(target) => Selection::Plugin(target.id.clone()),
            PageRow::PullRequestBuild(_) => Selection::PullRequestBuild,
        })
    }

    fn action_label(&self) -> Option<&'static str> {
        match self {
            PageRow::Header { .. } => None,
            PageRow::PullRequestBuild(build) => build.opens.then_some("open"),
            PageRow::Summary(summary) => summary.action.as_ref().and(summary.action_label),
            PageRow::Check(target) | PageRow::Host(target) | PageRow::Plugin(target) => {
                target.action.as_ref().and(target.action_label)
            }
        }
    }
}

pub(super) struct UpdatesView {
    focus_handle: FocusHandle,
    body_focused: bool,
    dismisser: SurfaceDismisser,
    on_back: Option<CustomPanelCallback>,
    notify: CustomPanelNotifier,
    list: ScrollList,
    list_height: ListHeight,
    selection: Selection,
    selected: usize,
    snapshot: Option<UpdatesSnapshot>,
    visited: BTreeMap<String, Option<Instant>>,
    pending: bool,
    last_render: Option<Instant>,
    poll_skipped: bool,
    reloading: bool,
    card: Option<Card>,
    closing: Option<Card>,
    step: usize,
    motion: Option<deck::Motion>,
    mark: Option<f32>,
    body_bounds: deck::BodyBounds,
    row_bounds: deck::RowBounds,
}

impl UpdatesView {
    pub(super) fn new(
        dismisser: SurfaceDismisser,
        on_back: Option<CustomPanelCallback>,
        notify: CustomPanelNotifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            focus_handle: cx.focus_handle(),
            body_focused: false,
            dismisser,
            on_back,
            notify,
            list: ScrollList::new(MAX_VISIBLE),
            list_height: ListHeight::default(),
            selection: Selection::None,
            selected: 0,
            snapshot: None,
            visited: BTreeMap::new(),
            pending: false,
            last_render: None,
            poll_skipped: false,
            reloading: false,
            card: None,
            closing: None,
            step: 0,
            motion: None,
            mark: None,
            body_bounds: Rc::new(Cell::new(None)),
            row_bounds: Rc::new(RefCell::new(HashMap::new())),
        };
        Self::spawn_poll(cx);
        view
    }

    fn spawn_poll(cx: &mut Context<Self>) {
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                loop {
                    let Ok(due) = this.update(&mut async_cx, |view, _| view.take_poll_tick())
                    else {
                        break;
                    };
                    if due {
                        let result = async_cx.background_spawn(async { data::load() }).await;
                        let alive = this
                            .update(&mut async_cx, |view, cx| {
                                if let Ok(snapshot) = result {
                                    view.apply_snapshot(snapshot);
                                }
                                cx.notify();
                            })
                            .is_ok();
                        if !alive {
                            break;
                        }
                    }
                    async_cx.background_executor().timer(POLL_INTERVAL).await;
                }
            }
        })
        .detach();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.reloading || self.snapshot.is_none() {
            return;
        }
        self.reloading = true;
        cx.spawn(|this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx.background_spawn(async { data::load() }).await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    view.reloading = false;
                    if let Ok(snapshot) = result {
                        view.apply_snapshot(snapshot);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn take_poll_tick(&mut self) -> bool {
        let due = self
            .last_render
            .is_none_or(|last| last.elapsed() < POLL_STALE_AFTER);
        if !due {
            self.poll_skipped = true;
        }
        due
    }

    fn comes_into_view(&self) -> bool {
        self.last_render
            .is_none_or(|last| last.elapsed() >= POLL_STALE_AFTER)
    }

    fn connection(&self) -> Level {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.connection.level)
            .unwrap_or_default()
    }

    fn apply_snapshot(&mut self, snapshot: UpdatesSnapshot) {
        if self.poll_skipped {
            self.poll_skipped = false;
            self.visited.retain(|id, _| {
                snapshot
                    .plugins
                    .iter()
                    .any(|plugin| &plugin.id == id && !finished(plugin.state))
            });
        }
        self.note_visited(&snapshot);
        self.snapshot = Some(snapshot);
    }

    fn note_visited(&mut self, snapshot: &UpdatesSnapshot) {
        for plugin in &snapshot.plugins {
            if !finished(plugin.state) {
                self.visited.insert(plugin.id.clone(), None);
            } else if let Some(since @ None) = self.visited.get_mut(&plugin.id) {
                *since = Some(Instant::now());
            }
        }
    }

    fn forget_faded(&mut self) {
        self.visited.retain(|_, since| {
            since.is_none_or(|since| {
                since.elapsed() < qol_theme::STAY_BRIEF + qol_theme::Motion::FADE.duration
            })
        });
    }

    fn fading(&self) -> bool {
        self.visited.values().any(Option::is_some)
    }

    fn row_opacity(&self, id: &str) -> f32 {
        let Some(Some(since)) = self.visited.get(id) else {
            return 1.0;
        };
        fade_opacity(since.elapsed(), qol_gpui::motion::moving())
    }

    fn page_rows(&self) -> (Vec<PageRow>, Fold) {
        let Some(snapshot) = &self.snapshot else {
            return (Vec::new(), None);
        };
        let shown: BTreeSet<String> = self.visited.keys().cloned().collect();
        rows_for(snapshot, &shown)
    }

    fn needs_tray(&self) -> Option<u64> {
        self.snapshot
            .as_ref()
            .and_then(pull_request_build::needs_tray)
    }

    fn card_rows(&self, card: &Card) -> Vec<CardRow> {
        let trial = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.trial.as_ref());
        let builds = match &card.builds {
            Some(Ok(builds)) => builds.as_slice(),
            _ => &[],
        };
        pull_request_build::card_rows(builds, trial)
    }

    fn open_card(&mut self, cx: &mut Context<Self>) {
        self.mark = deck::row_mark(&self.row_bounds, self.selected, self.body_bounds.get());
        self.closing = None;
        self.card = Some(Card::new());
        self.step = self.step.wrapping_add(1);
        self.motion = Some(deck::Motion::Push);
        let failed = self.snapshot.as_ref().is_some_and(|snapshot| {
            matches!(snapshot.pull_request_build, Some(Job::Failed { .. }))
        });
        if failed {
            self.run_action(RowAction::KeepTray, cx);
        }
        let step = self.step;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let builds = async_cx
                    .background_spawn(async { data::load_builds() })
                    .await
                    .map_err(|error| format!("{error:#}"));
                let _ = this.update(&mut async_cx, |view, cx| {
                    if view.step != step {
                        return;
                    }
                    let Some(mut card) = view.card.take() else {
                        return;
                    };
                    card.builds = Some(builds);
                    let rows = view.card_rows(&card);
                    card.selected = rows.iter().position(|row| row.running).unwrap_or(0);
                    card.list.selected = card.selected;
                    card.list.sync(rows.len());
                    view.card = Some(card);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.card.take() else {
            return;
        };
        self.closing = Some(card);
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

    fn move_card(&mut self, direction: isize) {
        let Some(card) = &self.card else {
            return;
        };
        let total = self.card_rows(card).len();
        let Some(card) = self
            .card
            .as_mut()
            .filter(|card| card.builds.is_some() && total > 0)
        else {
            return;
        };
        let indices = (0..total).collect::<Vec<_>>();
        card.selected = adjacent_visible_row(&indices, card.selected, direction);
        card.list.selected = card.selected;
        card.list.sync(total);
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.card.as_ref().filter(|card| card.builds.is_some()) else {
            return;
        };
        let action = self
            .card_rows(card)
            .get(card.selected)
            .and_then(|row| row.action.clone());
        if let Some(action) = action {
            self.run_action(action, cx);
        }
        self.leave(cx);
    }

    fn sync_selection(&mut self, rows: &[PageRow]) {
        let navigable = navigable(rows);
        if navigable.is_empty() {
            self.selection = Selection::None;
            self.selected = 0;
            self.list.selected = 0;
            self.list.sync(rows.len());
            return;
        }
        let position = navigable
            .iter()
            .position(|(_, selection)| *selection == self.selection)
            .unwrap_or_else(|| nearest_navigable(&navigable, self.selected));
        self.select_position(&navigable, position, rows.len());
    }

    fn move_selection(&mut self, direction: isize) {
        let (rows, _) = self.page_rows();
        let navigable = navigable(&rows);
        if navigable.is_empty() {
            return;
        }
        let indices = navigable
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        let next = adjacent_visible_row(&indices, self.selected, direction);
        let position = indices.iter().position(|index| *index == next).unwrap_or(0);
        self.select_position(&navigable, position, rows.len());
    }

    fn select_position(&mut self, navigable: &[(usize, Selection)], position: usize, rows: usize) {
        self.selected = navigable[position].0;
        self.selection = navigable[position].1.clone();
        self.list.selected = self.selected;
        self.list.sync(rows);
    }

    fn select_row(&mut self, index: usize, selection: Selection, cx: &mut Context<Self>) {
        self.selected = index;
        self.selection = selection;
        self.activate(cx);
        cx.notify();
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        let (rows, _) = self.page_rows();
        if let Some(PageRow::PullRequestBuild(build)) = rows.get(self.selected) {
            if build.opens {
                self.open_card(cx);
            }
            return;
        }
        let Some(action) = rows.get(self.selected).and_then(PageRow::action).cloned() else {
            return;
        };
        self.run_action(action, cx);
    }

    fn run_action(&mut self, action: RowAction, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let (name, body) = match &action {
            RowAction::Check => ("check_updates", None),
            RowAction::UpdateAll => ("update_all", None),
            RowAction::StopUpdates => ("stop_updates", None),
            RowAction::Update(id) => ("update", Some(serde_json::json!({ "id": id }).to_string())),
            pull_request => match pull_request_build::action_request(pull_request) {
                Some(request) => request,
                None => return,
            },
        };
        self.pending = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let result = async_cx
                    .background_spawn(async move { data::action(name, body.as_deref()) })
                    .await;
                match result {
                    Ok(()) => {
                        let snapshot = async_cx.background_spawn(async { data::load() }).await;
                        let _ = this.update(&mut async_cx, |view, cx| {
                            view.pending = false;
                            if let Ok(snapshot) = snapshot {
                                view.apply_snapshot(snapshot);
                            }
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        let _ = this.update(&mut async_cx, |view, cx| {
                            view.pending = false;
                            view.fail(&message, cx);
                            cx.notify();
                        });
                    }
                }
            }
        })
        .detach();
    }

    fn fail(&self, message: &str, cx: &mut Context<Self>) {
        (self.notify)(CustomPanelNoticeTone::Failure, message.to_string(), cx);
    }

    fn go_back(&self, window: &mut Window, cx: &mut App) {
        if let Some(on_back) = &self.on_back {
            on_back(window, cx);
        } else {
            self.dismisser.dismiss(cx);
        }
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.card.is_none() && self.needs_tray().is_some() {
            self.run_action(RowAction::KeepTray, cx);
            return;
        }
        match escape_step(
            usize::from(self.card.is_some()),
            false,
            self.on_back.is_some(),
        ) {
            EscapeStep::CloseFilter => {}
            EscapeStep::PopCard => self.leave(cx),
            EscapeStep::AscendRail | EscapeStep::Dismiss => self.go_back(window, cx),
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        if matches!(key, "escape" | "esc") {
            self.escape(window, cx);
            cx.notify();
            return;
        }
        if self.card.is_some() {
            match intent(key, None, false) {
                Some(Intent::Up) => self.move_card(-1),
                Some(Intent::Down) => self.move_card(1),
                Some(Intent::Activate) => self.choose(cx),
                _ => return,
            }
            cx.notify();
            return;
        }
        if let Some(number) = self.needs_tray() {
            if intent(key, None, false) == Some(Intent::Activate) {
                self.run_action(RowAction::TryWithTray(number), cx);
                cx.notify();
            }
            return;
        }
        if key == "c" && !event.keystroke.modifiers.modified() {
            if self.snapshot.as_ref().is_some_and(can_check) {
                self.run_action(RowAction::Check, cx);
            }
            return;
        }
        match intent(key, None, false) {
            Some(Intent::Up) => self.move_selection(-1),
            Some(Intent::Down) => self.move_selection(1),
            Some(Intent::Activate) => self.activate(cx),
            _ => return,
        }
        cx.notify();
    }

    fn render_body(&self, rows: &[PageRow], fold: &Fold, cx: &mut Context<Self>) -> AnyElement {
        if self.snapshot.is_none() {
            return settings_busy_message("updates-loading", "Loading updates", kit())
                .into_any_element();
        }
        let kit = kit();
        let width = deck::body_width(self.body_bounds.get());
        if let Some(card) = &self.card {
            return deck::shell(deck::render(
                kit,
                self.render_card(card, true, cx),
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
        let page = settings_page().child(self.render_list(rows, fold, cx));
        let Some(leaving) = &self.closing else {
            return page.into_any_element();
        };
        deck::shell(deck::reveal(
            kit,
            page,
            self.render_card(leaving, false, cx),
            deck::exit(self.step, CARD_DEPTH, width),
        ))
    }

    fn render_card(&self, card: &Card, live: bool, cx: &mut Context<Self>) -> Div {
        let kit = kit();
        let page = settings_page();
        match &card.builds {
            None => page.child(settings_busy_message(
                ("updates-card-loading", usize::from(live)),
                "Getting pull requests",
                kit,
            )),
            Some(builds) => page
                .children(
                    builds
                        .as_ref()
                        .err()
                        .map(|message| SettingsFeedback::new(message.clone(), true)),
                )
                .child(self.render_card_list(card, live, cx)),
        }
    }

    fn render_card_list(&self, card: &Card, live: bool, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.card_rows(card);
        let focused = live && self.body_focused;
        let total = rows.len();
        let range = card.list.visible_range(total);
        let mut list = settings_list()
            .id(("updates-card-list", usize::from(live)))
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    for step in wheel_steps(&event.delta, qol_theme::HEIGHT_SETTING_ROW) {
                        this.move_card(step);
                    }
                    cx.notify();
                }),
            );
        for index in range.clone() {
            let selected = live && card.selected == index;
            list = list.child(self.render_card_row(index, &rows[index], selected, focused, cx));
        }
        list.child(card.list_height.recorder(cx.weak_entity()))
            .child(kit().scroll_cue(
                ("updates-card-list-bar", usize::from(live)),
                qol_gpui::scrollbar::ScrollSource::window(
                    range.start,
                    range.len(),
                    total,
                    cx.listener(move |view: &mut Self, at: &f32, _, cx| {
                        if let Some(card) = view.card.as_mut() {
                            card.list.seek(*at, total);
                            card.selected = card.list.selected;
                        }
                        cx.notify();
                    }),
                ),
                kit().grounds.pane,
            ))
            .into_any_element()
    }

    fn render_card_row(
        &self,
        index: usize,
        row: &CardRow,
        selected: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        let ground = RowGround::of(selected, focused);
        SettingsRow::setting(("updates-card-row", index), kit)
            .selected(selected, focused)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                if let Some(card) = this.card.as_mut() {
                    card.selected = index;
                }
                this.choose(cx);
                cx.notify();
            }))
            .child(settings_label_group(
                row.label.clone(),
                (!row.detail.is_empty()).then(|| row.detail.clone().into()),
                ground,
                kit,
            ))
            .child(
                settings_value_group()
                    .children(
                        row.running.then(|| {
                            settings_tag("running", SettingsValueTone::Muted, ground, kit)
                        }),
                    )
                    .children(row.verb.map(|verb| {
                        settings_action_word(
                            ("updates-card-action", index),
                            verb,
                            false,
                            ground,
                            kit,
                        )
                    })),
            )
            .into_any_element()
    }

    fn render_build(&self, index: usize, build: &BuildRow, cx: &mut Context<Self>) -> AnyElement {
        let kit = kit();
        let selected = self.selected == index;
        let ground = RowGround::of(selected, self.body_focused);
        SettingsRow::setting(("updates-pull-request-build", index), kit)
            .selected(selected, self.body_focused)
            .attention(build.attention)
            .opens(build.opens)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, Selection::PullRequestBuild, cx);
                }
            }))
            .child(settings_label_group(
                pull_request_build::TITLE,
                Some(build.description.clone().into()),
                ground,
                kit,
            ))
            .child(
                settings_value_group()
                    .children(build.spinner.then(|| {
                        settings_connection_spinner(
                            ("updates-build-spinner", index),
                            self.connection(),
                            kit,
                        )
                    }))
                    .child(settings_tag(
                        build.value.clone(),
                        SettingsValueTone::Normal,
                        ground,
                        kit,
                    )),
            )
            .child(deck::bounds_recorder(Rc::clone(&self.row_bounds), index))
            .into_any_element()
    }

    fn render_list(&self, rows: &[PageRow], fold: &Fold, cx: &mut Context<Self>) -> AnyElement {
        let total = rows.len();
        let mut list = settings_list()
            .id("updates-list")
            .on_scroll_wheel(
                cx.listener(|this: &mut Self, event: &ScrollWheelEvent, _, cx| {
                    for step in wheel_steps(&event.delta, qol_theme::HEIGHT_SETTING_ROW) {
                        this.move_selection(step);
                    }
                    cx.notify();
                }),
            );
        let cursor_group = self
            .selected
            .checked_add(1)
            .and_then(|end| rows.get(..end))
            .and_then(|seen| {
                seen.iter()
                    .rposition(|row| matches!(row, PageRow::Header { .. }))
            });
        let range = self.list.visible_range(total);
        for index in range.clone() {
            let current = self.body_focused && cursor_group == Some(index);
            list = list.child(self.render_row(index, &rows[index], current, cx));
            if let Some((_, text)) = fold.as_ref().filter(|(at, _)| *at == index) {
                list = list.child(self.render_fold(text));
            }
        }
        list = list.child(self.list_height.recorder(cx.weak_entity()));
        list = list.child(kit().scroll_cue(
            "updates-list-bar",
            qol_gpui::scrollbar::ScrollSource::window(
                range.start,
                range.len(),
                total,
                cx.listener(move |view: &mut Self, at: &f32, _, cx| {
                    view.list.seek(*at, total);
                    let (rows, _) = view.page_rows();
                    let navigable = navigable(&rows);
                    if !navigable.is_empty() {
                        let position = nearest_navigable(&navigable, view.list.selected);
                        view.select_position(&navigable, position, rows.len());
                    }
                    cx.notify();
                }),
            ),
            kit().grounds.pane,
        ));
        list.into_any_element()
    }

    fn render_fold(&self, text: &str) -> Div {
        let kit = kit();
        div()
            .flex_none()
            .h(px(qol_theme::HEIGHT_CONTROL))
            .flex()
            .items_center()
            .px(px(qol_theme::SPACE_INSET))
            .child(settings_description(
                text.to_string(),
                RowGround::of(false, self.body_focused),
                kit,
            ))
    }

    fn render_row(
        &self,
        index: usize,
        row: &PageRow,
        current: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        match row {
            PageRow::Header { title, detail } => {
                SettingsGroupHeader::new(title.clone(), Some((*detail).into()), kit)
                    .current(current)
                    .into_any_element()
            }
            PageRow::Summary(summary) => self.render_summary(index, summary, cx),
            PageRow::PullRequestBuild(build) => self.render_build(index, build, cx),
            PageRow::Check(target) => self.render_target(index, target, Selection::Check, cx),
            PageRow::Host(target) => self.render_target(index, target, Selection::Host, cx),
            PageRow::Plugin(target) => {
                let selection = Selection::Plugin(target.id.clone());
                let row = self.render_target(index, target, selection, cx);
                let opacity = self.row_opacity(&target.id);
                if opacity < 1.0 {
                    div()
                        .w_full()
                        .opacity(opacity)
                        .child(row)
                        .into_any_element()
                } else {
                    row
                }
            }
        }
    }

    fn render_summary(
        &self,
        index: usize,
        summary: &Summary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        let ground = RowGround::of(self.selected == index, self.body_focused);
        let row = SettingsRow::setting(("updates-summary", index), kit)
            .selected(self.selected == index, self.body_focused)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, Selection::Summary, cx);
                }
            }));
        let label = div()
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
                    .children(summary.busy.then(|| {
                        settings_connection_spinner(
                            ("updates-summary-spinner", index),
                            self.connection(),
                            kit,
                        )
                    }))
                    .children(summary.dot.map(|dot| summary_dot(dot, kit)))
                    .child(settings_label(summary.label.clone(), kit)),
            )
            .child(settings_description(
                summary.description.clone(),
                ground,
                kit,
            ));
        row.child(label)
            .child(
                settings_value_group().children(summary.action_label.map(|verb| {
                    settings_action_word(
                        ("updates-summary-action", index),
                        verb.to_lowercase(),
                        false,
                        ground,
                        kit,
                    )
                    .when(summary.dimmed(), |control| control.opacity(DIMMED_OPACITY))
                })),
            )
            .into_any_element()
    }

    fn render_target(
        &self,
        index: usize,
        target: &TargetRow,
        selection: Selection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kit = kit();
        let ground = RowGround::of(self.selected == index, self.body_focused);
        let row = SettingsRow::setting(("updates-target", index), kit)
            .selected(self.selected == index, self.body_focused)
            .attention(target.attention)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.standard_click() {
                    this.select_row(index, selection.clone(), cx);
                }
            }));
        row.child(settings_label_group(
            target.name.clone(),
            target.description.clone().map(Into::into),
            ground,
            kit,
        ))
        .child(
            settings_value_group()
                .children(target.spinner.then(|| {
                    settings_connection_spinner(
                        ("updates-target-spinner", index),
                        self.connection(),
                        kit,
                    )
                }))
                .child(settings_tag(
                    target.value.clone(),
                    value_tone(target.tone),
                    ground,
                    kit,
                ))
                .children(target.action_label.map(|verb| {
                    settings_action_word(
                        ("updates-target-action", index),
                        verb.to_lowercase(),
                        false,
                        ground,
                        kit,
                    )
                    .when(target.dimmed(), |control| control.opacity(DIMMED_OPACITY))
                })),
        )
        .into_any_element()
    }
}

impl CustomSettingsBreadcrumbs for UpdatesView {
    fn settings_page_shown(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination> {
        if self.card.is_none() {
            return Vec::new();
        }
        SettingsDestination::new(pull_request_build::TITLE)
            .ok()
            .into_iter()
            .collect()
    }

    fn settings_hints(&self) -> Option<CustomHints> {
        if let Some(card) = &self.card {
            let mut left = Vec::new();
            let verb = self
                .card_rows(card)
                .get(card.selected)
                .and_then(|row| row.verb)
                .filter(|_| card.builds.is_some());
            if let Some(verb) = verb {
                left.push(SettingsHint::new(Key::ENTER, verb));
            }
            left.push(SettingsHint::new(Key::UP_DOWN, "move"));
            return Some(CustomHints {
                question: None,
                left,
                right: Vec::new(),
            });
        }
        if let Some(number) = self.needs_tray() {
            return Some(CustomHints {
                question: Some(pull_request_build::tray_question(number).into()),
                left: vec![SettingsHint::new(Key::ENTER, "restart").tone(HintTone::Discard)],
                right: vec![SettingsHint::new(Key::ESC, "keep yours")],
            });
        }
        let (rows, _) = self.page_rows();
        let mut left = Vec::new();
        let selected = rows.get(self.selected);
        if let Some(label) = selected.and_then(PageRow::action_label) {
            left.push(SettingsHint::new(Key::ENTER, label.to_lowercase()));
        }
        let enter_checks = selected.and_then(PageRow::action) == Some(&RowAction::Check);
        if !enter_checks && self.snapshot.as_ref().is_some_and(can_check) {
            left.push(SettingsHint::new(Key::letter('c'), "check now"));
        }
        left.push(SettingsHint::new(Key::UP_DOWN, "move"));
        Some(CustomHints {
            question: None,
            left,
            right: Vec::new(),
        })
    }
}

impl Focusable for UpdatesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UpdatesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.body_focused = self.focus_handle.is_focused(window);
        if self.comes_into_view() {
            cx.background_spawn(async { super::super::check_updates() })
                .detach();
            self.reload(cx);
        }
        self.last_render = Some(Instant::now());
        self.forget_faded();
        let (rows, fold) = self.page_rows();
        self.sync_selection(&rows);
        self.list
            .fit_to(&self.list_height, rows.len(), SETTINGS_LIST_GAP, |index| {
                let folded = fold.as_ref().is_some_and(|(at, _)| *at == index);
                row_height(&rows[index], folded)
            });
        if let Some(card) = &self.card {
            let total = self.card_rows(card).len();
            if let Some(card) = self.card.as_mut() {
                card.list
                    .fit_to(&card.list_height, total, SETTINGS_LIST_GAP, |_| {
                        qol_theme::HEIGHT_SETTING_ROW
                    });
            }
        }
        div()
            .id("qol-native-updates-body")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .child(deck::body_recorder(Rc::clone(&self.body_bounds)))
            .child(
                div()
                    .id("qol-native-updates-content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(ActivityAnimation::new(
                        "qol-native-updates-fade",
                        self.fading(),
                        self.render_body(&rows, &fold, cx),
                    )),
            )
    }
}

fn row_height(row: &PageRow, folded: bool) -> f32 {
    let height = match row {
        PageRow::Header { .. } => SettingsGroupHeader::height_with_detail(),
        PageRow::Summary(_)
        | PageRow::Check(_)
        | PageRow::Host(_)
        | PageRow::Plugin(_)
        | PageRow::PullRequestBuild(_) => qol_theme::HEIGHT_SETTING_ROW,
    };
    match folded {
        true => height + SETTINGS_LIST_GAP + qol_theme::HEIGHT_CONTROL,
        false => height,
    }
}

fn fade_opacity(elapsed: Duration, moving: bool) -> f32 {
    if elapsed <= qol_theme::STAY_BRIEF {
        return 1.0;
    }
    let fade = qol_theme::Motion::FADE;
    let fading = elapsed - qol_theme::STAY_BRIEF;
    1.0 - qol_gpui::motion::ease_when(
        fade,
        fading.as_secs_f32() / fade.duration.as_secs_f32(),
        moving,
    )
}

fn finished(state: TargetState) -> bool {
    matches!(
        state,
        TargetState::UpToDate | TargetState::Unchecked | TargetState::DevLinked
    )
}

fn navigable(rows: &[PageRow]) -> Vec<(usize, Selection)> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| row.selection().map(|selection| (index, selection)))
        .collect()
}

fn nearest_navigable(navigable: &[(usize, Selection)], index: usize) -> usize {
    navigable
        .iter()
        .position(|(row, _)| *row >= index)
        .unwrap_or_else(|| navigable.len().saturating_sub(1))
}

fn summary_dot(dot: SummaryDot, kit: Kit) -> Div {
    let (tone, halo) = match dot {
        SummaryDot::Danger => (kit.palette.danger, kit.washes.halo_invalid),
        SummaryDot::Warning => (kit.palette.warning, kit.washes.halo_attention),
        SummaryDot::Success => (kit.palette.success, kit.washes.halo_success),
    };
    kit.status_dot(tone, halo.packed())
}

fn value_tone(tone: ValueTone) -> SettingsValueTone {
    match tone {
        ValueTone::Normal => SettingsValueTone::Normal,
        ValueTone::Muted => SettingsValueTone::Muted,
        ValueTone::Attention => SettingsValueTone::Attention,
        ValueTone::Danger => SettingsValueTone::Danger,
        ValueTone::Success => SettingsValueTone::Success,
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Summary, TargetRow, TargetState, UpdatesSnapshot, ValueTone};
    use super::{
        fade_opacity, finished, navigable, nearest_navigable, rows_for, PageRow, Selection,
    };
    use std::collections::BTreeSet;
    use std::time::Duration;

    fn snapshot(can_try_pull_requests: bool) -> UpdatesSnapshot {
        let target = |id: &str, state: &str| {
            format!(
                r#"{{"id":"{id}","name":"{id}","current":"1.0.0","latest":"1.1.0","state":"{state}"}}"#
            )
        };
        serde_json::from_str(&format!(
            r#"{{"checking":false,"checks_enabled":true,"running":false,"last_checked_secs":60,"can_try_pull_requests":{can_try_pull_requests},"host":{},"plugins":[{},{}]}}"#,
            target("qol-tray", "up_to_date"),
            target("launcher", "available"),
            target("lights", "up_to_date"),
        ))
        .unwrap()
    }

    #[test]
    fn the_development_rows_follow_the_plugins_only_while_the_flag_is_on() {
        let (off, off_fold) = rows_for(&snapshot(false), &BTreeSet::new());
        let (on, on_fold) = rows_for(&snapshot(true), &BTreeSet::new());
        assert_eq!(on.len(), off.len() + 2);
        assert!(matches!(off.last(), Some(PageRow::Plugin(row)) if row.id == "launcher"));
        assert!(matches!(
            &on[off.len()],
            PageRow::Header { title, .. } if title == "Development"
        ));
        assert!(matches!(on.last(), Some(PageRow::PullRequestBuild(_))));
        let anchor = off.len() - 1;
        assert_eq!(off_fold.as_ref().map(|fold| fold.0), Some(anchor));
        assert_eq!(on_fold, off_fold);
    }

    #[test]
    fn only_rows_with_nothing_left_to_do_count_as_finished() {
        let cases = [
            (TargetState::UpToDate, true),
            (TargetState::Unchecked, true),
            (TargetState::DevLinked, true),
            (TargetState::Available, false),
            (TargetState::Queued, false),
            (TargetState::Updating, false),
            (TargetState::Failed, false),
            (TargetState::Trial, false),
        ];
        for (state, expected) in cases {
            assert_eq!(finished(state), expected, "state: {state:?}");
        }
    }

    fn summary() -> Summary {
        Summary {
            dot: None,
            busy: false,
            label: "Everything is up to date".to_string(),
            description: "No updates are waiting".to_string(),
            action: None,
            action_label: None,
        }
    }

    fn target() -> TargetRow {
        TargetRow {
            id: "qol-tray".to_string(),
            name: "Version".to_string(),
            value: "3.66.1".to_string(),
            tone: ValueTone::Muted,
            description: Some("Updates come from Recompile".to_string()),
            action: None,
            action_label: None,
            attention: false,
            spinner: false,
        }
    }

    #[test]
    fn finished_row_holds_then_fades_out() {
        let cases = [
            (0, 1.0),
            (4000, 1.0),
            (4500, 0.03125),
            (5000, 0.0),
            (9000, 0.0),
        ];
        for (millis, expected) in cases {
            let opacity = fade_opacity(Duration::from_millis(millis), true);
            assert!(
                (opacity - expected).abs() < 0.001,
                "{millis} ms gave {opacity}"
            );
        }
    }

    #[test]
    fn finished_row_holds_then_vanishes_without_motion() {
        let cases = [(0, 1.0), (4000, 1.0), (4001, 0.0), (9000, 0.0)];
        for (millis, expected) in cases {
            let opacity = fade_opacity(Duration::from_millis(millis), false);
            assert_eq!(opacity, expected, "{millis} ms");
        }
    }

    #[test]
    fn every_row_but_a_header_navigates_even_without_an_action() {
        let rows = vec![
            PageRow::Header {
                title: "Status".to_string(),
                detail: "What the last check found.",
            },
            PageRow::Summary(summary()),
            PageRow::Header {
                title: "qol-tray".to_string(),
                detail: "The tray itself.",
            },
            PageRow::Host(target()),
        ];
        let navigable = navigable(&rows);
        assert_eq!(
            navigable,
            vec![(1, Selection::Summary), (3, Selection::Host)]
        );
    }

    #[test]
    fn a_dimmed_button_shows_but_offers_no_enter_hint() {
        let dimmed = TargetRow {
            action_label: Some("Check now"),
            ..target()
        };
        assert!(dimmed.dimmed());
        assert_eq!(PageRow::Check(dimmed).action_label(), None);
        assert!(!target().dimmed());
    }

    #[test]
    fn nearest_navigable_keeps_the_cursor_close_when_a_row_disappears() {
        let navigable = [
            (2usize, Selection::Summary),
            (5, Selection::Plugin("bt".to_string())),
            (9, Selection::Host),
        ];
        let cases = [(0, 0), (2, 0), (3, 1), (5, 1), (8, 2), (40, 2)];
        for (index, expected) in cases {
            assert_eq!(
                nearest_navigable(&navigable, index),
                expected,
                "index={index}"
            );
        }
    }
}
