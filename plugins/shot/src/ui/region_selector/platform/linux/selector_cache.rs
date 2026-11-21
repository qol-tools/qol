use crate::capture::space::CaptureKind;
use crate::ui::preview::{live_topology, WarmWindowPool};
use crate::ui::region_selector::{
    open_window, rect_from_bounds, RectMapper, RegionSelector, SelectionState, SelectorReveal,
    SelectorWindow, SelectorWindowSources,
};
use crate::Rect;
use gpui::{App, Bounds, Context, Focusable, Pixels};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

const SELECTOR_WINDOW_KIND: &str = "selector";
const SELECTOR_POOL_CAPACITY: usize = 1;

pub(crate) struct SelectorCache {
    pool: WarmWindowPool<RegionSelector>,
}

impl Default for SelectorCache {
    fn default() -> Self {
        Self {
            pool: WarmWindowPool::new(SELECTOR_WINDOW_KIND, SELECTOR_POOL_CAPACITY),
        }
    }
}

pub(crate) fn pre_create_cached(
    cache: &SelectorCache,
    selector: SelectorWindow,
    kind: CaptureKind,
    cx: &mut App,
) -> Option<String> {
    let topology = live_topology(cx);
    let bounds = rect_from_bounds(selector.bounds);
    let size = (bounds.w, bounds.h);
    if cache.pool.peek(size, &topology, cx).is_some() {
        return None;
    }
    let title = selector.title.clone();
    let (tx, _rx) = mpsc::channel();
    let active_bounds = selector.active_bounds;
    let default_target = selector.default_target;
    let monitor_bounds = selector.monitor_bounds.clone();
    let titles = vec![selector.title.clone()];
    let state = Rc::new(RefCell::new(SelectionState::new(
        tx,
        active_bounds,
        default_target,
        monitor_bounds,
        titles,
        kind,
    )));
    let Some(handle) = open_window(selector, state.clone(), false, true, false, cx) else {
        qol_runtime::probe!("SHOT_SELECT_PRECREATE", "result=failed");
        return None;
    };
    state.borrow_mut().handles = vec![handle];
    let _hidden = handle
        .update(cx, |view, window, _cx| {
            view.handle = Some(handle);
            let _reason = qol_gpui::popup_window::reason_scope("shot-selector-precreate");
            qol_gpui::popup_window::hide_for_capture(&view.title, window)
        })
        .unwrap_or(false);
    let key = cache.pool.key(&topology, size);
    if !cache.pool.put(key, handle) {
        let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
        qol_runtime::probe!("SHOT_SELECT_PRECREATE", "result=failed");
        return None;
    }
    qol_runtime::probe!("SHOT_SELECT_PRECREATE", "result=ok hidden={_hidden}");
    Some(title)
}

pub(crate) fn open_adopted(
    cache: &SelectorCache,
    tx: &mut Option<mpsc::Sender<Option<Rect>>>,
    selector: &mut Option<SelectorWindow>,
    kind: CaptureKind,
    cx: &mut App,
) -> Option<String> {
    if cache.pool.len() >= SELECTOR_POOL_CAPACITY {
        return None;
    }
    let (Some(tx), Some(selector)) = (tx.take(), selector.take()) else {
        return None;
    };
    let topology = live_topology(cx);
    let bounds = rect_from_bounds(selector.bounds);
    let size = (bounds.w, bounds.h);
    let title = selector.title.clone();
    let state = Rc::new(RefCell::new(SelectionState::new(
        tx,
        selector.active_bounds,
        selector.default_target,
        selector.monitor_bounds.clone(),
        vec![title.clone()],
        kind,
    )));
    let Some(handle) = open_window(selector, state.clone(), false, true, true, cx) else {
        qol_runtime::probe!("SHOT_SELECT_OPEN", "selectors=1 result=none");
        return None;
    };
    state.borrow_mut().handles = vec![handle];
    let _ = handle.update(cx, |view, _window, cx| {
        view.handle = Some(handle);
        view.start_active_monitor_poll(cx);
    });
    let _adopted = cache.pool.put(cache.pool.key(&topology, size), handle);
    qol_runtime::probe!(
        "SHOT_SELECT_OPEN",
        "selectors=1 windows=1 result=adopt pooled={_adopted}"
    );
    Some(title)
}

pub(crate) fn open_cached(
    cache: &SelectorCache,
    tx: &mut Option<mpsc::Sender<Option<Rect>>>,
    selector: &mut Option<SelectorWindow>,
    kind: CaptureKind,
    reveal: SelectorReveal,
    cx: &mut App,
) -> Option<String> {
    let size = {
        let selector = selector.as_ref()?;
        let bounds = rect_from_bounds(selector.bounds);
        (bounds.w, bounds.h)
    };
    let topology = live_topology(cx);
    let handle = cache.pool.peek(size, &topology, cx)?;
    let result = handle.update(cx, |view, window, cx| {
        let tx = tx.take()?;
        let selector = selector.take()?;
        let title = view.title.clone();
        let window_bounds = selector.bounds;
        let monitor_bounds = selector.monitor_bounds.clone();
        let state = Rc::new(RefCell::new(SelectionState::new(
            tx,
            selector.active_bounds,
            selector.default_target,
            monitor_bounds,
            vec![title.clone()],
            kind,
        )));
        state.borrow_mut().handles = vec![handle];
        state.borrow_mut().record_display(
            rect_from_bounds(window_bounds),
            window.scale_factor() as f64,
        );
        view.handle = Some(handle);
        crate::ui::release_image(view.frozen_image.take(), window);
        view.reset(state, false, window_bounds, selector.sources, reveal, cx);
        let _ = qol_gpui::popup_window::sync_window_layout(
            &title,
            window,
            window_bounds.origin,
            window_bounds.size,
            None,
        );
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view.start_active_monitor_poll(cx);
        view.schedule_reveal_after_present(window, cx);
        qol_runtime::probe!("SHOT_SELECT_WINDOW", "title={title} state=reuse");
        Some(title)
    });
    match result {
        Ok(Some(title)) => {
            qol_runtime::probe!("SHOT_SELECT_OPEN", "selectors=1 windows=1 result=reuse");
            Some(title)
        }
        Ok(None) => None,
        Err(_) => {
            let _ = cache.pool.take(size, &topology, cx);
            qol_runtime::probe!("SHOT_SELECT_OPEN", "selectors=1 result=stale-cache");
            None
        }
    }
}

