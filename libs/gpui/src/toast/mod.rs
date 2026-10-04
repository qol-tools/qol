use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;

use crate::kit::Kit;
use crate::monitor::MonitorTracker;
use crate::placement::{
    anchor_placement, Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN,
};
use crate::popup_window::{present_topmost, restore_composite, HiddenWindowsBarrier};
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};

mod card;
mod pile;

const COMPACT_WIDTH: f32 = 340.0;
const COMPACT_HEIGHT: f32 = 76.0;
const STATUS_WIDTH: f32 = 520.0;
const STATUS_HEIGHT: f32 = 78.0;

const PREVIEW_WIDTH: f32 = 72.0;
const DISMISS_WIDTH: f32 = 44.0;
const MAX_ROWS_PER_GROUP: usize = 3;
const HOVER_HOLD_RECHECK: Duration = Duration::from_millis(400);
const POINTER_POLL: Duration = Duration::from_millis(60);
const RING_TICK: Duration = Duration::from_millis(50);
const AGE_TICK: Duration = Duration::from_secs(30);

static TOAST_HOST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

type Activation = Rc<dyn Fn(&mut App) -> anyhow::Result<()>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastStyle {
    Compact,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastLayout {
    placement: MonitorPlacement,
    size: Size<Pixels>,
    style: ToastStyle,
}

impl ToastLayout {
    pub fn status() -> Self {
        Self {
            placement: MonitorPlacement::top_center(TOP_CENTER_MARGIN),
            size: size(px(STATUS_WIDTH), px(STATUS_HEIGHT)),
            style: ToastStyle::Status,
        }
    }

    pub fn compact() -> Self {
        Self {
            placement: MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
            size: size(px(COMPACT_WIDTH), px(COMPACT_HEIGHT)),
            style: ToastStyle::Compact,
        }
    }

    pub fn at(mut self, placement: MonitorPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn sized(mut self, size: Size<Pixels>) -> Self {
        self.size = size;
        self
    }

    pub fn for_push(
        anchor: Option<&str>,
        width: Option<f32>,
        height: Option<f32>,
        style: Option<&str>,
    ) -> Self {
        let base = match style {
            Some("compact") => ToastLayout::compact(),
            _ => ToastLayout::status(),
        };
        let Some(placement) = anchor.and_then(anchor_placement) else {
            return base;
        };
        let mut layout = base.at(placement);
        if let (Some(width), Some(height)) = (width, height) {
            layout = layout.sized(size(px(width), px(height)));
        }
        layout
    }

    pub fn placement(self) -> MonitorPlacement {
        self.placement
    }

    pub fn size(self) -> Size<Pixels> {
        self.size
    }

    pub fn style(self) -> ToastStyle {
        self.style
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

impl ToastTone {
    fn default_timeout(self) -> Option<Duration> {
        match self {
            Self::Neutral | Self::Info | Self::Success => Some(qol_theme::STAY_BRIEF),
            Self::Warning => Some(qol_theme::STAY_LONG),
            Self::Danger => qol_theme::STAY_UNTIL_CLOSED,
        }
    }

    fn notice(self) -> crate::kit::NoticeTone {
        match self {
            Self::Neutral | Self::Info => crate::kit::NoticeTone::Quiet,
            Self::Success => crate::kit::NoticeTone::Done,
            Self::Warning => crate::kit::NoticeTone::Attention,
            Self::Danger => crate::kit::NoticeTone::Invalid,
        }
    }

    fn color(self, kit: Kit) -> u32 {
        match self {
            Self::Neutral => kit.palette.border_subtle,
            Self::Info => kit.palette.info,
            Self::Success => kit.palette.success,
            Self::Warning => kit.palette.warning,
            Self::Danger => kit.palette.danger,
        }
    }
}

fn row_ground(row: &SlabSnapshotRow, kit: Kit) -> u32 {
    if row.toast.live {
        kit.grounds.menu.bg
    } else {
        tone_ground(row.toast.tone, kit)
    }
}

fn row_lift(row: &SlabSnapshotRow, kit: Kit) -> Rgba {
    rgb(qol_theme::lift(row_ground(row, kit), kit.grounds.pane.ink))
}

fn tone_ground(tone: ToastTone, kit: Kit) -> u32 {
    if tone == ToastTone::Danger {
        crate::kit::kit().grounds.invalid.bg
    } else {
        kit.grounds.pane.bg
    }
}

#[derive(Clone)]
pub struct Toast {
    title: SharedString,
    message: SharedString,
    tone: ToastTone,
    layout: ToastLayout,
    timeout: Option<Duration>,
    activation: Option<Activation>,
    message_is_path: bool,
    timeout_explicit: bool,
    group: SharedString,
    source: SharedString,
    key: Option<SharedString>,
    preview: Option<Rc<dyn crate::artifact::ArtifactPreview>>,
    preview_action: Option<Activation>,
    live: bool,
}

impl Toast {
    pub fn new(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        layout: ToastLayout,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            tone: ToastTone::Neutral,
            layout,
            timeout: None,
            activation: None,
            message_is_path: false,
            timeout_explicit: false,
            group: "".into(),
            source: "".into(),
            key: None,
            preview: None,
            preview_action: None,
            live: false,
        }
    }

    pub fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self.timeout_explicit = true;
        self
    }

    pub fn persistent(mut self) -> Self {
        self.timeout = None;
        self.timeout_explicit = true;
        self
    }

    pub fn on_activate(
        mut self,
        activation: impl Fn(&mut App) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.activation = Some(Rc::new(activation));
        self
    }

    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = group.into();
        self
    }

    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn detail_path(mut self, path: impl Into<SharedString>) -> Self {
        self.message = path.into();
        self.message_is_path = true;
        self
    }

    pub fn live(mut self) -> Self {
        self.live = true;
        self
    }

    pub fn source(mut self, source: impl Into<SharedString>) -> Self {
        self.source = source.into();
        self
    }

    pub fn on_preview(
        mut self,
        activation: impl Fn(&mut App) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.preview_action = Some(Rc::new(activation));
        self
    }

    pub fn artifact(self, path: impl Into<std::path::PathBuf>) -> Self {
        let path: Arc<std::path::Path> = path.into().into();
        let open = path.clone();
        let reveal = path.clone();
        let mut toast = self.detail_path(path.to_string_lossy().into_owned());
        toast.preview = Some(crate::artifact::preview_for(&path));
        toast
            .on_preview(move |_| crate::artifact::open_artifact(&open))
            .on_activate(move |_| crate::artifact::reveal_artifact(&reveal))
    }

    pub fn element(&self) -> Div {
        toast_notice(self)
    }

    pub fn positioned(&self, bounds: Bounds<Pixels>) -> Div {
        div()
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(self.element())
    }

    fn effective_timeout(&self) -> Option<Duration> {
        if self.timeout_explicit {
            self.timeout
        } else {
            self.tone.default_timeout()
        }
    }

    fn open(
        self,
        tracker: &MonitorTracker,
        title: &str,
        cx: &mut App,
    ) -> anyhow::Result<OpenedSurface<BannerToastView>> {
        Surface::new(SurfaceKind::Toast)
            .title(title)
            .placement(self.layout.placement())
            .size(self.layout.size())
            .open(tracker, cx, move |dismisser, _window, _cx| {
                BannerToastView {
                    toast: self,
                    dismisser,
                }
            })
    }
}

struct ActiveToast {
    surface: OpenedSurface<BannerToastView>,
    layout: ToastLayout,
}

#[derive(Clone)]
pub struct BannerPresenter {
    tracker: MonitorTracker,
    active: Rc<RefCell<Option<ActiveToast>>>,
    generation: Rc<Cell<u64>>,
    title: Rc<str>,
}

impl BannerPresenter {
    pub fn new(tracker: MonitorTracker, title: impl Into<Rc<str>>) -> Self {
        Self {
            tracker,
            active: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
            title: title.into(),
        }
    }

    pub fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let timeout = toast.effective_timeout();
        if !self.update_active(toast.clone(), cx) {
            self.close_active(cx);
            let layout = toast.layout;
            let surface = toast.open(&self.tracker, &self.title, cx)?;
            self.active
                .borrow_mut()
                .replace(ActiveToast { surface, layout });
        }
        let generation = self.next_generation();
        if let Some(timeout) = timeout {
            self.dismiss_after(generation, timeout, cx);
        }
        Ok(())
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.next_generation();
        self.close_active(cx);
    }

    fn update_active(&self, toast: Toast, cx: &mut App) -> bool {
        let mut slot = self.active.borrow_mut();
        let Some(active) = slot.as_mut() else {
            return false;
        };
        if active.layout != toast.layout {
            return false;
        }
        let updated = active
            .surface
            .handle
            .update(cx, |root, _, cx| {
                root.inner.update(cx, |view, cx| {
                    view.toast = toast;
                    cx.notify();
                });
            })
            .is_ok();
        if !updated {
            *slot = None;
        }
        updated
    }

    fn close_active(&self, cx: &mut App) {
        let Some(active) = self.active.borrow_mut().take() else {
            return;
        };
        active.surface.dismisser.dismiss(cx);
    }

    fn next_generation(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    fn dismiss_after(&self, generation: u64, timeout: Duration, cx: &mut App) {
        let presenter = self.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            if presenter.generation.get() != generation {
                return;
            }
            let _ = cx.update(|cx| presenter.dismiss(cx));
        })
        .detach();
    }
}

