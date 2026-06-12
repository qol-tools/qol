#[cfg(debug_assertions)]
use gpui::{Pixels, Window};

use super::input::InputEffect;
use super::LauncherView;
use qol_gpui::window::WindowPlacement;

#[cfg(debug_assertions)]
#[derive(Clone, PartialEq, Eq)]
pub(super) struct RenderSignature {
    showing: bool,
    title: String,
    query: String,
    query_len: usize,
    cursor: usize,
    mode: &'static str,
    fuzziness: &'static str,
    result_count: usize,
    visible_rows: usize,
    selected: usize,
    scroll_offset: usize,
    hidden_above: usize,
    hidden_below: usize,
    window_x: i32,
    window_y: i32,
    window_w: i32,
    window_h: i32,
    target_h: i32,
    visual_h: i32,
    selected_name: String,
}

#[cfg(debug_assertions)]
pub(super) struct RenderSample {
    pub result_count: usize,
    pub visible_rows: usize,
    pub scroll_offset: usize,
    pub hidden_above: usize,
    pub hidden_below: usize,
    pub results_height: f32,
    pub target_height: f32,
    pub selected_name: String,
    pub resize: Option<(f32, f32)>,
    pub total_us: u128,
    pub filter_us: u128,
    pub rows_us: u128,
    pub gap_us: u64,
}

pub(super) fn show(path: &'static str, title: &str, placement: &WindowPlacement) {
    #[cfg(debug_assertions)]
    {
        let bounds = placement.bounds;
        qol_runtime::probe!(
            "LAUNCHER_SHOW",
            "path={path} title={} pos=({},{}) size={}x{} target={}",
            token(title),
            px_i32(bounds.origin.x),
            px_i32(bounds.origin.y),
            px_i32(bounds.size.width),
            px_i32(bounds.size.height),
            token(&format!("{:?}", placement.target)),
        );
    }

    #[cfg(not(debug_assertions))]
    let _ = (path, title, placement);
}

pub(super) fn input(view: &LauncherView, key: &str, effect: InputEffect, result_count: usize) {
    #[cfg(debug_assertions)]
    {
        if matches!(effect, InputEffect::Ignore) {
            return;
        }

        qol_runtime::probe!(
            "LAUNCHER_INPUT",
            "key={} effect={} title={} q=\"{}\" q_len={} cursor={} selection={} selected={} results_before={} mode={} fuzz={}",
            token(key),
            effect_label(effect),
            token(&view.window_title),
            quoted(&view.state.query),
            view.state.query_len(),
            view.state.cursor,
            selection_label(view),
            view.state.selected,
            result_count,
            view.state.mode.label(),
            view.state.fuzziness.label(),
        );
    }

    #[cfg(not(debug_assertions))]
    let _ = (view, key, effect, result_count);
}

pub(super) fn dismiss(view: &LauncherView, from: &'static str) {
    #[cfg(debug_assertions)]
    {
        let selected_name = view
            .store
            .get(view.state.selected)
            .map(|scored| view.store.name(scored))
            .unwrap_or("");
        qol_runtime::probe!(
            "LAUNCHER_DISMISS",
            "from={from} title={} q=\"{}\" q_len={} results={} selected={} selected_name=\"{}\"",
            token(&view.window_title),
            quoted(&view.state.query),
            view.state.query_len(),
            view.store.result_count(),
            view.state.selected,
            quoted(selected_name),
        );
    }

    #[cfg(not(debug_assertions))]
    let _ = (view, from);
}

#[cfg(debug_assertions)]
pub(super) fn render(view: &mut LauncherView, window: &Window, sample: RenderSample) {
    let bounds = window.window_bounds().get_bounds();
    if let Some((from_h, to_h)) = sample.resize {
        qol_runtime::probe!(
            "LAUNCHER_RESIZE",
            "title={} q=\"{}\" rows={} results={} from_h={:.1} to_h={:.1} win=({},{},{}x{})",
            token(&view.window_title),
            quoted(&view.state.query),
            sample.visible_rows,
            sample.result_count,
            from_h,
            to_h,
            px_i32(bounds.origin.x),
            px_i32(bounds.origin.y),
            px_i32(bounds.size.width),
            px_i32(bounds.size.height),
        );
    }

    let signature = RenderSignature {
        showing: view.is_showing,
        title: view.window_title.clone(),
        query: view.state.query.clone(),
        query_len: view.state.query_len(),
        cursor: view.state.cursor,
        mode: view.state.mode.label(),
        fuzziness: view.state.fuzziness.label(),
        result_count: sample.result_count,
        visible_rows: sample.visible_rows,
        selected: view.state.selected,
        scroll_offset: sample.scroll_offset,
        hidden_above: sample.hidden_above,
        hidden_below: sample.hidden_below,
        window_x: px_i32(bounds.origin.x),
        window_y: px_i32(bounds.origin.y),
        window_w: px_i32(bounds.size.width),
        window_h: px_i32(bounds.size.height),
        target_h: (super::layout::HEADER_HEIGHT + sample.results_height).round() as i32,
        visual_h: sample.target_height.round() as i32,
        selected_name: compact(&sample.selected_name, 80),
    };

    if view.last_render_trace.as_ref() == Some(&signature) {
        return;
    }

    view.last_render_trace = Some(signature.clone());
    qol_runtime::probe!(
            "LAUNCHER_RENDER",
            "title={} showing={} q=\"{}\" q_len={} cursor={} mode={} fuzz={} results={} visible={} selected={} scroll={} hidden={}/{} win=({},{},{}x{}) target_h={} visual_h={} selected_name=\"{}\" total_us={} filter_us={} rows_us={} gap_us={}",
            token(&signature.title),
            signature.showing,
            quoted(&signature.query),
            signature.query_len,
            signature.cursor,
            signature.mode,
            signature.fuzziness,
            signature.result_count,
            signature.visible_rows,
            signature.selected,
            signature.scroll_offset,
            signature.hidden_above,
            signature.hidden_below,
            signature.window_x,
            signature.window_y,
            signature.window_w,
            signature.window_h,
            signature.target_h,
            signature.visual_h,
            quoted(&signature.selected_name),
            sample.total_us,
            sample.filter_us,
            sample.rows_us,
            sample.gap_us,
        );
}

#[cfg(debug_assertions)]
fn effect_label(effect: InputEffect) -> &'static str {
    match effect {
        InputEffect::Ignore => "ignore",
        InputEffect::Navigate => "navigate",
        InputEffect::QueryChanged => "query",
        InputEffect::Launch => "launch",
        InputEffect::Dismiss => "dismiss",
        InputEffect::BoostUp => "boost_up",
        InputEffect::BoostDown => "boost_down",
    }
}

#[cfg(debug_assertions)]
fn selection_label(view: &LauncherView) -> String {
    let Some((start, end)) = view.state.selected_range() else {
        return "none".to_string();
    };
    format!("{start}-{end}")
}

#[cfg(debug_assertions)]
fn token(value: &str) -> String {
    compact(value, 96)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(debug_assertions)]
fn quoted(value: &str) -> String {
    compact(value, 120)
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' {
                c
            } else if c == ' ' {
                ' '
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(debug_assertions)]
fn compact(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(debug_assertions)]
fn px_i32(px: Pixels) -> i32 {
    px.to_f64().round() as i32
}
