use crate::icon::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionMark {
    Page,
    Leave,
    Add,
    More,
    Busy,
    Chosen,
    Close,
    Copy,
}

impl ActionMark {
    pub const ALL: [Self; 8] = [
        Self::Page,
        Self::Leave,
        Self::Add,
        Self::More,
        Self::Busy,
        Self::Chosen,
        Self::Close,
        Self::Copy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Leave => "leave",
            Self::Add => "add",
            Self::More => "more",
            Self::Busy => "busy",
            Self::Chosen => "chosen",
            Self::Close => "close",
            Self::Copy => "copy",
        }
    }

    pub fn meaning(self) -> &'static str {
        match self {
            Self::Page => "opens something: a page, a list or, from a settings row, another app",
            Self::Leave => "shows it in another app, on a surface that is not a settings row",
            Self::Add => "adds one",
            Self::More => "opens the menu of what else it can do",
            Self::Busy => "is working on it",
            Self::Chosen => "is the one in use",
            Self::Close => "closes or dismisses",
            Self::Copy => "copies it to the clipboard, on a surface that is not a settings row",
        }
    }

    pub fn icon(self) -> Option<Icon> {
        match self {
            Self::Close => Some(Icon::Close),
            Self::Add => Some(Icon::Add),
            Self::More => Some(Icon::More),
            Self::Chosen => Some(Icon::Tick),
            Self::Copy => Some(Icon::Copy),
            Self::Leave => Some(Icon::Reveal),
            Self::Page | Self::Busy => None,
        }
    }

    pub fn drawn_by(self) -> &'static [&'static str] {
        match self {
            Self::Page => &["pictures::chevron"],
            Self::Leave => &["Icon::Reveal"],
            Self::Close => &["Icon::Close"],
            Self::Add => &["Icon::Add"],
            Self::More => &["Icon::More"],
            Self::Busy => &["Busy::ring", "Busy::new"],
            Self::Chosen => &["Icon::Tick", "pictures::tick"],
            Self::Copy => &["Icon::Copy"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ActionMark;
    use std::collections::HashSet;

    #[test]
    fn every_mark_has_its_own_name() {
        let names: HashSet<_> = ActionMark::ALL.iter().map(|mark| mark.name()).collect();
        assert_eq!(names.len(), ActionMark::ALL.len());
    }

    #[test]
    fn every_mark_is_drawn_by_something() {
        for mark in ActionMark::ALL {
            assert!(!mark.drawn_by().is_empty(), "{}", mark.name());
        }
    }
}