struct BannerToastView {
    toast: Toast,
    dismisser: SurfaceDismisser,
}

impl Render for BannerToastView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = self.toast.element();
        let Some(activation) = self.toast.activation.clone() else {
            return root;
        };
        let dismisser = self.dismisser.clone();
        root = root.cursor_pointer().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _event, _window, cx| {
                if let Err(error) = activation(cx) {
                    qol_runtime::probe!("TOAST_ACTIVATION", "presentation=banner error={error:#}");
                }
                dismisser.dismiss(cx);
            }),
        );
        root
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RowId(u64);

struct ToastRow {
    id: RowId,
    toast: Toast,
    generation: u64,
    created: Instant,
    deadline: Option<Instant>,
}

struct SlabSnapshotRow {
    id: RowId,
    toast: Toast,
    created: Instant,
    deadline: Option<Instant>,
}

struct HostState {
    surface: Option<OpenedSurface<SlabToastView>>,
    rows: Vec<ToastRow>,
    next_id: u64,
    next_generation: u64,
    expanded: bool,
}

impl HostState {
    fn next_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }
}

#[derive(Clone)]
pub struct SlabPresenter {
    tracker: MonitorTracker,
    state: Rc<RefCell<HostState>>,
    title: Rc<str>,
}

enum PushOutcome {
    Open {
        placement: MonitorPlacement,
        size: Size<Pixels>,
    },
    Notify,
}

