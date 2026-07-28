use crate::capture::space::CaptureKind;
use crate::ui::region_selector::{
    open_window, rect_from_bounds, RectMapper, RegionSelector, SelectionState, SelectorReveal,
    SelectorWindow, SelectorWindowSources,
};
use crate::Rect;
use gpui::{App, Bounds, Context, Focusable, Pixels, WindowHandle};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

#[derive(Clone, Default)]
pub(crate) struct SelectorCache {
    handle: Rc<RefCell<Option<WindowHandle<RegionSelector>>>>,
}

pub(crate) fn pre_create_cached(
    cache: &SelectorCache,
    selector: SelectorWindow,
    kind: CaptureKind,
    cx: &mut App,
) -> Option<String> {
    if cache.handle.borrow().is_some() {
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
    let Some(handle) = open_window(selector, state.clone(), false, true, true, cx) else {
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
    *cache.handle.borrow_mut() = Some(handle);
    qol_runtime::probe!("SHOT_SELECT_PRECREATE", "result=ok hidden={_hidden}");
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
    let handle = (*cache.handle.borrow())?;
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
        view.reset(state, false, window_bounds, selector.sources, reveal, cx);
        let _ = qol_gpui::popup_window::sync_window_layout(
            &title,
            window,
            window_bounds.origin,
            window_bounds.size,
        );
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view.start_active_monitor_poll(cx);
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
            *cache.handle.borrow_mut() = None;
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
