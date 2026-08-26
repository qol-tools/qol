use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::placement::{
    anchor_placement, Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN,
};
use crate::popup_window::{present_topmost, restore_composite, HiddenWindowsBarrier};
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};
use crate::theme::{toast_runtime, ToastPalette};

const COMPACT_WIDTH: f32 = 340.0;
const COMPACT_HEIGHT: f32 = 76.0;
const STATUS_WIDTH: f32 = 520.0;
const STATUS_HEIGHT: f32 = 78.0;

const SLAB_WIDTH: f32 = 440.0;
const ROW_HEIGHT: f32 = 68.0;
const HEADER_HEIGHT: f32 = 30.0;
const SUMMARY_HEIGHT: f32 = 36.0;
const LIVE_BAND_HEIGHT: f32 = 20.0;
const PREVIEW_WIDTH: f32 = 72.0;
const FOLDER_WIDTH: f32 = 64.0;
const DISMISS_WIDTH: f32 = 44.0;
const GUTTER: f32 = 8.0;
const TEXT_PAD: f32 = 16.0;
const PREVIEW_SLOT_WIDTH: u32 = 72;
const PREVIEW_SLOT_HEIGHT: u32 = 68;
const MAX_ROWS_PER_GROUP: usize = 3;
const MAX_VISIBLE_NON_LIVE_ROWS: usize = 4;

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
            Self::Neutral | Self::Info | Self::Success => Some(Duration::from_secs(4)),
            Self::Warning => Some(Duration::from_secs(8)),
            Self::Danger => None,
        }
    }

    fn color(self, palette: ToastPalette) -> u32 {
        match self {
            Self::Neutral => palette.border,
            Self::Info => palette.info,
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Danger => palette.danger,
        }
    }
}

fn tone_glyph(tone: ToastTone) -> &'static str {
    match tone {
        ToastTone::Neutral | ToastTone::Info => "\u{25CF}",
        ToastTone::Success => "\u{2713}",
        ToastTone::Warning => "!",
        ToastTone::Danger => "\u{2715}",
    }
}

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
    key: Option<SharedString>,
    preview: Option<(Arc<std::path::Path>, ObjectFit)>,
    folder: Option<Activation>,
    live: bool,
}

impl Clone for Toast {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            message: self.message.clone(),
            tone: self.tone,
            layout: self.layout,
            timeout: self.timeout,
            activation: self.activation.clone(),
            message_is_path: self.message_is_path,
            timeout_explicit: self.timeout_explicit,
            group: self.group.clone(),
            key: self.key.clone(),
            preview: self
                .preview
                .as_ref()
                .map(|(path, object_fit)| (path.clone(), duplicate_object_fit(object_fit))),
            folder: self.folder.clone(),
            live: self.live,
        }
    }
}