impl SlabPresenter {
    pub fn new(tracker: MonitorTracker, title: impl Into<Rc<str>>) -> Self {
        Self {
            tracker,
            state: Rc::new(RefCell::new(HostState {
                surface: None,
                rows: Vec::new(),
                next_id: 0,
                next_generation: 0,
                expanded: false,
            })),
            title: title.into(),
        }
    }

    pub fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let timeout = toast.effective_timeout();
        let (outcome, row_id, generation) = {
            let mut state = self.state.borrow_mut();
            let placement = toast.layout.placement();
            let group = toast.group.clone();
            let key = toast.key.clone();

            let target = key.as_ref().and_then(|key| {
                state
                    .rows
                    .iter()
                    .position(|row| row.toast.group == group && row.toast.key.as_ref() == Some(key))
            });

            let row_id;
            let generation;
            let created = Instant::now();
            let deadline = timeout.map(|timeout| created + timeout);
            match target {
                Some(index) => {
                    generation = state.next_generation();
                    let row = &mut state.rows[index];
                    row.generation = generation;
                    row.toast = toast;
                    row.created = created;
                    row.deadline = deadline;
                    row_id = row.id;
                }
                None => {
                    row_id = RowId(state.next_id);
                    state.next_id += 1;
                    generation = state.next_generation();
                    state.rows.push(ToastRow {
                        id: row_id,
                        generation,
                        toast,
                        created,
                        deadline,
                    });
                }
            }

            let positions: Vec<usize> = state
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| !row.toast.live && row.toast.group == group)
                .map(|(index, _)| index)
                .collect();
            if positions.len() > MAX_ROWS_PER_GROUP {
                let stale_ids: Vec<RowId> = positions[..positions.len() - MAX_ROWS_PER_GROUP]
                    .iter()
                    .map(|&index| state.rows[index].id)
                    .collect();
                state.rows.retain(|row| !stale_ids.contains(&row.id));
            }

            let resting = pile::layout(state.rows.len(), 0.0, 0.0, &[]);
            let size = size(px(resting.width.ceil()), px(resting.height.ceil()));

