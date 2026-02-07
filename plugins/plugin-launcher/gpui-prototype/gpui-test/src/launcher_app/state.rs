use super::layout::HEADER_HEIGHT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Apps,
}

impl SearchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
        }
    }
}

pub struct LauncherState {
    pub mode: SearchMode,
    pub query: String,
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
    pub selected: usize,
    pub window_height: f32,
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            mode: SearchMode::Apps,
            query: String::new(),
            cursor: 0,
            selection_anchor: None,
            selected: 0,
            window_height: HEADER_HEIGHT,
        }
    }

    pub fn query_len(&self) -> usize {
        self.query.chars().count()
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
}