pub(crate) fn identity_rect_mapper() -> RectMapper {
    Rc::new(Some)
}

impl RegionSelector {
    fn reset(
        &mut self,
        state: Rc<RefCell<SelectionState>>,
        quit_on_finish: bool,
        window_bounds: Bounds<Pixels>,
        sources: SelectorWindowSources,
        reveal: SelectorReveal,
        cx: &mut Context<Self>,
    ) {
        let _image_started = std::time::Instant::now();
        let frozen_image = sources
            .frozen_frame
            .as_ref()
            .and_then(|frame| frame.render_image(rect_from_bounds(window_bounds)));
        qol_runtime::probe!(
            "SHOT_FREEZE_IMAGE",
            "ms={} ready={}",
            _image_started.elapsed().as_millis(),
            frozen_image.is_some()
        );
        self.state = state;
        self.quit_on_finish = quit_on_finish;
        self.window_bounds = window_bounds;
        self.map_rect = sources.map_rect;
        self.global_pointer = sources.global_pointer;
        self.cancel_signal = sources.cancel_signal;
        self.active_bounds = sources.active_bounds;
        self.hover_target = sources.hover_target;
        self.frozen_image = frozen_image;
        self.reveal_generation = self.reveal_generation.wrapping_add(1);
        self.scheduled_reveal_generation = None;
        self.pending_reveal = Some(reveal);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{identity_rect_mapper, open_adopted, open_cached, SelectorCache};
    use crate::capture::space::CaptureKind;
    use crate::ui::region_selector::{
        SelectorReveal, SelectorWindow, SelectorWindowOptions, SelectorWindowSources,
    };
    use gpui::{point, px, size, Bounds, TestAppContext, WindowDecorations, WindowKind};
    use std::rc::Rc;
    use std::sync::mpsc;

    fn selector() -> SelectorWindow {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        SelectorWindow::new(
            bounds,
            vec![bounds],
            None,
            None,
            SelectorWindowOptions {
                display_id: None,
                placement: None,
                kind: WindowKind::PopUp,
                decorations: WindowDecorations::Client,
                focus: true,
            },
            SelectorWindowSources {
                map_rect: identity_rect_mapper(),
                global_pointer: None,
                cancel_signal: None,
                active_bounds: None,
                hover_target: None,
                frozen_frame: None,
            },
        )
    }

    #[gpui::test]
    fn a_cold_opened_selector_is_reused_by_the_next_open(cx: &mut TestAppContext) {
        let cache = SelectorCache::default();
        let (tx, _cold_rx) = mpsc::channel();
        let adopted = cx.update(|cx| {
            open_adopted(
                &cache,
                &mut Some(tx),
                &mut Some(selector()),
                CaptureKind::Screenshot,
                cx,
            )
        });
        assert!(adopted.is_some(), "the cold selector did not open");
        assert_eq!(cache.pool.len(), 1, "the cold selector was not pooled");

        let (tx, _warm_rx) = mpsc::channel();
        let reveal: SelectorReveal = Rc::new(|_title: String| {});
        let reused = cx.update(|cx| {
            open_cached(
                &cache,
                &mut Some(tx),
                &mut Some(selector()),
                CaptureKind::Screenshot,
                reveal,
                cx,
            )
        });
        assert_eq!(reused, adopted, "the next open built a new selector");
        assert_eq!(cache.pool.len(), 1);
    }

    #[gpui::test]
    fn a_full_pool_hands_the_cold_open_back_untouched(cx: &mut TestAppContext) {
        let cache = SelectorCache::default();
        let (tx, _pooled_rx) = mpsc::channel();
        let pooled = cx.update(|cx| {
            open_adopted(
                &cache,
                &mut Some(tx),
                &mut Some(selector()),
                CaptureKind::Screenshot,
                cx,
            )
        });
        assert!(pooled.is_some());

        let (tx, _extra_rx) = mpsc::channel();
        let mut tx = Some(tx);
        let mut extra = Some(selector());
        let adopted =
            cx.update(|cx| open_adopted(&cache, &mut tx, &mut extra, CaptureKind::Screenshot, cx));
        assert!(adopted.is_none());
        assert!(tx.is_some(), "the sender was consumed without a window");
        assert!(
            extra.is_some(),
            "the selector was consumed without a window"
        );
        assert_eq!(cache.pool.len(), 1);
    }
}
