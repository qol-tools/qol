//! The control set the shot surfaces share: which circles a surface offers,
//! how each one presents, and which keystroke activates it. A pinned image
//! carries the same controls as the preview it came from, minus the pin.

use gpui::Keystroke;

use crate::capture::actions::ShotAction;
use crate::config::CopyCommand;
use crate::ui::shortcuts::is_standard_copy_chord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceControl {
    Action(ShotAction),
    Edit,
    Pin,
}

impl SurfaceControl {
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Action(action) => action.glyph(),
            Self::Edit => "✎",
            Self::Pin => "◉",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Edit => "Edit",
            Self::Pin => "Pin",
        }
    }

    pub(crate) fn accel(self) -> char {
        match self {
            Self::Action(action) => action.accel(),
            Self::Edit => 'e',
            Self::Pin => 'i',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlSurface {
    Preview,
    Pinned,
}

/// The two copy actions in the order the surfaces present them, the
/// configured default first.
pub(crate) fn copy_actions(default_copy_action: CopyCommand) -> [ShotAction; 2] {
    match default_copy_action {
        CopyCommand::CopyImage => [ShotAction::Copy, ShotAction::CopyPath],
        CopyCommand::CopyPath => [ShotAction::CopyPath, ShotAction::Copy],
    }
}

pub(crate) fn controls(
    surface: ControlSurface,
    default_copy_action: CopyCommand,
) -> Vec<SurfaceControl> {
    let copy = copy_actions(default_copy_action);
    let mut controls = vec![
        SurfaceControl::Action(copy[0]),
        SurfaceControl::Action(copy[1]),
        SurfaceControl::Action(ShotAction::OpenFolder),
        SurfaceControl::Edit,
    ];
    if surface == ControlSurface::Preview {
        controls.push(SurfaceControl::Pin);
    }
    controls
}

/// How many circles `surface` shows; the copy order never changes the count.
pub(crate) fn control_count(surface: ControlSurface) -> usize {
    controls(surface, CopyCommand::CopyImage).len()
}

/// The control `keystroke` activates on `surface`. The platform copy chord
/// activates `standard_copy` (the selected control where a surface has one),
/// every other accelerator maps to the control that owns it.
pub(crate) fn control_for_keystroke(
    keystroke: &Keystroke,
    surface: ControlSurface,
    default_copy_action: CopyCommand,
    standard_copy: SurfaceControl,
) -> Option<SurfaceControl> {
    if is_standard_copy_chord(keystroke) {
        return Some(standard_copy);
    }
    if keystroke.modifiers.modified() {
        return None;
    }

    let mut keys = keystroke.key.chars();
    let accel = keys.next()?;
    if keys.next().is_some() {
        return None;
    }
    controls(surface, default_copy_action)
        .into_iter()
        .find(|control| control.accel() == accel)
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};

    use super::{
        control_count, control_for_keystroke, controls, ControlSurface, ShotAction, SurfaceControl,
    };
    use crate::config::CopyCommand;

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn default_copy_action_only_changes_control_order() {
        let cases = [
            (
                CopyCommand::CopyImage,
                [
                    SurfaceControl::Action(ShotAction::Copy),
                    SurfaceControl::Action(ShotAction::CopyPath),
                    SurfaceControl::Action(ShotAction::OpenFolder),
                    SurfaceControl::Edit,
                    SurfaceControl::Pin,
                ],
            ),
            (
                CopyCommand::CopyPath,
                [
                    SurfaceControl::Action(ShotAction::CopyPath),
                    SurfaceControl::Action(ShotAction::Copy),
                    SurfaceControl::Action(ShotAction::OpenFolder),
                    SurfaceControl::Edit,
                    SurfaceControl::Pin,
                ],
            ),
        ];

        for (copy_command, expected) in cases {
            assert_eq!(controls(ControlSurface::Preview, copy_command), expected);
        }
    }

    #[test]
    fn a_pinned_image_keeps_every_preview_control_but_the_pin() {
        for copy_command in [CopyCommand::CopyImage, CopyCommand::CopyPath] {
            let preview = controls(ControlSurface::Preview, copy_command);
            let pinned = controls(ControlSurface::Pinned, copy_command);

            assert_eq!(
                pinned,
                preview
                    .iter()
                    .copied()
                    .filter(|control| *control != SurfaceControl::Pin)
                    .collect::<Vec<_>>(),
                "copy command: {copy_command:?}"
            );
            assert!(pinned.contains(&SurfaceControl::Edit));
        }
        assert_eq!(control_count(ControlSurface::Preview), 5);
        assert_eq!(control_count(ControlSurface::Pinned), 4);
    }

    #[test]
    fn standard_copy_chord_activates_the_selected_control() {
        let chord = keystroke("c", Modifiers::secondary_key());
        let cases = [
            SurfaceControl::Action(ShotAction::Copy),
            SurfaceControl::Action(ShotAction::CopyPath),
            SurfaceControl::Action(ShotAction::OpenFolder),
            SurfaceControl::Edit,
            SurfaceControl::Pin,
        ];

        for selected in cases {
            assert_eq!(
                control_for_keystroke(
                    &chord,
                    ControlSurface::Preview,
                    CopyCommand::CopyImage,
                    selected
                ),
                Some(selected)
            );
        }
    }

    #[test]
    fn plain_accelerators_keep_their_direct_controls() {
        let cases = [
            ("c", SurfaceControl::Action(ShotAction::Copy)),
            ("p", SurfaceControl::Action(ShotAction::CopyPath)),
            ("o", SurfaceControl::Action(ShotAction::OpenFolder)),
            ("e", SurfaceControl::Edit),
            ("i", SurfaceControl::Pin),
        ];

        for (key, expected) in cases {
            assert_eq!(
                control_for_keystroke(
                    &keystroke(key, Modifiers::none()),
                    ControlSurface::Preview,
                    CopyCommand::CopyPath,
                    SurfaceControl::Pin,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn a_pinned_image_answers_every_accelerator_but_the_pin() {
        let cases = [
            ("c", Some(SurfaceControl::Action(ShotAction::Copy))),
            ("p", Some(SurfaceControl::Action(ShotAction::CopyPath))),
            ("o", Some(SurfaceControl::Action(ShotAction::OpenFolder))),
            ("e", Some(SurfaceControl::Edit)),
            ("i", None),
        ];

        for (key, expected) in cases {
            assert_eq!(
                control_for_keystroke(
                    &keystroke(key, Modifiers::none()),
                    ControlSurface::Pinned,
                    CopyCommand::CopyImage,
                    SurfaceControl::Action(ShotAction::Copy),
                ),
                expected,
                "key: {key}"
            );
        }
    }

    #[test]
    fn named_control_keys_never_trigger_accelerators() {
        let keys = [
            "escape", "esc", "enter", "return", "space", "left", "right", "up", "down", "tab",
        ];

        for key in keys {
            assert_eq!(
                control_for_keystroke(
                    &keystroke(key, Modifiers::none()),
                    ControlSurface::Preview,
                    CopyCommand::CopyPath,
                    SurfaceControl::Edit,
                ),
                None,
                "key: {key}"
            );
        }
    }
}
