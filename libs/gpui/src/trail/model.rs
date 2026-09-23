use gpui::SharedString;

const PREVIEW_MAX_BYTES: usize = 400;

fn single_line_preview(text: SharedString) -> SharedString {
    let has_newline = text.contains(['\n', '\r']);
    if !has_newline && text.len() <= PREVIEW_MAX_BYTES {
        return text;
    }
    let mut collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > PREVIEW_MAX_BYTES {
        let mut cut = PREVIEW_MAX_BYTES;
        while !collapsed.is_char_boundary(cut) {
            cut -= 1;
        }
        collapsed.truncate(cut);
    }
    collapsed.into()
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrailItem {
    pub at: SharedString,
    pub tag: SharedString,
    pub text: SharedString,
    pub struck: bool,
}

impl TrailItem {
    pub fn new(
        at: impl Into<SharedString>,
        tag: impl Into<SharedString>,
        text: impl Into<SharedString>,
    ) -> Self {
        Self {
            at: at.into(),
            tag: tag.into(),
            text: single_line_preview(text.into()),
            struck: false,
        }
    }

    pub fn struck(mut self, struck: bool) -> Self {
        self.struck = struck;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{single_line_preview, TrailItem, PREVIEW_MAX_BYTES};

    #[test]
    fn preview_collapses_newlines_and_whitespace_runs() {
        let item = TrailItem::new("now", "note", "first line\nsecond\r\n  indented\t\ttail");
        assert_eq!(item.text.as_ref(), "first line second indented tail");
    }

    #[test]
    fn preview_leaves_short_single_line_text_untouched() {
        let text = "already a single line";
        assert_eq!(single_line_preview(text.into()).as_ref(), text);
    }

    #[test]
    fn preview_caps_length_on_a_char_boundary() {
        let long = "é".repeat(PREVIEW_MAX_BYTES);
        let preview = single_line_preview(long.into());
        assert!(preview.len() <= PREVIEW_MAX_BYTES);
        assert!(preview.as_ref().chars().all(|c| c == 'é'));
    }
}
