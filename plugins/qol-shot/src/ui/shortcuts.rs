use gpui::{Keystroke, Modifiers};

use crate::capture::actions::ShotAction;
use crate::config::CopyCommand;

pub(crate) fn resolve_copy_command(command: CopyCommand) -> ShotAction {
    match command {
        CopyCommand::CopyImage => ShotAction::Copy,
        CopyCommand::CopyPath => ShotAction::CopyPath,
    }
}

pub(crate) fn shot_action_for_keystroke(
    keystroke: &Keystroke,
    copy_command: ShotAction,
) -> Option<ShotAction> {
    if keystroke.key.eq_ignore_ascii_case("c") && keystroke.modifiers == Modifiers::secondary_key()
    {
        return Some(copy_command);
    }
    if keystroke.modifiers.modified() {
        return None;
    }

    let accel = keystroke.key.chars().next()?;
    ShotAction::ALL
        .iter()
        .copied()
        .find(|action| action.accel() == accel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn standard_copy_chord_uses_configured_action() {
        let chord = keystroke("c", Modifiers::secondary_key());

        assert_eq!(
            shot_action_for_keystroke(&chord, ShotAction::CopyPath),
            Some(ShotAction::CopyPath)
        );
    }

    #[test]
    fn plain_accelerators_keep_their_direct_actions() {
        assert_eq!(
            shot_action_for_keystroke(&keystroke("c", Modifiers::none()), ShotAction::CopyPath),
            Some(ShotAction::Copy)
        );
        assert_eq!(
            shot_action_for_keystroke(&keystroke("p", Modifiers::none()), ShotAction::Copy),
            Some(ShotAction::CopyPath)
        );
        assert_eq!(
            shot_action_for_keystroke(&keystroke("o", Modifiers::none()), ShotAction::Copy),
            Some(ShotAction::OpenFolder)
        );
    }

    #[test]
    fn unrelated_modified_accelerators_are_ignored() {
        assert_eq!(
            shot_action_for_keystroke(
                &keystroke("p", Modifiers::secondary_key()),
                ShotAction::Copy
            ),
            None
        );
    }
}
