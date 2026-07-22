#![allow(dead_code)]

use super::super::*;

#[derive(Clone, Default)]
pub(crate) struct SelectorCache;

pub(crate) fn pre_create_cached(
    _cache: &SelectorCache,
    _selector: SelectorWindow,
    _kind: CaptureKind,
    _cx: &mut App,
) -> Option<String> {
    None
}

pub(crate) fn open_cached(
    _cache: &SelectorCache,
    _tx: &mut Option<mpsc::Sender<Option<Rect>>>,
    _selector: &mut Option<SelectorWindow>,
    _kind: CaptureKind,
    _reveal: SelectorReveal,
    _cx: &mut App,
) -> Option<String> {
    None
}

pub(crate) fn show_cached_guide(
    _cache: &SelectorCache,
    _bounds: Bounds<Pixels>,
    _title: SharedString,
    _subtitle: SharedString,
    _reveal: SelectorReveal,
    _cx: &mut App,
) -> Option<String> {
    None
}

pub(crate) fn hide_cached_guide(_cache: &SelectorCache, _cx: &mut App) {}

pub(crate) fn identity_rect_mapper() -> RectMapper {
    Rc::new(Some)
}