            let outcome = if state.surface.is_none() {
                PushOutcome::Open { placement, size }
            } else {
                PushOutcome::Notify
            };
            (outcome, row_id, generation)
        };

        match outcome {
            PushOutcome::Open { placement, size } => {
                let owner: &str = &self.title;
                let host = self.clone();
                let surface = Surface::new(SurfaceKind::Toast)
                    .title(owner)
                    .placement(placement)
                    .size(size)
                    .open(&self.tracker, cx, move |dismisser, _window, _cx| {
                        SlabToastView::new(host, dismisser)
                    });
                match surface {
                    Ok(surface) => {
                        self.state.borrow_mut().surface.replace(surface);
                        present_topmost(&self.title);
                    }
                    Err(error) => {
                        let mut state = self.state.borrow_mut();
                        state.rows.clear();
                        state.expanded = false;
                        return Err(error);
                    }
                }
            }
            PushOutcome::Notify => self.notify_view(cx),
        }

        if let Some(timeout) = timeout {
            arm_timer(self.clone(), row_id, generation, timeout, cx);
        }
        Ok(())
    }

    pub fn dismiss(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.rows.clear();
        }
        self.close(cx);
    }

    pub fn clear_all(&self, cx: &mut App) {
        let ids: Vec<RowId> = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .filter(|row| !row.toast.live)
                .map(|row| row.id)
                .collect()
        };
        self.drop_rows(&ids, cx);
    }

    fn set_expanded(&self, expanded: bool, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            let expanded = expanded && state.rows.len() >= 2;
            if state.expanded == expanded {
                return;
            }
            state.expanded = expanded;
        }
        if !expanded {
            self.restart_timers(cx);
        }
        self.notify_view(cx);
    }

    fn restart_timers(&self, cx: &mut App) {
        let now = Instant::now();
        let armed: Vec<(RowId, u64, Duration)> = {
            let mut state = self.state.borrow_mut();
            let mut armed = Vec::new();
            for index in 0..state.rows.len() {
                let Some(timeout) = state.rows[index].toast.effective_timeout() else {
                    continue;
                };
                let generation = state.next_generation();
                let row = &mut state.rows[index];
                row.generation = generation;
                row.deadline = Some(now + timeout);
                armed.push((row.id, generation, timeout));
            }
            armed
        };
        for (id, generation, timeout) in armed {
            arm_timer(self.clone(), id, generation, timeout, cx);
        }
    }

    fn mark_row_failed(&self, id: RowId, error: anyhow::Error, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            let generation = state.next_generation();
            if let Some(row) = state.rows.iter_mut().find(|row| row.id == id) {
                row.generation = generation;
                let previous_title = row.toast.title.clone();
                row.toast.tone = ToastTone::Danger;
                row.toast.message = previous_title;
                row.toast.title = error.to_string().into();
                row.toast.timeout = None;
                row.toast.timeout_explicit = true;
                row.deadline = None;
            }
        }
        self.notify_view(cx);
    }

    fn activate(&self, id: RowId, cx: &mut App) {
        self.run(id, |toast| toast.activation.clone(), cx);
    }

    fn open_preview(&self, id: RowId, cx: &mut App) {
        self.run(id, |toast| toast.preview_action.clone(), cx);
    }

    fn run(&self, id: RowId, pick: impl Fn(&Toast) -> Option<Activation>, cx: &mut App) {
        let action = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| pick(&row.toast))
        };
        let Some(action) = action else {
            return;
        };
        match action(cx) {
            Ok(()) => self.remove(id, cx),
            Err(error) => self.mark_row_failed(id, error, cx),
        }
    }

    fn on_timer(&self, id: RowId, generation: u64, pointer_inside: bool, cx: &mut App) {
        let (owned, expanded) = {
            let state = self.state.borrow();
            let owned = state
                .rows
                .iter()
                .any(|row| row.id == id && row.generation == generation);
            (owned, state.expanded)
        };
        if !owned {
            return;
        }
        if pointer_inside || expanded {
            arm_timer(self.clone(), id, generation, HOVER_HOLD_RECHECK, cx);
        } else {
            self.remove(id, cx);
        }
    }

    fn remove(&self, id: RowId, cx: &mut App) {
        self.drop_rows(&[id], cx);
    }

    fn drop_rows(&self, ids: &[RowId], cx: &mut App) {
        let remains_empty = {
            let mut state = self.state.borrow_mut();
            state.rows.retain(|row| !ids.contains(&row.id));
            if state.rows.len() < 2 {
                state.expanded = false;
            }
            state.rows.is_empty()
        };
        if remains_empty {
            self.close(cx);
        } else {
            self.notify_view(cx);
        }
    }

    fn close(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.expanded = false;
        }
        if let Some(surface) = self.state.borrow_mut().surface.take() {
            surface.dismisser.dismiss(cx);
        }
        restore_composite(&self.title);
    }

    fn notify_view(&self, cx: &mut App) {
        if let Some(surface) = self.state.borrow().surface.as_ref() {
            let _ = surface.handle.update(cx, |root, _, cx| {
                root.inner.update(cx, |_view, cx| cx.notify());
            });
        }
    }

    fn slab_snapshot(&self) -> (Vec<SlabSnapshotRow>, bool) {
        let state = self.state.borrow();
        let rows = state
            .rows
            .iter()
            .map(|row| SlabSnapshotRow {
                id: row.id,
                toast: row.toast.clone(),
                created: row.created,
                deadline: row.deadline,
            })
            .collect();
        (rows, state.expanded)
    }

    fn anchored_origin(&self, content: Size<Pixels>) -> Option<Point<Pixels>> {
        self.state
            .borrow()
            .surface
            .as_ref()
            .map(|surface| surface.anchored_origin(content))
    }
}

enum Presentation {
    Banner,
    Slab,
}

fn routed_presentation(toast: &Toast) -> Presentation {
    match toast.layout.style() {
        ToastStyle::Status => Presentation::Banner,
        ToastStyle::Compact => Presentation::Slab,
    }
}

#[derive(Clone)]
pub struct ToastHost {
    banner: BannerPresenter,
    slab: SlabPresenter,
}

impl ToastHost {
    pub fn new(tracker: MonitorTracker) -> Self {
        let sequence = TOAST_HOST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let process_id = std::process::id();
        Self {
            banner: BannerPresenter::new(
                tracker.clone(),
                format!("qol-toast-banner-{process_id}-{sequence}"),
            ),
            slab: SlabPresenter::new(tracker, format!("qol-toast-slab-{process_id}-{sequence}")),
        }
    }

    pub fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        if toast.title.is_empty() {
            anyhow::bail!("toast push refused: no title");
        }
        match routed_presentation(&toast) {
            Presentation::Banner => self.banner.show(toast, cx),
            Presentation::Slab => self.slab.show(toast, cx),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.banner.dismiss(cx);
        self.slab.dismiss(cx);
    }

    pub fn clear_all(&self, cx: &mut App) {
        self.slab.clear_all(cx);
    }

    pub async fn wait_until_hidden(
        &self,
        cx: &mut AsyncApp,
    ) -> crate::popup_window::HiddenWindowsBarrier {
        let started = Instant::now();
        let banner = crate::popup_window::wait_for_hidden_windows(cx, &self.banner.title).await;
        let slab = crate::popup_window::wait_for_hidden_windows(cx, &self.slab.title).await;
        HiddenWindowsBarrier {
            cleared: banner.cleared && slab.cleared,
            visible: banner.visible + slab.visible,
            clear_samples: banner.clear_samples.min(slab.clear_samples),
            elapsed: started.elapsed(),
        }
    }
}

