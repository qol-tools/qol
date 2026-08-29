use gpui::SharedString;

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
            text: text.into(),
            struck: false,
        }
    }

    pub fn struck(mut self, struck: bool) -> Self {
        self.struck = struck;
        self
    }
}
