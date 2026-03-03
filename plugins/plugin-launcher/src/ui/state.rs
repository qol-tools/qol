use super::layout::{HEADER_HEIGHT, MAX_VISIBLE};
use crate::discovery::search::{Fuzziness, SearchMode};
use std::time::{Duration, Instant};

const NAV_FAST_WINDOW: Duration = Duration::from_millis(95);
const NAV_DECAY_STEP_FAST: Duration = Duration::from_millis(35);
const NAV_DECAY_STEP_SLOW: Duration = Duration::from_millis(90);
const FOCUS_GRAVITY_IDLE: Duration = Duration::from_millis(140);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeHit {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavCues {
    pub decayed_momentum: u8,
    pub previous_selected: Option<usize>,
    pub momentum_signed: i8,
    pub trail_len: usize,
    pub trail_direction: Option<NavDirection>,
}

pub struct LauncherState {
    pub mode: SearchMode,
    pub fuzziness: Fuzziness,
    pub query: String,
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
    pub selected: usize,
    pub previous_selected: Option<usize>,
    pub edge_hit: Option<EdgeHit>,
    pub nav_direction: Option<NavDirection>,
    pub nav_momentum: u8,
    pub nav_decay_step: Duration,
    pub last_nav_at: Option<Instant>,
    pub scroll_offset: usize,
    pub window_height: f32,
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            mode: SearchMode::Apps,
            fuzziness: Fuzziness::Balanced,
            query: String::new(),
            cursor: 0,
            selection_anchor: None,
            selected: 0,
            previous_selected: None,
            edge_hit: None,
            nav_direction: None,
            nav_momentum: 0,
            nav_decay_step: NAV_DECAY_STEP_SLOW,
            last_nav_at: None,
            scroll_offset: 0,
            window_height: HEADER_HEIGHT,
        }
    }

    pub fn query_len(&self) -> usize {
        self.query.chars().count()
    }

    pub fn cycle_mode(&mut self, _reverse: bool) {
        self.mode = self.mode.next();
    }

    pub fn increase_fuzziness(&mut self) -> bool {
        let before = self.fuzziness;
        self.fuzziness = self.fuzziness.more();
        self.fuzziness != before
    }

    pub fn decrease_fuzziness(&mut self) -> bool {
        let before = self.fuzziness;
        self.fuzziness = self.fuzziness.less();
        self.fuzziness != before
    }

    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            None
        } else {
            Some((anchor.min(self.cursor), anchor.max(self.cursor)))
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        let start_b = Self::char_to_byte_index(&self.query, start);
        let end_b = Self::char_to_byte_index(&self.query, end);
        Some(self.query[start_b..end_b].to_string())
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let selected = self.selection_text()?;
        self.delete_selection();
        Some(selected)
    }

    pub fn paste_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }

        self.delete_selection();
        let idx = Self::char_to_byte_index(&self.query, self.cursor);
        self.query.insert_str(idx, text);
        self.cursor += text.chars().count();
        self.clear_selection();
        true
    }

    pub(crate) fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
        s.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(s.len())
    }

    pub(crate) fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selected_range() else {
            return false;
        };
        let start_b = Self::char_to_byte_index(&self.query, start);
        let end_b = Self::char_to_byte_index(&self.query, end);
        self.query.replace_range(start_b..end_b, "");
        self.cursor = start;
        self.clear_selection();
        true
    }

    pub fn reset_results_position(&mut self) {
        self.selected = 0;
        self.previous_selected = None;
        self.edge_hit = None;
        self.nav_direction = None;
        self.nav_momentum = 0;
        self.nav_decay_step = NAV_DECAY_STEP_SLOW;
        self.last_nav_at = None;
        self.scroll_offset = 0;
    }

    pub fn sync_result_window(&mut self, result_count: usize) {
        if result_count == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }

        self.selected = self.selected.min(result_count.saturating_sub(1));
        let max_offset = result_count.saturating_sub(MAX_VISIBLE);
        self.scroll_offset = self.scroll_offset.min(max_offset);

        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
            return;
        }

        let bottom = self.scroll_offset + MAX_VISIBLE.saturating_sub(1);
        if self.selected > bottom {
            self.scroll_offset = self.selected + 1 - MAX_VISIBLE;
        }
    }

    pub fn take_edge_hit(&mut self) -> Option<EdgeHit> {
        self.edge_hit.take()
    }

    pub fn register_nav(&mut self, direction: NavDirection) {
        let now = Instant::now();
        let fast_repeat = self
            .last_nav_at
            .map(|last| now.duration_since(last) <= NAV_FAST_WINDOW)
            .unwrap_or(false);

        let accelerating = fast_repeat && self.nav_direction == Some(direction);
        self.nav_momentum = if accelerating {
            self.nav_momentum.saturating_add(1).min(2)
        } else {
            1
        };
        self.nav_decay_step = if accelerating {
            NAV_DECAY_STEP_FAST
        } else {
            NAV_DECAY_STEP_SLOW
        };
        self.nav_direction = Some(direction);
        self.last_nav_at = Some(now);
    }

    pub fn decayed_momentum(&self) -> u8 {
        let Some(last) = self.last_nav_at else {
            return 0;
        };
        let elapsed = Instant::now().duration_since(last);
        let steps = (elapsed.as_millis() / self.nav_decay_step.as_millis()) as u8;
        self.nav_momentum.saturating_sub(steps)
    }

    pub fn nav_cues(&self) -> NavCues {
        let decayed_momentum = self.decayed_momentum();
        let momentum_signed = match self.nav_direction {
            Some(NavDirection::Down) => decayed_momentum as i8,
            Some(NavDirection::Up) => -(decayed_momentum as i8),
            None => 0,
        };
        let trail_len = match decayed_momentum {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 3,
        };
        let trail_direction = if decayed_momentum > 0 {
            self.nav_direction
        } else {
            None
        };
        let previous_selected = if decayed_momentum > 0 {
            self.previous_selected
        } else {
            None
        };

        NavCues {
            decayed_momentum,
            previous_selected,
            momentum_signed,
            trail_len,
            trail_direction,
        }
    }

    pub fn should_focus_gravity(&self) -> bool {
        self.last_nav_at
            .map(|last| Instant::now().duration_since(last) >= FOCUS_GRAVITY_IDLE)
            .unwrap_or(false)
    }

    pub fn focus_gravity_target(&self, result_count: usize, visible: usize) -> usize {
        if result_count == 0 || visible == 0 {
            return 0;
        }
        let max_offset = result_count.saturating_sub(visible);
        self.selected.saturating_sub(visible / 2).min(max_offset)
    }
}