fn arm_timer(
    presenter: SlabPresenter,
    id: RowId,
    generation: u64,
    timeout: Duration,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(timeout).await;
        let title = presenter.title.to_string();
        let pointer_inside = cx
            .background_spawn(
                async move { crate::popup_window::pointer_over_window_by_title(&title) },
            )
            .await;
        let _ = cx.update(|cx| presenter.on_timer(id, generation, pointer_inside, cx));
    })
    .detach();
}

struct SlabToastView {
    host: SlabPresenter,
    dismisser: SurfaceDismisser,
    inside: bool,
    hovered: Option<RowId>,
    lit: bool,
    grow: pile::Tween,
    open: pile::Tween,
    focus: Vec<(RowId, pile::Tween)>,
    polling: bool,
    ring_ticking: bool,
    age_ticking: bool,
}

impl SlabToastView {
    fn new(host: SlabPresenter, dismisser: SurfaceDismisser) -> Self {
        Self {
            host,
            dismisser,
            inside: false,
            hovered: None,
            lit: false,
            grow: pile::Tween::at(0.0),
            open: pile::Tween::at(0.0),
            focus: Vec::new(),
            polling: false,
            ring_ticking: false,
            age_ticking: false,
        }
    }

    fn enter(&mut self, cx: &mut Context<Self>) {
        if self.inside {
            return;
        }
        self.inside = true;
        self.ensure_polling(cx);
        cx.notify();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if !self.inside {
            return;
        }
        self.inside = false;
        self.hovered = None;
        self.lit = false;
        self.host.restart_timers(cx);
        cx.notify();
    }

