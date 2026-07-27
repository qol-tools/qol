use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN};
use crate::surface::{OpenedSurface, Surface, SurfaceDismisser, SurfaceKind};
use crate::theme::{toast_runtime, ToastPalette};

const COMPACT_WIDTH: f32 = 340.0;
const COMPACT_HEIGHT: f32 = 76.0;
const STATUS_WIDTH: f32 = 520.0;
const STATUS_HEIGHT: f32 = 78.0;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

static TOAST_HOST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

type Activation = Rc<dyn Fn(&mut App)>;

#[derive(Clone)]
pub struct ToastHost {
    tracker: MonitorTracker,
    active: Rc<RefCell<Option<ActiveToast>>>,
    generation: Rc<Cell<u64>>,
    title: Rc<str>,
}

struct ActiveToast {
    surface: OpenedSurface<ToastView>,
    layout: ToastLayout,
}

impl ToastHost {
    pub fn new(tracker: MonitorTracker) -> Self {
        let sequence = TOAST_HOST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            tracker,
            active: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
            title: format!("qol-toast-host-{}-{sequence}", std::process::id()).into(),
        }
    }

    pub fn show(&self, toast: Toast, cx: &mut App) -> anyhow::Result<()> {
        let timeout = toast.timeout;
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

    pub async fn wait_until_hidden(
        &self,
        cx: &mut AsyncApp,
    ) -> crate::popup_window::HiddenWindowsBarrier {
        crate::popup_window::wait_for_hidden_windows(cx, &self.title).await
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
        let host = self.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            if host.generation.get() != generation {
                return;
            }
            let _ = cx.update(|cx| host.dismiss(cx));
        })
        .detach();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastLayout {
    #[default]
    Compact,
    Status,
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

#[derive(Clone)]
pub struct Toast {
    title: SharedString,
    message: SharedString,
    tone: ToastTone,
    layout: ToastLayout,
    timeout: Option<Duration>,
    activation: Option<Activation>,
}

impl Toast {
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            tone: ToastTone::Neutral,
            layout: ToastLayout::Compact,
            timeout: Some(DEFAULT_TIMEOUT),
            activation: None,
        }
    }

    pub fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn layout(mut self, layout: ToastLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn persistent(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub fn on_activate(mut self, activation: impl Fn(&mut App) + 'static) -> Self {
        self.activation = Some(Rc::new(activation));
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

    fn open(
        self,
        tracker: &MonitorTracker,
        title: &str,
        cx: &mut App,
    ) -> anyhow::Result<OpenedSurface<ToastView>> {
        Surface::new(SurfaceKind::Toast)
            .title(title)
            .placement(self.layout.placement())
            .size(self.layout.size())
            .open(tracker, cx, move |dismisser, _window, _cx| ToastView {
                toast: self,
                dismisser,
            })
    }
}

struct ToastView {
    toast: Toast,
    dismisser: SurfaceDismisser,
}

impl Render for ToastView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = self.toast.element();
        let Some(activation) = self.toast.activation.clone() else {
            return root;
        };
        let dismisser = self.dismisser.clone();
        root = root.cursor_pointer().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _event, _window, cx| {
                activation(cx);
                dismisser.dismiss(cx);
            }),
        );
        root
    }
}

impl ToastLayout {
    pub fn placement(self) -> MonitorPlacement {
        match self {
            Self::Compact => MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
            Self::Status => MonitorPlacement::top_center(TOP_CENTER_MARGIN),
        }
    }

    pub fn size(self) -> Size<Pixels> {
        match self {
            Self::Compact => size(px(COMPACT_WIDTH), px(COMPACT_HEIGHT)),
            Self::Status => size(px(STATUS_WIDTH), px(STATUS_HEIGHT)),
        }
    }

    fn render(self, toast: &Toast, palette: ToastPalette) -> Div {
        match self {
            Self::Compact => render_compact(toast, palette),
            Self::Status => render_status(toast, palette),
        }
    }
}

fn render_compact(toast: &Toast, palette: ToastPalette) -> Div {
    toast_root(toast, palette)
        .flex_col()
        .justify_center()
        .gap_1()
        .px_4()
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.text_primary))
                .child(toast.title.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.text_secondary))
                .child(toast.message.clone()),
        )
}

fn render_status(toast: &Toast, palette: ToastPalette) -> Div {
    toast_root(toast, palette)
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .px_4()
        .text_center()
        .child(
            div()
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text_primary))
                .child(toast.title.clone()),
        )
        .child(
            div()
                .text_size(px(14.0))
                .text_color(rgb(palette.text_secondary))
                .child(toast.message.clone()),
        )
}

fn toast_root(toast: &Toast, palette: ToastPalette) -> Div {
    div()
        .size_full()
        .flex()
        .rounded_xl()
        .border_1()
        .border_color(rgb(toast.tone.color(palette)))
        .bg(rgb(palette.window_bg))
}

impl ToastTone {
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

#[cfg(test)]
mod tests {
    use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN};
    use crate::theme::ToastPalette;

    use super::{ToastLayout, ToastTone};

    #[test]
    fn layouts_select_shared_placement_and_dimensions() {
        let cases = [
            (
                ToastLayout::Compact,
                MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
                (340.0, 76.0),
            ),
            (
                ToastLayout::Status,
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
}