fn duplicate_object_fit(object_fit: &ObjectFit) -> ObjectFit {
    match object_fit {
        ObjectFit::Fill => ObjectFit::Fill,
        ObjectFit::Contain => ObjectFit::Contain,
        ObjectFit::Cover => ObjectFit::Cover,
        ObjectFit::ScaleDown => ObjectFit::ScaleDown,
        ObjectFit::None => ObjectFit::None,
    }
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
            key: None,
            preview: None,
            folder: None,
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

    pub fn preview(
        mut self,
        path: impl Into<Arc<std::path::Path>>,
        width: u32,
        height: u32,
    ) -> Self {
        let object_fit = if width < PREVIEW_SLOT_WIDTH || height < PREVIEW_SLOT_HEIGHT {
            ObjectFit::ScaleDown
        } else {
            ObjectFit::Cover
        };
        self.preview = Some((path.into(), object_fit));
        self
    }

    pub fn live(mut self) -> Self {
        self.live = true;
        self
    }

    pub fn on_folder(
        mut self,
        activation: impl Fn(&mut App) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.folder = Some(Rc::new(activation));
        self
    }

    pub fn element(&self) -> Div {
        self.layout.render(self, toast_runtime())
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

fn slab_height(live_rows: usize, visible_rows: usize, header: bool, summary: bool) -> f32 {
    let header = if header { HEADER_HEIGHT } else { 0.0 };
    let band = if live_rows > 0 { LIVE_BAND_HEIGHT } else { 0.0 };
    let rows = (live_rows + visible_rows) as f32 * ROW_HEIGHT;
    let summary = if summary { SUMMARY_HEIGHT } else { 0.0 };
    header + band + rows + summary
}

fn visible_slab_counts(non_live_rows: usize, expanded: bool) -> (bool, usize, bool) {
    let header = non_live_rows >= 2;
    let visible = if expanded || non_live_rows <= MAX_VISIBLE_NON_LIVE_ROWS {
        non_live_rows
    } else {
        MAX_VISIBLE_NON_LIVE_ROWS
    };
    let summary = !expanded && non_live_rows > MAX_VISIBLE_NON_LIVE_ROWS;
    (header, visible, summary)
}

fn compute_slab_height(rows: &[SlabSnapshotRow], expanded: bool) -> f32 {
    let live_rows = rows.iter().filter(|row| row.toast.live).count();
    let non_live_rows = rows.len() - live_rows;
    let (header, visible, summary) = visible_slab_counts(non_live_rows, expanded);
    slab_height(live_rows, visible, header, summary)
}

pub trait ToastPresenter {
    fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()>;
    fn dismiss(&self, cx: &mut App);
    fn is_idle(&self) -> bool;
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

impl ToastPresenter for BannerPresenter {
    fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        BannerPresenter::show(self, toast, cx)
    }

    fn dismiss(&self, cx: &mut App) {
        BannerPresenter::dismiss(self, cx)
    }

    fn is_idle(&self) -> bool {
        self.active.borrow().is_none()
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
}

struct SlabSnapshotRow {
    id: RowId,
    toast: Toast,
}

struct HostState {
    surface: Option<OpenedSurface<SlabToastView>>,
    rows: Vec<ToastRow>,
    next_id: u64,
    next_generation: u64,
    hovered: bool,
    queued_removals: Vec<RowId>,
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
        height: f32,
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
                hovered: false,
                queued_removals: Vec::new(),
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
            match target {
                Some(index) => {
                    generation = state.next_generation();
                    let row = &mut state.rows[index];
                    row.generation = generation;
                    row.toast = toast;
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

            let snapshot: Vec<SlabSnapshotRow> = state
                .rows
                .iter()
                .map(|row| SlabSnapshotRow {
                    id: row.id,
                    toast: row.toast.clone(),
                })
                .collect();
            let height = compute_slab_height(&snapshot, state.expanded);

            let outcome = if state.surface.is_none() {
                PushOutcome::Open { placement, height }
            } else {
                PushOutcome::Notify
            };
            (outcome, row_id, generation)
        };

        match outcome {
            PushOutcome::Open { placement, height } => {
                let owner: &str = &self.title;
                let host = self.clone();
                let surface = Surface::new(SurfaceKind::Toast)
                    .title(owner)
                    .placement(placement)
                    .size(size(px(SLAB_WIDTH), px(height)))
                    .open(&self.tracker, cx, move |dismisser, _window, _cx| {
                        SlabToastView { host, dismisser }
                    });
                match surface {
                    Ok(surface) => {
                        self.state.borrow_mut().surface.replace(surface);
                        present_topmost(&self.title);
                    }
                    Err(error) => {
                        let mut state = self.state.borrow_mut();
                        state.rows.clear();
                        state.queued_removals.clear();
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
            state.queued_removals.clear();
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

    fn set_hovered(&self, hovered: bool, cx: &mut App) {
        let drained = {
            let mut state = self.state.borrow_mut();
            state.hovered = hovered;
            if hovered {
                Vec::new()
            } else {
                std::mem::take(&mut state.queued_removals)
            }
        };
        for id in drained {
            self.remove(id, cx);
        }
    }

    fn toggle_expanded(&self, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            state.expanded = !state.expanded;
        }
        self.notify_view(cx);
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
            }
        }
        self.notify_view(cx);
    }

    fn activate(&self, id: RowId, cx: &mut App) {
        let activation = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| row.toast.activation.clone())
        };
        let Some(activation) = activation else {
            return;
        };
        match activation(cx) {
            Ok(()) => self.remove(id, cx),
            Err(error) => self.mark_row_failed(id, error, cx),
        }
    }

    fn run_folder(&self, id: RowId, cx: &mut App) {
        let action = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| row.toast.folder.clone())
        };
        let Some(action) = action else {
            return;
        };
        if let Err(error) = action(cx) {
            self.mark_row_failed(id, error, cx);
        }
    }

    fn on_timer(&self, id: RowId, generation: u64, cx: &mut App) {
        let owned = {
            let state = self.state.borrow();
            state
                .rows
                .iter()
                .any(|row| row.id == id && row.generation == generation)
        };
        if !owned {
            return;
        }
        let hovered = self.state.borrow().hovered;
        if hovered {
            self.state.borrow_mut().queued_removals.push(id);
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
            state.queued_removals.retain(|queued| !ids.contains(queued));
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
            state.queued_removals.clear();
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
            })
            .collect();
        (rows, state.expanded)
    }
}

