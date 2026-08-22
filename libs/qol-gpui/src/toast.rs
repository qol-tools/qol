use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::placement::{
    anchor_placement, Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN,
};
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
            timeout: Some(DEFAULT_TIMEOUT),
            activation: None,
        }
    }

    pub fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
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

    use super::{ToastLayout, ToastStyle, ToastTone};

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
