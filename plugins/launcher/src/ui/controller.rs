use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{px, size, AppContext as _, AsyncApp, ClipboardItem, Context, KeyDownEvent, WeakEntity};

use super::feedback::{Copied, Cue, Feedback, Placed};
use super::input::InputEffect;
use super::layout::{full_window_height, window_height_for_detail, WINDOW_WIDTH};
use super::trace;
use super::LauncherView;
use crate::discovery::search::{ResultItem, ResultSource, SearchMode};

const DETAIL_SCROLL_STEP: f32 = 54.0;

enum CopyPart {
    Path,
    Name,
}

enum ClipboardShortcut {
    Copy,
    Cut,
    Paste,
}

impl LauncherView {
    pub(super) fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.feedback.as_ref().is_some_and(Feedback::closing) {
            return;
        }
        self.state.feedback = None;
        if self.handle_menu_key(event, cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let secondary = event.keystroke.modifiers.secondary();

        if self.handle_clipboard_shortcut(key, secondary, cx) {
            return;
        }
        let flow_active = self.state.flow.is_some();
        if !flow_active {
            self.store.ensure_filtered(
                self.state.query.text(),
                self.state.mode,
                self.state.fuzziness,
            );
        }
        let result_count = if flow_active {
            self.state.flow_result_count()
        } else {
            self.store.result_count()
        };
        let selected_before = self.state.scroll_list.selected;
        let effect = self
            .state
            .apply_key(key, &event.keystroke.modifiers, result_count);
        trace::input(
            self,
            key,
            effect,
            result_count,
            selected_before,
            event.is_held,
            event.keystroke.key_char.as_deref(),
        );

        if !matches!(effect, InputEffect::BoostUp | InputEffect::BoostDown) {
            self.state.boost_adjusting = false;
        }

        match effect {
            InputEffect::Ignore => {}
            InputEffect::Navigate => {
                self.state.sync_result_window(result_count);
                cx.notify();
            }
            InputEffect::QueryChanged | InputEffect::FlowQueryChanged => {
                self.dispatch_query_change(cx)
            }
            InputEffect::Tune { narrower, changed } => self.tune(narrower, changed, cx),
            InputEffect::BoostUp => self.step_selected_rank(true, cx),
            InputEffect::BoostDown => self.step_selected_rank(false, cx),
            InputEffect::Launch => self.launch_selected(cx),
            InputEffect::OpenFolder => self.open_selected_folder(cx),
            InputEffect::CopyPath => self.copy_chosen_file(CopyPart::Path, cx),
            InputEffect::CopyName => self.copy_chosen_file(CopyPart::Name, cx),
            InputEffect::Dismiss => self.hide_to_ghost("key", window),
            InputEffect::FlowExit => {
                self.state.exit_flow();
                trace::flow(self, "exited");
                cx.notify();
            }
            InputEffect::FlowActivate => self.activate_flow_row(window, cx),
            InputEffect::FlowDetail => self.open_flow_detail(window, cx),
            InputEffect::FlowDetailClose => self.close_flow_detail(window, cx),
            InputEffect::FlowDetailScrollUp => self.scroll_flow_detail(-DETAIL_SCROLL_STEP, cx),
            InputEffect::FlowDetailScrollDown => self.scroll_flow_detail(DETAIL_SCROLL_STEP, cx),
            InputEffect::FlowDislike => self.dislike_flow_row(cx),
        }
    }

    fn handle_clipboard_shortcut(
        &mut self,
        key: &str,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(shortcut) = Self::clipboard_shortcut(key, secondary) else {
            return false;
        };
        let in_results = self.state.flow.is_none() && self.state.list_focused;
        if in_results && matches!(shortcut, ClipboardShortcut::Copy) {
            let copied = match self.state.mode {
                SearchMode::Files => {
                    self.copy_chosen_file(CopyPart::Path, cx);
                    true
                }
                SearchMode::Apps => self.copy_chosen_app_path(cx),
            };
            if copied {
                return true;
            }
        }
        self.apply_clipboard_shortcut(shortcut, cx);
        true
    }