impl ToastPresenter for SlabPresenter {
    fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        SlabPresenter::show(self, toast, cx)
    }

    fn dismiss(&self, cx: &mut App) {
        SlabPresenter::dismiss(self, cx)
    }

    fn is_idle(&self) -> bool {
        let state = self.state.borrow();
        state.surface.is_none() && state.rows.is_empty()
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
        let _ = cx.update(|cx| presenter.on_timer(id, generation, cx));
    })
    .detach();
}

struct SlabToastView {
    host: SlabPresenter,
    dismisser: SurfaceDismisser,
}

impl Render for SlabToastView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let palette = toast_runtime();
        let (snapshot, expanded) = self.host.slab_snapshot();

        let mut live_rows: Vec<SlabSnapshotRow> = Vec::new();
        let mut list_rows: Vec<SlabSnapshotRow> = Vec::new();
        for row in snapshot {
            if row.toast.live {
                live_rows.push(row);
            } else {
                list_rows.push(row);
            }
        }

        let non_live_total = list_rows.len();
        let (header_shown, visible_non_live, summary_shown) =
            visible_slab_counts(non_live_total, expanded);
        let hidden_count = non_live_total.saturating_sub(MAX_VISIBLE_NON_LIVE_ROWS);
        let display_rows: Vec<SlabSnapshotRow> = if expanded {
            list_rows
        } else if non_live_total > visible_non_live {
            list_rows.split_off(non_live_total - visible_non_live)
        } else {
            list_rows
        };

        let height = slab_height(
            live_rows.len(),
            display_rows.len(),
            header_shown,
            summary_shown,
        );
        self.dismisser
            .resize_window(size(px(SLAB_WIDTH), px(height)), window);

        let mut contents: Vec<AnyElement> = Vec::new();
        if header_shown {
            contents
                .push(slab_header(non_live_total, palette, self.host.clone()).into_any_element());
        }
        if !live_rows.is_empty() {
            contents.push(live_band_label(palette).into_any_element());
            for row in &live_rows {
                contents.push(slab_row_view(row, palette, self.host.clone()).into_any_element());
            }
        }
        for row in &display_rows {
            contents.push(slab_row_view(row, palette, self.host.clone()).into_any_element());
        }
        if summary_shown {
            contents.push(summary_row(hidden_count, palette, self.host.clone()).into_any_element());
        }

        let host = self.host.clone();
        slab_root(palette)
            .id("toast-slab-root")
            .on_hover(move |entered, _, cx| host.set_hovered(*entered, cx))
            .children(contents)
    }
}

