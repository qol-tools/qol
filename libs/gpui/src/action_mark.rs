use crate::icon::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionMark {
    Page,
    Leave,
    Run,
    Destroy,
    Add,
    More,
    Busy,
    Chosen,
    Close,
    Copy,
}

impl ActionMark {
    pub const ALL: [Self; 10] = [
        Self::Page,
        Self::Leave,
        Self::Run,
        Self::Destroy,
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
            Self::Run => "run",
            Self::Destroy => "destroy",
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
            Self::Page => "goes into a page in qol",
            Self::Leave => "leaves qol for another app or a web page",
            Self::Run => "does it now, right here",
            Self::Destroy => "destroys something, after one ask in the hint bar",
            Self::Add => "adds one",
            Self::More => "opens the menu of what else it can do",
            Self::Busy => "is working on it",
            Self::Chosen => "is the one in use",
            Self::Close => "closes or dismisses",
            Self::Copy => "copies the value to the clipboard",
        }
    }

    pub fn for_variant(variant: Option<&str>) -> Self {
        if variant == Some("danger") {
            Self::Destroy
        } else {
            Self::Run
        }
    }

    pub fn icon(self) -> Option<Icon> {
        match self {
            Self::Run => Some(Icon::Enter),
            Self::Destroy | Self::Close => Some(Icon::Close),
            Self::Add => Some(Icon::Add),
            Self::More => Some(Icon::More),
            Self::Chosen => Some(Icon::Tick),
            Self::Copy => Some(Icon::Copy),
            Self::Page | Self::Leave | Self::Busy => None,
        }
    }

    pub fn is_danger(self) -> bool {
        matches!(self, Self::Destroy)
    }

    pub fn drawn_by(self) -> &'static [&'static str] {
        match self {
            Self::Page => &["pictures::chevron"],
            Self::Leave => &["pictures::open_outside", "Icon::Reveal"],
            Self::Run => &["Icon::Enter"],
            Self::Destroy | Self::Close => &["Icon::Close"],
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

    #[test]
    fn only_destroy_is_red() {
        let red: Vec<_> = ActionMark::ALL
            .into_iter()
            .filter(|mark| mark.is_danger())
            .collect();
        assert_eq!(red, [ActionMark::Destroy]);
    }
}