    fn copy_chosen_file(&mut self, part: CopyPart, cx: &mut Context<Self>) {
        let (text, name) = match self
            .store
            .get(self.state.scroll_list.selected)
            .and_then(|scored| self.store.item(scored))
        {
            Some(ResultItem::File(entry)) => (
                match part {
                    CopyPart::Path => entry.path.display().to_string(),
                    CopyPart::Name => entry.name.clone(),
                },
                entry.name.clone(),
            ),
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let what = match part {
            CopyPart::Path => Copied::Path,
            CopyPart::Name => Copied::Name,
        };
        self.show_cue(Cue::Copied { what, name }, Vec::new(), cx);
    }

    fn copy_chosen_app_path(&mut self, cx: &mut Context<Self>) -> bool {
        let found = self
            .store
            .get(self.state.scroll_list.selected)
            .and_then(|scored| self.store.item(scored))
            .and_then(|item| match item {
                ResultItem::App(entry) => Some(entry),
                _ => None,
            })
            .and_then(|entry| {
                let binary = self.details.app(&entry.path)?.binary.as_ref()?;
                Some((binary.display().to_string(), entry.name.clone()))
            });
        let Some((path, name)) = found else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path));
        self.show_cue(
            Cue::Copied {
                what: Copied::Path,
                name,
            },
            Vec::new(),
            cx,
        );
        true
    }

    pub(super) fn show_cue(&mut self, cue: Cue, before: Vec<Placed>, cx: &mut Context<Self>) {
        let feedback = Feedback::new(cue, before);
        if let Some(delay) = feedback.cue.closes_after() {
            self.close_after(feedback.seq, delay, dismiss_reason(&feedback.cue), cx);
        }
        self.state.feedback = Some(feedback);
        cx.notify();
    }

    fn close_after(
        &mut self,
        seq: u64,
        delay: Duration,
        from: &'static str,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx.background_executor().timer(delay).await;
                this.update(&mut async_cx, |view, cx| {
                    let current = view
                        .state
                        .feedback
                        .as_ref()
                        .is_some_and(|feedback| feedback.seq == seq);
                    if current && view.is_showing {
                        view.request_dismiss(from);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn tune(&mut self, narrower: bool, changed: bool, cx: &mut Context<Self>) {
        let cue = if changed {
            self.dispatch_query_change(cx);
            self.store.ensure_filtered(
                self.state.query.text(),
                self.state.mode,
                self.state.fuzziness,
            );
            Cue::Level {
                level: self.state.fuzziness,
                count: self.store.result_count(),
            }
        } else if narrower {
            Cue::Strictest
        } else {
            Cue::Loosest
        };
        self.show_cue(cue, Vec::new(), cx);
    }

    fn clipboard_shortcut(key: &str, secondary: bool) -> Option<ClipboardShortcut> {
        if !secondary {
            return None;
        }
        match key {
            "c" => Some(ClipboardShortcut::Copy),
            "x" => Some(ClipboardShortcut::Cut),
            "v" => Some(ClipboardShortcut::Paste),
            _ => None,
        }
    }

    fn apply_clipboard_shortcut(&mut self, shortcut: ClipboardShortcut, cx: &mut Context<Self>) {
        match shortcut {
            ClipboardShortcut::Copy => self.copy_selection(cx),
            ClipboardShortcut::Cut => self.cut_selection(cx),
            ClipboardShortcut::Paste => self.paste_from_clipboard(cx),
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.query.selection_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.query.cut_selection() else {
            return;
        };
        self.state.clear_launch_error();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.state.reset_results_position();
        cx.notify();
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        match self.state.apply_paste(&text) {
            InputEffect::QueryChanged | InputEffect::FlowQueryChanged => {
                self.dispatch_query_change(cx)
            }
            _ => {}
        }
    }

    pub(super) fn dispatch_query_change(&mut self, cx: &mut Context<Self>) {
        self.state.clear_launch_error();
        self.state.reset_results_position();
        if self.state.flow.is_some() {
            self.schedule_flow_query(cx);
            cx.notify();
        } else {
            self.schedule_query_render(cx);
        }
    }

    pub(super) fn step_selected_rank(&mut self, up: bool, cx: &mut Context<Self>) {
        let selected = self.state.scroll_list.selected;
        let Some(scored) = self.store.get(selected) else {
            return;
        };
        let name = self.store.name(scored).to_owned();
        let cue = if !matches!(scored.source, ResultSource::App) {
            Cue::NoRank { name }
        } else if up && selected == 0 {
            Cue::AlreadyTop { name }
        } else if !up && self.store.boost(&name) == 0 {
            Cue::NotRaised { name }
        } else {
            let before = self.last_layout.clone();
            let Some(place) = self.store.step_selected(
                selected,
                up,
                self.state.query.text(),
                self.state.mode,
                self.state.fuzziness,
            ) else {
                return;
            };
            self.state.boost_adjusting = true;
            self.state.scroll_list.selected = place;
            self.state.sync_result_window(self.store.result_count());
            self.show_cue(Cue::Moved { name, up, place }, before, cx);
            return;
        };
        self.show_cue(cue, Vec::new(), cx);
    }

    pub(super) fn launch_selected(&mut self, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        let started = std::time::Instant::now();
        #[cfg(not(debug_assertions))]
        let started = ();
        trace::launch(self, "start", started);
        self.store.ensure_filtered(
            self.state.query.text(),
            self.state.mode,
            self.state.fuzziness,
        );
        let Some(scored) = self.store.get(self.state.scroll_list.selected) else {
            eprintln!(
                "[controller] launch_selected: no scored item at index {}",
                self.state.scroll_list.selected
            );
            return;
        };
        eprintln!(
            "[controller] launch_selected: index={} source={:?} name={:?}",
            self.state.scroll_list.selected,
            scored.source,
            self.store.name(scored)
        );
        let Some(item) = self.store.item(scored) else {
            eprintln!("[controller] launch_selected: failed to resolve item");
            return;
        };
        if let crate::discovery::search::ResultItem::Flow(entry) = item {
            self.state.enter_flow(entry.clone());
            trace::flow(self, "entered");
            cx.notify();
            return;
        }
        let is_app = matches!(scored.source, crate::discovery::search::ResultSource::App);
        let name = self.store.name(scored).to_string();
        eprintln!("[controller] launching item...");
        trace::launch(self, "send", started);
        let launch_result = crate::launch::launch_item(&item);
        trace::launch(self, "sent", started);
        if let Err(error) = launch_result {
            eprintln!("[controller] launch error: {error}");
            self.state.set_launch_error(error.to_string());
            cx.notify();
            return;
        }
        eprintln!("[controller] launch succeeded, closing after the cue");
        self.yield_focus();
        if is_app {
            self.store.record_launch(&name);
        }
        self.show_cue(Cue::Opening { name }, Vec::new(), cx);
    }

    pub(super) fn open_selected_folder(&mut self, cx: &mut Context<Self>) {
        self.store.ensure_filtered(
            self.state.query.text(),
            self.state.mode,
            self.state.fuzziness,
        );
        let path = self
            .store
            .get(self.state.scroll_list.selected)
            .and_then(|scored| self.store.item(scored))
            .and_then(reveal_target);
        let Some((path, source)) = path else {
            return;
        };
        self.reveal(&path, source, cx);
    }

    pub(super) fn reveal_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.reveal(path, "app_binary", cx);
    }

    fn reveal(&mut self, path: &Path, source: &'static str, cx: &mut Context<Self>) {
        if !path.exists() {
            trace::open_folder(source, "missing", "");
            self.state
                .set_launch_error("Selected item no longer exists".to_owned());
            cx.notify();
            return;
        }
        match qol_apps::desktop_integration::reveal_in_file_manager(path) {
            Ok(()) => {
                trace::open_folder(source, "ok", "");
                self.yield_focus();
                let home = std::env::var_os("HOME").map(PathBuf::from);
                self.show_cue(
                    Cue::OpeningFolder {
                        folder: folder_label(path, home.as_deref()),
                    },
                    Vec::new(),
                    cx,
                );
            }
            Err(error) => {
                trace::open_folder(source, "error", &error.to_string());
                self.state
                    .set_launch_error(format!("Could not open containing folder: {error}"));
                cx.notify();
            }
        }
    }

    fn yield_focus(&self) {
        qol_gpui::popup_window::release_input(&self.window_title);
    }

    pub(super) fn open_website(
        &mut self,
        url: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        match qol_apps::desktop_integration::open_with_default_app(url) {
            Ok(()) => self.hide_to_ghost("website", window),
            Err(error) => {
                self.state
                    .set_launch_error(format!("Could not open {url}: {error}"));
                cx.notify();
            }
        }
    }

    fn schedule_flow_query(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        flow.generation += 1;
        flow.pending = true;
        flow.verification_deadline = None;
        let epoch = flow.epoch;
        let generation = flow.generation;
        if self.state.query.text().trim().is_empty() {
            flow.rows.clear();
            flow.verdict = crate::flow::FlowVerdict::Answered;
            flow.pending = false;
            cx.notify();
            return;
        }
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(qol_gpui::theme::SETTLE_INPUT)
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let current = view.state.flow.as_ref().is_some_and(|session| {
                        session.matches_request(epoch, generation) && !session.in_flight
                    });
                    if current && view.is_showing {
                        view.start_flow_fetch(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn start_flow_fetch(&mut self, cx: &mut Context<Self>) {
        if !self.is_showing {
            return;
        }
        let Some(flow) = self.state.flow.as_mut() else {
            return;
        };
        let text = self.state.query.text().to_owned();
        if text.trim().is_empty() {
            return;
        }
        let generation = flow.generation;
        let epoch = flow.epoch;
        let entry = flow.entry.clone();
        flow.in_flight = true;
        trace::flow(self, "queried");
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let outcome = async_cx
                    .background_spawn(async move { crate::flow::fetch_rows(&entry, &text) })
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let Some(session) = view.state.flow.as_mut() else {
                        return;
                    };
                    if session.epoch != epoch || !view.is_showing {
                        return;
                    }
                    session.in_flight = false;
                    if session.generation != generation {
                        view.start_flow_fetch(cx);
                        return;
                    }
                    let (rows, mut verdict, failure) = match outcome {
                        Ok(fetch) => (fetch.rows, fetch.verdict, None),
                        Err(message) => (
                            Vec::new(),
                            crate::flow::FlowVerdict::Answered,
                            Some(message),
                        ),
                    };
                    if verdict == crate::flow::FlowVerdict::Checking {
                        let deadline = session.verification_deadline.get_or_insert_with(|| {
                            std::time::Instant::now() + std::time::Duration::from_secs(60)
                        });
                        if std::time::Instant::now() >= *deadline {
                            verdict = crate::flow::FlowVerdict::Vague;
                        }
                    }
                    session.rows = rows;
                    session.verdict = verdict;
                    session.pending = false;
                    if let Some(message) = failure {
                        view.state.set_launch_error(message);
                    }
                    trace::flow(view, "rows");
                    cx.notify();
                    if verdict == crate::flow::FlowVerdict::Checking {
                        view.refresh_pending_flow(epoch, generation, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn refresh_pending_flow(&mut self, epoch: u64, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                this.update(&mut async_cx, |view, cx| {
                    let current = view.state.flow.as_ref().is_some_and(|session| {
                        session.matches_request(epoch, generation)
                            && !session.in_flight
                            && session.verdict == crate::flow::FlowVerdict::Checking
                    });
                    if current && view.is_showing {
                        view.start_flow_fetch(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn dislike_flow_row(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        if flow.entry.plugin_id != "qol-memory" {
            return;
        }
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let Some(key) = row.raw.get("key").and_then(|value| value.as_str()) else {
            return;
        };
        let query = self.state.query.text().trim().to_string();
        if query.is_empty() {
            return;
        }
        let entry = flow.entry.clone();
        let key = key.to_string();
        trace::flow(self, "dislike");
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                let outcome = async_cx
                    .background_spawn(
                        async move { crate::flow::send_feedback(&entry, &query, &key) },
                    )
                    .await;
                if let Err(error) = outcome {
                    eprintln!("[controller] flow dislike failed: {error}");
                }
                this.update(&mut async_cx, |view, _| trace::flow(view, "disliked"))
                    .ok();
            }
        })
        .detach();
    }

    fn activate_flow_row(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(flow) = self.state.flow.as_ref() else {
            return;
        };
        let Some(row) = flow.rows.get(self.state.scroll_list.selected) else {
            return;
        };
        let entry = &flow.entry;
        if !entry.row_actions.is_empty() {
            if let Err(message) = crate::flow::run_row_action(entry, &entry.row_actions[0], row) {
                self.state.set_launch_error(message);
                cx.notify();
                return;
            }
        } else {
            let text = row.copy.clone().unwrap_or_else(|| row.title.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        trace::flow(self, "activated");
        self.hide_to_ghost("flow", window);
    }

    fn open_flow_detail(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.state.open_flow_detail() {
            return;
        }
        self.detail_scroll.set_offset(gpui::Point::default());
        window.resize(size(px(WINDOW_WIDTH), px(window_height_for_detail())));
        trace::flow(self, "detail_open");
        cx.notify();
    }

    fn scroll_flow_detail(&mut self, delta: f32, cx: &mut Context<Self>) {
        let max = self.detail_scroll.max_offset().height;
        let y = (self.detail_scroll.offset().y - px(delta))
            .max(-max)
            .min(px(0.0));
        self.detail_scroll.set_offset(gpui::point(px(0.0), y));
        cx.notify();
    }

    fn close_flow_detail(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.state.close_flow_detail() {
            return;
        }
        window.resize(size(px(WINDOW_WIDTH), px(full_window_height())));
        trace::flow(self, "detail_close");
        cx.notify();
    }
}

fn dismiss_reason(cue: &Cue) -> &'static str {
    match cue {
        Cue::Opening { .. } => "launch",
        Cue::OpeningFolder { .. } => "open_folder",
        _ => "copy",
    }
}

fn folder_label(path: &Path, home: Option<&Path>) -> String {
    let dir = path.parent().unwrap_or(path);
    match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

fn reveal_target(item: ResultItem<'_>) -> Option<(PathBuf, &'static str)> {
    match item {
        ResultItem::App(entry) => Some((entry.path.clone(), "app")),
        ResultItem::File(entry) => Some((entry.path.clone(), "file")),
        ResultItem::Flow(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{folder_label, reveal_target};
    use crate::discovery::search::ResultItem;
    use crate::discovery::FileEntry;
    use qol_apps::AppEntry;
    use std::path::PathBuf;

    #[test]
    fn folder_labels_name_the_containing_folder() {
        let home = PathBuf::from("/home/qol");
        assert_eq!(
            folder_label(&home.join("Documents/notes.md"), Some(&home)),
            "~/Documents"
        );
        assert_eq!(folder_label(&home.join("todo.txt"), Some(&home)), "~");
        assert_eq!(
            folder_label(std::path::Path::new("/usr/bin/xed"), Some(&home)),
            "/usr/bin"
        );
    }

    #[test]
    fn folder_action_reveals_both_app_entries_and_files() {
        let app_path = PathBuf::from("/usr/share/applications/sound.desktop");
        let file_path = PathBuf::from("/home/qol/Documents/sound.txt");
        let app = AppEntry {
            name: "Sound".to_owned(),
            exec: Vec::new(),
            path: app_path.clone(),
        };
        let file = FileEntry {
            name: "sound.txt".to_owned(),
            path: file_path.clone(),
        };
        assert_eq!(
            reveal_target(ResultItem::App(&app)),
            Some((app_path, "app"))
        );
        assert_eq!(
            reveal_target(ResultItem::File(&file)),
            Some((file_path, "file"))
        );
    }
}