fn slab_root(palette: ToastPalette) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .overflow_hidden()
        .rounded_none()
        .shadow(crate::kit::float_shadow(palette.text_primary))
        .bg(rgb(palette.window_bg))
}

fn slab_header(row_count: usize, palette: ToastPalette, host: SlabPresenter) -> Div {
    div()
        .flex_none()
        .h(px(HEADER_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .px(px(12.0))
        .text_size(px(qol_theme::TEXT_NANO))
        .text_color(rgb(palette.text_muted))
        .child(SharedString::from(format!("{row_count} notifications")))
        .child(div().flex_1())
        .child(
            div()
                .id("toast-clear-all")
                .h_full()
                .px(px(10.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(qol_theme::TEXT_NANO))
                .text_color(rgb(palette.text_muted))
                .hover(move |mut style| {
                    style.background = Some(rgb(palette.surface_hovered).into());
                    style
                })
                .child(SharedString::from("Clear all"))
                .on_click(move |_, _, cx| host.clear_all(cx)),
        )
}

fn live_band_label(palette: ToastPalette) -> Div {
    div()
        .flex_none()
        .h(px(LIVE_BAND_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .px(px(16.0))
        .text_size(px(qol_theme::TEXT_NANO))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.text_muted))
        .child(SharedString::from("LIVE"))
}

fn summary_row(hidden_count: usize, palette: ToastPalette, host: SlabPresenter) -> Stateful<Div> {
    div()
        .id("toast-summary")
        .flex_none()
        .h(px(SUMMARY_HEIGHT))
        .pl(px(16.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .bg(rgb(palette.surface_raised))
        .text_size(px(qol_theme::TEXT_MICRO))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.text_muted))
        .child(SharedString::from(format!(
            "{hidden_count} older notifications"
        )))
        .on_click(move |_, _, cx| host.toggle_expanded(cx))
}

fn slab_row_view(row: &SlabSnapshotRow, palette: ToastPalette, host: SlabPresenter) -> Div {
    let id = row.id;
    let mut container = div()
        .flex_none()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row();
    if row.toast.live {
        container = container.bg(rgb(palette.surface_raised));
    } else if row.toast.tone == ToastTone::Danger {
        container = container.bg(rgba(crate::kit::alpha(palette.danger, 26)));
    }

    let open_parts = open_zone_parts(row, palette);
    let mut container = if row.toast.activation.is_some() {
        let host_for_click = host.clone();
        container.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_row()
                .children(open_parts)
                .id(("toast-open", id.0))
                .cursor_pointer()
                .hover(move |mut style| {
                    style.background = Some(rgb(palette.surface_hovered).into());
                    style
                })
                .on_click(move |_, _, cx| host_for_click.activate(id, cx)),
        )
    } else {
        container.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_row()
                .children(open_parts),
        )
    };

    if row.toast.folder.is_some() {
        container = container.child(gutter());
        container = container.child(folder_control(row, palette, host.clone()));
    }

    container = container.child(gutter());
    container = container.child(dismiss_control(row, palette, host));
    container
}

fn open_zone_parts(row: &SlabSnapshotRow, palette: ToastPalette) -> Vec<AnyElement> {
    vec![
        preview_slot(row, palette).into_any_element(),
        text_column(row, palette).into_any_element(),
    ]
}

fn gutter() -> Div {
    div().flex_none().h_full().w(px(GUTTER))
}

fn preview_slot(row: &SlabSnapshotRow, palette: ToastPalette) -> Div {
    let tone_color = row.toast.tone.color(palette);
    let mut slot = div()
        .flex_none()
        .w(px(PREVIEW_WIDTH))
        .h_full()
        .overflow_hidden();
    match &row.toast.preview {
        Some((path, object_fit)) => {
            slot = slot.child(
                img(path.clone())
                    .w_full()
                    .h_full()
                    .object_fit(duplicate_object_fit(object_fit)),
            );
        }
        None => {
            slot = slot
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(crate::kit::alpha(tone_color, 51)))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(tone_color))
                        .child(SharedString::from(tone_glyph(row.toast.tone))),
                );
        }
    }
    slot
}

