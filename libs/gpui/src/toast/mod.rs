use std::cell::Cell;
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
use crate::popup_window::HiddenWindowsBarrier;

mod banner;
mod card;
mod follow;
mod pile;
mod rows;
mod shape;
mod slab;

use banner::BannerPresenter;
use slab::SlabPresenter;

const POINTER_POLL: Duration = Duration::from_millis(60);
const AGE_TICK: Duration = Duration::from_secs(30);
const WINDOW_ROOM: f32 = 4096.0;

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
            size: size(px(pile::CARD_WIDTH), px(qol_theme::toast::MESSAGE_HEIGHT)),
            style: ToastStyle::Status,
        }
    }

    pub fn compact() -> Self {
        Self {
            placement: MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN),
            size: size(px(pile::CARD_WIDTH), px(pile::CARD_HEIGHT)),
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
            Self::Danger => Some(qol_theme::STAY_ERROR),
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
    mark: Option<qol_theme::Mark>,
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
            mark: None,
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

    pub fn mark(mut self, mark: qol_theme::Mark) -> Self {
        self.mark = Some(mark);
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
        let now = Instant::now();
        let row = SlabSnapshotRow {
            id: RowId(0),
            toast: Rc::new(self.clone()),
            created: now,
            deadline: None,
        };
        match routed_presentation(self) {
            Presentation::Banner => card::message(&row, 1.0, 1.0, 1.0, crate::kit::kit()),
            Presentation::Slab => card::lone(&row, None, now),
        }
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

    fn for_the_top(self) -> Self {
        if self.timeout_explicit {
            self
        } else {
            self.timeout(message_timeout())
        }
    }

    fn is_message(&self) -> bool {
        self.tone != ToastTone::Danger && self.preview.is_none()
    }

    fn in_the_pile(mut self) -> Self {
        if self.layout.style() == ToastStyle::Status {
            self.layout = ToastLayout::compact();
        }
        self
    }

    fn effective_timeout(&self) -> Option<Duration> {
        if self.timeout_explicit {
            self.timeout
        } else {
            self.tone.default_timeout()
        }
    }
}

fn message_timeout() -> Duration {
    qol_config::config_dir()
        .and_then(|dir| {
            std::fs::read_to_string(dir.join(qol_conventions::NOTIFICATIONS_SETTINGS_FILE)).ok()
        })
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| settings.get("message_seconds")?.as_u64())
        .filter(|seconds| *seconds > 0)
        .map_or(qol_theme::STAY_MESSAGE, Duration::from_secs)
}

#[derive(Default)]
struct Tick(Rc<Cell<bool>>);

impl Tick {
    fn after<V: 'static>(&self, interval: Duration, cx: &mut Context<V>) {
        if self.0.replace(true) {
            return;
        }
        let pending = self.0.clone();
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(interval).await;
            pending.set(false);
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RowId(u64);

#[derive(Clone)]
struct SlabSnapshotRow {
    id: RowId,
    toast: Rc<Toast>,
    created: Instant,
    deadline: Option<Instant>,
}

enum Presentation {
    Banner,
    Slab,
}

fn routed_presentation(toast: &Toast) -> Presentation {
    match toast.layout.style() {
        ToastStyle::Status if toast.is_message() => Presentation::Banner,
        _ => Presentation::Slab,
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
            Presentation::Slab => self.slab.show(toast.in_the_pile(), cx),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.banner.dismiss(cx);
        self.slab.dismiss(cx);
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::placement::{Corner, MonitorPlacement, CORNER_MARGIN, TOP_CENTER_MARGIN};

    use super::{
        message_timeout, routed_presentation, Presentation, Toast, ToastLayout, ToastStyle,
        ToastTone,
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
        assert_eq!(ToastLayout::status().size().width.to_f64(), 440.0);
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
        assert_eq!(ToastLayout::compact().size().width.to_f64(), 440.0);
        assert_eq!(ToastLayout::compact().size().height.to_f64(), 84.0);
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
                (440.0, 84.0),
            ),
            (
                ToastLayout::status(),
                MonitorPlacement::top_center(TOP_CENTER_MARGIN),
                (440.0, 84.0),
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
            (ToastTone::Danger, Some(Duration::from_secs(10))),
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
        assert_eq!(danger.effective_timeout(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn explicit_timeout_beats_the_tone_default() {
        let toast = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Danger);
        assert_eq!(toast.effective_timeout(), Some(Duration::from_secs(10)));
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
    fn the_top_keeps_a_chosen_time_and_gives_the_rest_the_message_time() {
        let plain = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Warning);
        assert_eq!(
            plain.for_the_top().effective_timeout(),
            Some(message_timeout())
        );
        let chosen = Toast::new("t", "m", ToastLayout::status()).timeout(Duration::from_secs(9));
        assert_eq!(
            chosen.for_the_top().effective_timeout(),
            Some(Duration::from_secs(9))
        );
        let kept = Toast::new("t", "m", ToastLayout::status()).persistent();
        assert_eq!(kept.for_the_top().effective_timeout(), None);
    }

    #[test]
    fn errors_and_pictures_go_to_the_pile_never_the_top() {
        let error = Toast::new("t", "m", ToastLayout::status()).tone(ToastTone::Danger);
        assert!(matches!(routed_presentation(&error), Presentation::Slab));
        assert_eq!(error.in_the_pile().layout, ToastLayout::compact());
        let picture = Toast::new("t", "m", ToastLayout::status()).artifact("/nowhere/shot.png");
        assert!(matches!(routed_presentation(&picture), Presentation::Slab));
        let saving = Toast::new("t", "m", ToastLayout::status()).persistent();
        assert!(matches!(routed_presentation(&saving), Presentation::Banner));
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
                    width: Dimension::length(qol_theme::toast::PREVIEW),
                    height: Dimension::length(super::pile::CARD_HEIGHT),
                },
                ..Default::default()
            })
            .unwrap();
        let dismiss = tree
            .new_leaf(Style {
                size: TaffySize {
                    width: Dimension::length(qol_theme::toast::CLOSE),
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