    fn ensure_polling(&mut self, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        let title = self.host.title.to_string();
        let host = self.host.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POINTER_POLL).await;
            let probe = title.clone();
            let pointer = cx
                .background_spawn(
                    async move { crate::popup_window::pointer_on_window_by_title(&probe) },
                )
                .await;
            let Ok(Some(fold)) = this.update(cx, |view, cx| view.on_pointer(pointer, cx)) else {
                break;
            };
            if fold {
                let _ = cx.update(|cx| host.set_expanded(false, cx));
            }
        })
        .detach();
    }

    fn on_pointer(
        &mut self,
        pointer: Option<crate::popup_window::PointerOnWindow>,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let Some(pointer) = pointer else {
            self.polling = false;
            return None;
        };
        if !pointer.inside {
            self.leave(cx);
        }
        let expanded = self.host.state.borrow().expanded;
        if !self.inside && !expanded {
            self.polling = false;
            return None;
        }
        Some(expanded && pointer.pressed && !pointer.inside)
    }

    fn tick(&mut self, ring: bool, cx: &mut Context<Self>) {
        let pending = if ring {
            &mut self.ring_ticking
        } else {
            &mut self.age_ticking
        };
        if *pending {
            return;
        }
        *pending = true;
        let interval = if ring { RING_TICK } else { AGE_TICK };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(interval).await;
            let _ = this.update(cx, |view, cx| {
                if ring {
                    view.ring_ticking = false;
                } else {
                    view.age_ticking = false;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn settle_focus(&mut self, rows: &[&SlabSnapshotRow], open: bool, now: Instant) {
        self.focus
            .retain(|(id, _)| rows.iter().any(|row| row.id == *id));
        for row in rows {
            let target = if open && self.hovered == Some(row.id) {
                1.0
            } else {
                0.0
            };
            match self.focus.iter_mut().find(|(id, _)| *id == row.id) {
                Some((_, tween)) => tween.toward(target, now),
                None => {
                    let mut tween = pile::Tween::at(0.0);
                    tween.toward(target, now);
                    self.focus.push((row.id, tween));
                }
            }
        }
    }

    fn focus_of(&self, id: RowId) -> Option<&pile::Tween> {
        self.focus
            .iter()
            .find(|(row, _)| *row == id)
            .map(|(_, tween)| tween)
    }
}

impl Render for SlabToastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = crate::kit::kit();
        let now = Instant::now();
        let (snapshot, expanded) = self.host.slab_snapshot();
        let rows: Vec<&SlabSnapshotRow> = snapshot.iter().rev().collect();
        let count = rows.len();
        let open = expanded && count >= 2;

        self.grow.toward(if self.inside { 1.0 } else { 0.0 }, now);
        self.open.toward(if open { 1.0 } else { 0.0 }, now);
        self.settle_focus(&rows, open, now);
        let focus_now: Vec<f32> = rows
            .iter()
            .map(|row| self.focus_of(row.id).map_or(0.0, |tween| tween.value(now)))
            .collect();
        let focus_end: Vec<f32> = rows
            .iter()
            .map(|row| self.focus_of(row.id).map_or(0.0, pile::Tween::target))
            .collect();
        let current = pile::layout(
            count,
            self.grow.value(now),
            self.open.value(now),
            &focus_now,
        );
        let settled = pile::layout(count, self.grow.target(), self.open.target(), &focus_end);
        let moving = self.grow.moving(now)
            || self.open.moving(now)
            || self.focus.iter().any(|(_, tween)| tween.moving(now));

        let shown = window.bounds().size;
        let target = if moving {
            size(
                px(current
                    .width
                    .max(settled.width)
                    .max(f32::from(shown.width))
                    .ceil()),
                px(current
                    .height
                    .max(settled.height)
                    .max(f32::from(shown.height))
                    .ceil()),
            )
        } else {
            size(px(settled.width.ceil()), px(settled.height.ceil()))
        };
        if shown != target && self.dismisser.resize_window(target, window) {
            if let Some(origin) = self.host.anchored_origin(target) {
                self.dismisser.reposition_window(origin);
            }
        }
        if moving {
            window.request_animation_frame();
        }
        let frame = window.bounds().size;
        let dx = f32::from(frame.width) - current.width;
        let dy = f32::from(frame.height) - current.height;
        let place = |element: Stateful<Div>, at: pile::Frame| {
            element
                .absolute()
                .left(px(at.left + dx))
                .top(px(at.top + dy))
                .w(px(at.width))
                .h(px(at.height))
        };

        let mut ring_running = false;
        let card_view = |index: usize, view: &Self, ring_running: &mut bool| -> AnyElement {
            let row = rows[index];
            let card = current.cards[index];
            let ring = row.toast.effective_timeout().map(|timeout| {
                if view.inside || open {
                    card::Ring {
                        remaining: 1.0,
                        ink: kit.grounds.pane.faint,
                    }
                } else {
                    *ring_running = true;
                    let left = row.deadline.map_or(Duration::ZERO, |deadline| {
                        deadline.saturating_duration_since(now)
                    });
                    card::Ring {
                        remaining: left.as_secs_f32() / timeout.as_secs_f32(),
                        ink: row.toast.tone.color(kit),
                    }
                }
            });
            let ground = if index > 0 && view.lit && !open {
                row_lift(row, kit)
            } else {
                rgb(row_ground(row, kit))
            };
            let id = row.id;
            let host = view.host.clone();
            let mut element = place(div().id(card::card_id(id)), card.frame)
                .opacity(card.opacity)
                .occlude()
                .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                    if *hovered {
                        view.hovered = Some(id);
                        view.enter(cx);
                    } else if view.hovered == Some(id) {
                        view.hovered = None;
                    }
                    cx.notify();
                }));
            if index > 0 && !open {
                let opener = host.clone();
                element = element
                    .cursor_pointer()
                    .on_click(move |_, _, cx| opener.set_expanded(true, cx));
            }
            element
                .child(kit.window().bg(ground).child(card::content(
                    row,
                    card::CardParts {
                        scale: card.scale,
                        content: card.content,
                        interactive: index == 0 || open,
                        ring,
                        age: pile::age_label(now.saturating_duration_since(row.created)),
                    },
                    kit,
                    host,
                )))
                .into_any_element()
        };

        let mut layers: Vec<AnyElement> = Vec::new();
        if open {
            let host = self.host.clone();
            layers.push(
                div()
                    .id("toast-fold")
                    .absolute()
                    .size_full()
                    .on_click(move |_, _, cx| host.set_expanded(false, cx))
                    .into_any_element(),
            );
        }
        let focused = rows
            .iter()
            .position(|row| open && self.hovered == Some(row.id));
        for index in (1..count).rev() {
            if focused != Some(index) && current.cards[index].opacity > 0.01 {
                layers.push(card_view(index, self, &mut ring_running));
            }
        }
        if let Some(strip) = current.strip {
            layers.push(
                place(div().id("toast-strip"), strip.frame)
                    .occlude()
                    .child(kit.window().shadow(Vec::new()).child(card::strip(
                        count,
                        strip.scale,
                        kit,
                        self.host.clone(),
                    )))
                    .into_any_element(),
            );
        }
        if count > 0 {
            layers.push(card_view(0, self, &mut ring_running));
        }
        if let Some(index) = focused.filter(|index| *index > 0) {
            layers.push(card_view(index, self, &mut ring_running));
        }
        if let Some(band) = current.band.filter(|_| current.words > 0.01) {
            let host = self.host.clone();
            layers.push(
                place(div().id("toast-show-all"), band)
                    .occlude()
                    .cursor_pointer()
                    .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                        view.lit = *hovered;
                        if *hovered {
                            view.enter(cx);
                        }
                        cx.notify();
                    }))
                    .on_click(move |_, _, cx| host.set_expanded(true, cx))
                    .child(card::show_all(current.words, kit))
                    .into_any_element(),
            );
        }

        if ring_running {
            self.tick(true, cx);
        }
        if count > 0 {
            self.tick(false, cx);
        }

        div()
            .id("toast-pile")
            .size_full()
            .relative()
            .on_mouse_move(cx.listener(|view, _, _, cx| view.enter(cx)))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if *hovered {
                    view.enter(cx);
                } else {
                    view.leave(cx);
                }
            }))
            .children(layers)
    }
}