fn text_column(row: &SlabSnapshotRow, palette: ToastPalette) -> Div {
    let mut column = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(3.0))
        .px(px(TEXT_PAD))
        .child(
            div()
                .w_full()
                .truncate()
                .text_size(px(qol_theme::TEXT_CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text_primary))
                .child(row.toast.title.clone()),
        );
    if row.toast.message.is_empty() {
        return column;
    }
    if row.toast.message_is_path {
        let (head, tail) = crate::kit::path_label(&row.toast.message);
        column = column.child(path_body_line(head, tail, palette));
    } else {
        column = column.child(
            div()
                .w_full()
                .truncate()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(palette.text_secondary))
                .child(row.toast.message.clone()),
        );
    }
    column
}

fn path_body_line(head: String, tail: String, palette: ToastPalette) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_row()
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(palette.text_secondary))
                .child(SharedString::from(head)),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(qol_theme::TEXT_MICRO))
                .text_color(rgb(palette.text_secondary))
                .child(SharedString::from(tail)),
        )
}

fn folder_control(
    row: &SlabSnapshotRow,
    palette: ToastPalette,
    host: SlabPresenter,
) -> Stateful<Div> {
    let id = row.id;
    div()
        .id(("toast-folder", id.0))
        .flex_none()
        .w(px(FOLDER_WIDTH))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(px(qol_theme::TEXT_NANO))
        .text_color(rgb(palette.text_secondary))
        .hover(move |mut style| {
            style.background = Some(rgb(palette.surface_hovered).into());
            style.text.get_or_insert_with(Default::default).color =
                Some(rgb(palette.accent).into());
            style
        })
        .child(SharedString::from("Folder"))
        .on_click(move |_, _, cx| host.run_folder(id, cx))
}

fn dismiss_control(
    row: &SlabSnapshotRow,
    palette: ToastPalette,
    host: SlabPresenter,
) -> Stateful<Div> {
    let id = row.id;
    div()
        .id(("toast-dismiss", id.0))
        .flex_none()
        .w(px(DISMISS_WIDTH))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(px(qol_theme::TEXT_MICRO))
        .text_color(rgb(palette.text_secondary))
        .hover(move |mut style| {
            style.background = Some(rgb(palette.surface_hovered).into());
            style.text.get_or_insert_with(Default::default).color =
                Some(rgb(palette.danger).into());
            style
        })
        .child(SharedString::from("\u{2715}"))
        .on_click(move |_, _, cx| host.remove(id, cx))
}

impl ToastLayout {
    fn render(self, toast: &Toast, palette: ToastPalette) -> Div {
        match self.style {
            ToastStyle::Compact => render_compact(toast, palette),
            ToastStyle::Status => render_status(toast, palette),
        }
    }
}

fn render_compact(toast: &Toast, palette: ToastPalette) -> Div {
    toast_root(palette).child(tone_bar(toast, palette)).child(
        div()
            .flex_1()
            .min_w_0()
            .flex_col()
            .justify_center()
            .gap(px(2.0))
            .px_4()
            .py_3()
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text_primary))
                    .child(toast.title.clone()),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_MICRO))
                    .text_color(rgb(palette.text_secondary))
                    .child(toast.message.clone()),
            ),
    )
}

fn render_status(toast: &Toast, palette: ToastPalette) -> Div {
    toast_root(palette).child(tone_bar(toast, palette)).child(
        div()
            .flex_1()
            .min_w_0()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .px_6()
            .py_3()
            .text_center()
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_DISPLAY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text_primary))
                    .child(toast.title.clone()),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(palette.text_secondary))
                    .child(toast.message.clone()),
            ),
    )
}

