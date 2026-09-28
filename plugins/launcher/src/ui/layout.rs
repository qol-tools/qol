use crate::discovery::search::SearchMode;

pub const MAX_VISIBLE: usize = 8;
pub const FILES_VISIBLE: usize = 5;
pub const HEADER_HEIGHT: f32 = 52.0;
pub const FLOW_ROW_HEIGHT: f32 = 44.0;
pub const LIST_PAD_Y: f32 = 6.0;
pub const APP_CARD_MIN: f32 = 60.0;
pub const APP_CARD_MAX: f32 = 70.0;
pub const APP_GAP: f32 = 8.0;
pub const APP_PANEL_WIDTH: f32 = 320.0;
pub const FILE_CARD_MAX: f32 = 105.0;
pub const FILE_GAP: f32 = 6.0;
pub const FILE_INSET: f32 = 8.0;
pub const PANEL_WIDTH: f32 = 212.0;
pub const FILES_PANEL_MIN: f32 = 212.0;
pub const PANEL_GAP: f32 = 16.0;
pub const FLOW_WIDTH: f32 = 580.0;
pub const WINDOW_WIDTH: f32 = 840.0;
pub const DETAIL_HEIGHT: f32 = 380.0;
const FRAME: f32 = 2.0 * qol_gpui::theme::LINE;

pub fn visible_rows(mode: SearchMode) -> usize {
    match mode {
        SearchMode::Apps => MAX_VISIBLE,
        SearchMode::Files => FILES_VISIBLE,
    }
}

pub fn content_width(flow: bool) -> f32 {
    if flow {
        FLOW_WIDTH
    } else {
        WINDOW_WIDTH
    }
}

pub fn list_gap(mode: SearchMode) -> f32 {
    match mode {
        SearchMode::Apps => APP_GAP,
        SearchMode::Files => FILE_GAP,
    }
}

pub fn header_window_height() -> f32 {
    HEADER_HEIGHT + FRAME
}

pub fn list_height(heights: &[f32], gap: f32) -> f32 {
    if heights.is_empty() {
        return 0.0;
    }
    2.0 * LIST_PAD_Y + heights.iter().sum::<f32>() + gap * (heights.len() - 1) as f32
}

pub fn window_height_for_list(heights: &[f32], gap: f32) -> f32 {
    header_window_height() + list_height(heights, gap)
}

pub fn window_height_with_panel(heights: &[f32], gap: f32, panel: f32) -> f32 {
    let list = list_height(heights, gap);
    let beside = if panel > 0.0 {
        panel + 2.0 * LIST_PAD_Y
    } else {
        0.0
    };
    header_window_height() + list.max(beside)
}

pub fn window_height_for(visible_rows: usize, row_height: f32) -> f32 {
    window_height_for_list(&vec![row_height; visible_rows], 0.0) + qol_gpui::theme::HEIGHT_HINT_BAR
}

pub fn full_window_height() -> f32 {
    let apps = window_height_for_list(&[APP_CARD_MAX; MAX_VISIBLE], APP_GAP);
    let files = window_height_for_list(&[FILE_CARD_MAX; FILES_VISIBLE], FILE_GAP);
    apps.max(files)
        .max(window_height_for_trail(
            true,
            qol_gpui::trail::motion::ROW_H,
        ))
        .max(window_height_for_detail())
}

pub fn window_height_for_trail(fence: bool, head_height: f32) -> f32 {
    header_window_height()
        + qol_gpui::trail::motion::viewport_height(head_height)
        + if fence { FLOW_ROW_HEIGHT } else { 0.0 }
        + qol_gpui::theme::HEIGHT_HINT_BAR
}

pub fn window_height_for_detail() -> f32 {
    header_window_height() + DETAIL_HEIGHT + qol_gpui::theme::HEIGHT_HINT_BAR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::sizing;
    use crate::ui::view::CARD_HEIGHT;
    use qol_gpui::trail::motion::ROW_H;

    #[test]
    fn full_window_holds_every_layout() {
        for (fence, head) in [(true, ROW_H), (false, ROW_H), (false, CARD_HEIGHT)] {
            assert!(window_height_for_trail(fence, head) <= full_window_height());
        }
        assert_eq!(sizing::app_card(1.0).height, APP_CARD_MAX);
        assert_eq!(sizing::app_card(0.0).height, APP_CARD_MIN);
        assert_eq!(sizing::file_card(1.0).height, FILE_CARD_MAX);
    }

    #[test]
    fn heights_match_the_approved_board() {
        let cards = [70.0, 67.0, 60.0];
        assert_eq!(window_height_for_list(&cards, APP_GAP), 279.0);
        assert_eq!(window_height_with_panel(&cards, APP_GAP, 400.0), 466.0);
        assert_eq!(window_height_with_panel(&cards, APP_GAP, 100.0), 279.0);
        assert_eq!(window_height_for_list(&[], 0.0), 54.0);
        let inner = WINDOW_WIDTH - 2.0 * qol_gpui::theme::LINE - 2.0 * FILE_INSET;
        assert_eq!(inner - PANEL_GAP - PANEL_WIDTH, 594.0);
        assert_eq!(inner - APP_GAP - APP_PANEL_WIDTH, 494.0);
    }
}
