use crate::glide::Direction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowAction {
    SnapLeft,
    SnapRight,
    SnapBottom,
    Maximize,
    Minimize,
    Restore,
    Center,
    MoveMonitorLeft,
    MoveMonitorRight,
    Glide(Direction),
    PrintWorkAreas,
}

impl WindowAction {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::SnapLeft => "snap-left",
            Self::SnapRight => "snap-right",
            Self::SnapBottom => "snap-bottom",
            Self::Maximize => "maximize",
            Self::Minimize => "minimize",
            Self::Restore => "restore",
            Self::Center => "center",
            Self::MoveMonitorLeft => "move-monitor-left",
            Self::MoveMonitorRight => "move-monitor-right",
            Self::Glide(Direction::Left) => "glide-left",
            Self::Glide(Direction::Right) => "glide-right",
            Self::Glide(Direction::Up) => "glide-up",
            Self::Glide(Direction::Down) => "glide-down",
            Self::PrintWorkAreas => "print-work-areas",
        }
    }

    pub(crate) fn unknown_message(self) -> String {
        format!("Unknown action: {}", self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, WindowAction};

    #[test]
    fn ids_are_unique_and_kebab_case() {
        let actions = [
            WindowAction::SnapLeft,
            WindowAction::SnapRight,
            WindowAction::SnapBottom,
            WindowAction::Maximize,
            WindowAction::Minimize,
            WindowAction::Restore,
            WindowAction::Center,
            WindowAction::MoveMonitorLeft,
            WindowAction::MoveMonitorRight,
            WindowAction::Glide(Direction::Left),
            WindowAction::Glide(Direction::Right),
            WindowAction::Glide(Direction::Up),
            WindowAction::Glide(Direction::Down),
            WindowAction::PrintWorkAreas,
        ];
        let mut ids: Vec<&str> = actions.iter().map(|action| action.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), actions.len());
        for id in ids {
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{id}"
            );
        }
    }

    #[test]
    fn unknown_message_names_the_action_id() {
        assert_eq!(
            WindowAction::Glide(Direction::Left).unknown_message(),
            "Unknown action: glide-left"
        );
        assert_eq!(
            WindowAction::PrintWorkAreas.unknown_message(),
            "Unknown action: print-work-areas"
        );
    }
}