fn toast_root(palette: ToastPalette) -> Div {
    div()
        .size_full()
        .flex()
        .flex_row()
        .overflow_hidden()
        .rounded_none()
        .shadow(crate::kit::float_shadow(palette.text_primary))
        .bg(rgb(palette.window_bg))
}

fn tone_bar(toast: &Toast, palette: ToastPalette) -> Div {
    div()
        .flex_none()
        .w(px(qol_theme::SPACE_MARK))
        .h_full()
        .rounded(px(qol_theme::RADIUS_TONE_BAR))
        .bg(rgb(toast.tone.color(palette)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN};
    use crate::theme::ToastPalette;

    use super::{
        routed_presentation, slab_height, visible_slab_counts, Presentation, Toast, ToastLayout,
        ToastStyle, ToastTone,
    };

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
        let palette = ToastPalette {
            window_bg: 1,
            border: 2,
            text_primary: 3,
            text_secondary: 4,
            info: 5,
            success: 6,
            warning: 7,
            danger: 8,
            surface_raised: 9,
            surface_hovered: 10,
            text_muted: 11,
            accent: 12,
        };
        let cases = [
            (ToastTone::Neutral, 2),
            (ToastTone::Info, 5),
            (ToastTone::Success, 6),
            (ToastTone::Warning, 7),
            (ToastTone::Danger, 8),
        ];

        for (tone, expected) in cases {
            assert_eq!(tone.color(palette), expected, "tone: {tone:?}");
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
    fn preview_object_fit_scales_down_only_below_the_slot() {
        let smaller_width = Toast::new("t", "m", ToastLayout::status()).preview(
            std::path::Path::new("/tmp/a.png"),
            71,
            68,
        );
        assert!(matches!(
            smaller_width.preview.expect("preview stored").1,
            gpui::ObjectFit::ScaleDown
        ));
        let smaller_height = Toast::new("t", "m", ToastLayout::status()).preview(
            std::path::Path::new("/tmp/b.png"),
            72,
            67,
        );
        assert!(matches!(
            smaller_height.preview.expect("preview stored").1,
            gpui::ObjectFit::ScaleDown
        ));
        let exactly_slot = Toast::new("t", "m", ToastLayout::status()).preview(
            std::path::Path::new("/tmp/c.png"),
            72,
            68,
        );
        assert!(matches!(
            exactly_slot.preview.expect("preview stored").1,
            gpui::ObjectFit::Cover
        ));
        let larger = Toast::new("t", "m", ToastLayout::status()).preview(
            std::path::Path::new("/tmp/d.png"),
            300,
            400,
        );
        assert!(matches!(
            larger.preview.expect("preview stored").1,
            gpui::ObjectFit::Cover
        ));
    }

    #[test]
    fn slab_height_matches_the_fixed_geometry() {
        assert_eq!(slab_height(0, 1, false, false), 68.0);
        assert_eq!(slab_height(0, 2, true, false), 166.0);
        assert_eq!(slab_height(0, 4, true, true), 338.0);
        assert_eq!(slab_height(1, 0, false, false), 88.0);
        assert_eq!(slab_height(2, 1, true, false), 254.0);
    }

    #[test]
    fn visible_rows_and_summary_follow_the_cap_and_expansion() {
        let cases = [
            ((1usize, false), (false, 1usize, false)),
            ((2, false), (true, 2, false)),
            ((4, false), (true, 4, false)),
            ((5, false), (true, 4, true)),
            ((7, false), (true, 4, true)),
            ((5, true), (true, 5, false)),
        ];
        for ((non_live, expanded), expected) in cases {
            assert_eq!(
                visible_slab_counts(non_live, expanded),
                expected,
                "non_live={non_live} expanded={expanded}"
            );
        }
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
}