fn toast_notice(toast: &Toast) -> Div {
    let kit = crate::kit::kit();
    let detail = (!toast.message.is_empty()).then(|| toast.message.clone().into_any_element());
    kit.notice(toast.tone.notice(), toast.title.clone(), detail)
        .size_full()
        .overflow_hidden()
        .shadow(crate::kit::float_shadow(kit.palette.text_primary))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN};

    use super::{routed_presentation, Presentation, Toast, ToastLayout, ToastStyle, ToastTone};

    #[test]
    fn a_caller_overrides_the_preset_placement_and_size() {
        let corner = MonitorPlacement::corner(Corner::TopLeft, CORNER_MARGIN);
        let layout = ToastLayout::status()
            .at(corner)
            .sized(gpui::size(gpui::px(700.0), gpui::px(120.0)));
        assert_eq!(layout.placement(), corner);
        assert_eq!(layout.size().width.to_f64(), 700.0);
        assert_eq!(layout.size().height.to_f64(), 120.0);
        assert_eq!(ToastLayout::status().size().width.to_f64(), 520.0);
    }

    #[test]
    fn for_push_without_layout_is_the_status_preset() {
        assert_eq!(
            ToastLayout::for_push(None, None, None, None),
            ToastLayout::status()
        );
    }

    #[test]
    fn for_push_with_unknown_anchor_falls_back_to_the_status_preset() {
        assert_eq!(
            ToastLayout::for_push(Some("corner"), Some(400.0), Some(84.0), None),
            ToastLayout::status()
        );
    }

    #[test]
    fn for_push_with_compact_style_is_the_compact_preset_whole() {
        assert_eq!(
            ToastLayout::for_push(None, None, None, Some("compact")),
            ToastLayout::compact()
        );
        assert_eq!(
            ToastLayout::compact().placement(),
            MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN)
        );
        assert_eq!(ToastLayout::compact().size().width.to_f64(), 340.0);
        assert_eq!(ToastLayout::compact().size().height.to_f64(), 76.0);
    }

    #[test]
    fn for_push_with_unknown_style_falls_back_to_the_status_preset() {
        assert_eq!(
            ToastLayout::for_push(None, None, None, Some("headline")),
            ToastLayout::status()
        );
    }

    #[test]
    fn for_push_places_and_sizes_at_a_corner() {
        let layout = ToastLayout::for_push(Some("bottom-right"), Some(400.0), Some(84.0), None);
        assert_eq!(
            layout.placement(),
            MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN)
        );
        assert_eq!(layout.size().width.to_f64(), 400.0);
        assert_eq!(layout.size().height.to_f64(), 84.0);
    }

    #[test]
    fn for_push_with_partial_size_keeps_the_preset_dimensions() {
        let layout = ToastLayout::for_push(Some("center"), Some(600.0), None, None);
        assert_eq!(layout.placement(), MonitorPlacement::center());
        assert_eq!(layout.size(), ToastLayout::status().size());
    }

    #[test]
    fn for_push_with_compact_style_still_applies_overrides() {
        let layout =
            ToastLayout::for_push(Some("top-left"), Some(420.0), Some(90.0), Some("compact"));
        assert_eq!(layout.style, ToastStyle::Compact);
        assert_eq!(
            layout.placement(),
            MonitorPlacement::corner(Corner::TopLeft, CORNER_MARGIN)
        );
        assert_eq!(layout.size().width.to_f64(), 420.0);
        assert_eq!(layout.size().height.to_f64(), 90.0);
    }

    #[test]
    fn layouts_select_shared_placement_and_dimensions() {
        let cases = [
            (
                ToastLayout::compact(),
                MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
                (340.0, 76.0),
            ),
            (
                ToastLayout::status(),
                MonitorPlacement::top_center(TOP_CENTER_MARGIN),
                (520.0, 78.0),
            ),
        ];

        for (layout, placement, dimensions) in cases {
            let size = layout.size();
            assert_eq!(layout.placement(), placement, "layout: {layout:?}");
            assert_eq!(
                (size.width.to_f64(), size.height.to_f64()),
                dimensions,
                "layout: {layout:?}"
            );
        }
    }

    #[test]
    fn tones_map_to_semantic_palette_roles() {
        let system = qol_theme::DARK_SYSTEM;
        let kit = crate::kit::Kit::new(qol_theme::ThemeMode::Dark, system);
        let cases = [
            (ToastTone::Neutral, system.border_subtle),
            (ToastTone::Info, system.info),
            (ToastTone::Success, system.success),
            (ToastTone::Warning, system.warning),
            (ToastTone::Danger, system.danger),
        ];

        for (tone, expected) in cases {
            assert_eq!(tone.color(kit), expected, "tone: {tone:?}");
        }
    }

    #[test]
    fn tones_set_the_specified_default_timeouts() {
        let cases = [
            (ToastTone::Neutral, Some(Duration::from_secs(4))),
            (ToastTone::Info, Some(Duration::from_secs(4))),
            (ToastTone::Success, Some(Duration::from_secs(4))),
            (ToastTone::Warning, Some(Duration::from_secs(8))),
            (ToastTone::Danger, None),
        ];
        for (tone, expected) in cases {
            assert_eq!(tone.default_timeout(), expected, "tone: {tone:?}");
        }
    }

    #[test]
    fn neutral_toasts_expire_from_tone_defaults_without_explicit_calls() {
        let toast = Toast::new("t", "m", ToastLayout::status());
        assert_eq!(toast.effective_timeout(), Some(Duration::from_secs(4)));
        let warning = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Warning);
        assert_eq!(warning.effective_timeout(), Some(Duration::from_secs(8)));
        let danger = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Danger);
        assert_eq!(danger.effective_timeout(), None);
    }

    #[test]
    fn explicit_timeout_beats_the_tone_default() {
        let toast = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Danger);
        assert_eq!(toast.effective_timeout(), None);
        let timed = toast.timeout(Duration::from_secs(2));
        assert_eq!(timed.effective_timeout(), Some(Duration::from_secs(2)));
        let long_warning = Toast::new("t", "m", ToastLayout::status())
            .tone(ToastTone::Warning)
            .timeout(Duration::from_secs(60));
        assert_eq!(
            long_warning.effective_timeout(),
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn persistent_calls_drop_any_timeout_including_later_ones() {
        let persistent_info = Toast::new("t", "m", ToastLayout::status()).persistent();
        assert_eq!(persistent_info.effective_timeout(), None);
        assert!(persistent_info.timeout_explicit);
        let reinstated = persistent_info.timeout(Duration::from_secs(9));
        assert_eq!(reinstated.effective_timeout(), Some(Duration::from_secs(9)));
    }

    #[test]
    fn toast_host_routes_status_layouts_to_the_banner_and_compact_to_the_slab() {
        assert_eq!(ToastLayout::status().style(), ToastStyle::Status);
        assert_eq!(ToastLayout::compact().style(), ToastStyle::Compact);
        assert!(matches!(
            routed_presentation(&Toast::new("t", "m", ToastLayout::status())),
            Presentation::Banner
        ));
        assert!(matches!(
            routed_presentation(&Toast::new("t", "m", ToastLayout::compact())),
            Presentation::Slab
        ));
    }

    #[test]
    fn a_long_message_stays_inside_the_slab_bounds() {
        use taffy::geometry::{Point, Size as TaffySize};
        use taffy::style::{Dimension, FlexDirection, Overflow, Style};
        use taffy::{AvailableSpace, TaffyTree};

        let mut tree: TaffyTree<()> = TaffyTree::new();
        let preview = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::length(super::PREVIEW_WIDTH),
                    height: Dimension::length(super::pile::CARD_HEIGHT),
                },
                ..Default::default()
            })
            .unwrap();
        let dismiss = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::length(super::DISMISS_WIDTH),
                    height: Dimension::length(super::pile::CARD_HEIGHT),
                },
                ..Default::default()
            })
            .unwrap();
        let head = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::length(480.0),
                    height: Dimension::length(16.0),
                },
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_size: TaffySize {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            })
            .unwrap();
        let tail = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::length(4096.0),
                    height: Dimension::length(16.0),
                },
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_size: TaffySize {
                    width: Dimension::length(0.0),
                    height: Dimension::auto(),
                },
                ..Default::default()
            })
            .unwrap();
        let path_line = tree
            .new_with_children(
                Style {
                    size: TaffySize {
                        width: Dimension::percent(1.0),
                        height: Dimension::auto(),
                    },
                    flex_direction: FlexDirection::Row,
                    overflow: Point {
                        x: Overflow::Hidden,
                        y: Overflow::Hidden,
                    },
                    ..Default::default()
                },
                &[head, tail],
            )
            .unwrap();
        let message = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::percent(1.0),
                    height: Dimension::length(16.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Default::default()
            })
            .unwrap();
        let text_column = tree
            .new_with_children(
                Style {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_size: TaffySize {
                        width: Dimension::length(0.0),
                        height: Dimension::auto(),
                    },
                    overflow: Point {
                        x: Overflow::Hidden,
                        y: Overflow::Hidden,
                    },
                    ..Default::default()
                },
                &[path_line, message],
            )
            .unwrap();
        let slab = tree
            .new_with_children(
                Style {
                    size: TaffySize {
                        width: Dimension::length(super::pile::CARD_WIDTH),
                        height: Dimension::length(super::pile::CARD_HEIGHT),
                    },
                    flex_direction: FlexDirection::Row,
                    overflow: Point {
                        x: Overflow::Hidden,
                        y: Overflow::Hidden,
                    },
                    ..Default::default()
                },
                &[preview, text_column, dismiss],
            )
            .unwrap();
        tree.compute_layout(
            slab,
            TaffySize {
                width: AvailableSpace::Definite(super::pile::CARD_WIDTH),
                height: AvailableSpace::Definite(super::pile::CARD_HEIGHT),
            },
        )
        .unwrap();

        let column = tree.layout(text_column).unwrap();
        let line = tree.layout(path_line).unwrap();
        let head = tree.layout(head).unwrap();
        let tail = tree.layout(tail).unwrap();
        let message = tree.layout(message).unwrap();
        assert!(
            column.location.x + column.size.width <= super::pile::CARD_WIDTH,
            "the text column stays inside the slab"
        );
        assert!(
            line.location.x + line.size.width <= column.size.width,
            "the path line is bounded by the text column"
        );
        assert!(
            head.location.x + head.size.width <= line.size.width,
            "the path head stays inside the path line"
        );
        assert!(
            tail.location.x + tail.size.width <= line.size.width,
            "the long path tail stays inside the path line"
        );
        assert!(
            tail.size.width < 4096.0,
            "the long path tail must shrink into the line"
        );
        assert!(
            message.location.x + message.size.width <= column.size.width,
            "the message line stays inside the text column"
        );
    }
}
