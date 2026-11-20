use super::BackendId;
use crate::platform::engine::config::KeyBackendChoice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Availability {
    Ready,
    Missing { what: String, fix: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reason {
    Chosen,
    Auto,
    FellBack {
        wanted: BackendId,
        what: String,
        fix: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) backend: BackendId,
    pub(crate) reason: Reason,
}

impl Selection {
    pub(crate) fn describe(&self) -> String {
        let backend = self.backend.label();
        match &self.reason {
            Reason::Chosen => format!("{backend}, as chosen"),
            Reason::Auto => format!("{backend}, picked by auto"),
            Reason::FellBack { wanted, what, fix } => {
                format!("{backend}, because {} needs {what}: {fix}", wanted.label())
            }
        }
    }
}

pub(crate) fn wanted(choice: KeyBackendChoice) -> Option<BackendId> {
    match choice {
        KeyBackendChoice::Auto => None,
        KeyBackendChoice::EventTap => Some(BackendId::EventTap),
        KeyBackendChoice::HidHelper => Some(BackendId::HidHelper),
        KeyBackendChoice::KarabinerElements => Some(BackendId::KarabinerElements),
    }
}

pub(crate) fn helper_allowed(choice: KeyBackendChoice) -> bool {
    matches!(choice, KeyBackendChoice::Auto | KeyBackendChoice::HidHelper)
}

pub(crate) fn select(
    choice: KeyBackendChoice,
    probe: impl Fn(BackendId) -> Availability,
) -> Selection {
    let Some(wanted) = wanted(choice) else {
        let helper = probe(BackendId::HidHelper) == Availability::Ready;
        return Selection {
            backend: if helper {
                BackendId::HidHelper
            } else {
                BackendId::EventTap
            },
            reason: Reason::Auto,
        };
    };
    if wanted == BackendId::EventTap {
        return Selection {
            backend: wanted,
            reason: Reason::Chosen,
        };
    }
    match probe(wanted) {
        Availability::Ready => Selection {
            backend: wanted,
            reason: Reason::Chosen,
        },
        Availability::Missing { what, fix } => Selection {
            backend: BackendId::EventTap,
            reason: Reason::FellBack { wanted, what, fix },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(what: &str) -> Availability {
        Availability::Missing {
            what: what.to_string(),
            fix: format!("install {what}"),
        }
    }

    fn probe(helper: bool, karabiner: bool) -> impl Fn(BackendId) -> Availability {
        move |backend| match backend {
            BackendId::EventTap => panic!("the event tap needs no probe"),
            BackendId::HidHelper if helper => Availability::Ready,
            BackendId::HidHelper => missing("the keyboard helper"),
            BackendId::KarabinerElements if karabiner => Availability::Ready,
            BackendId::KarabinerElements => missing("Karabiner-Elements"),
        }
    }

    #[test]
    fn every_choice_against_every_probe() {
        use BackendId::{EventTap, HidHelper, KarabinerElements};
        use KeyBackendChoice as Choice;
        let cases = [
            (Choice::Auto, false, false, EventTap),
            (Choice::Auto, true, false, HidHelper),
            (Choice::Auto, false, true, EventTap),
            (Choice::Auto, true, true, HidHelper),
            (Choice::EventTap, true, true, EventTap),
            (Choice::HidHelper, true, false, HidHelper),
            (Choice::HidHelper, false, true, EventTap),
            (Choice::KarabinerElements, false, true, KarabinerElements),
            (Choice::KarabinerElements, true, false, EventTap),
            (Choice::KarabinerElements, false, false, EventTap),
        ];
        for (choice, helper, karabiner, expected) in cases {
            assert_eq!(
                select(choice, probe(helper, karabiner)).backend,
                expected,
                "{choice:?} with helper={helper} karabiner={karabiner}"
            );
        }
    }

    #[test]
    fn auto_never_switches_to_karabiner_on_its_own() {
        let selection = select(KeyBackendChoice::Auto, probe(false, true));
        assert_eq!(selection.reason, Reason::Auto);
        assert_eq!(selection.backend, BackendId::EventTap);
    }

    #[test]
    fn a_fallback_names_what_is_missing_and_the_fix() {
        let selection = select(KeyBackendChoice::KarabinerElements, probe(true, false));
        assert_eq!(
            selection.reason,
            Reason::FellBack {
                wanted: BackendId::KarabinerElements,
                what: "Karabiner-Elements".to_string(),
                fix: "install Karabiner-Elements".to_string(),
            }
        );
        let text = selection.describe();
        assert!(text.starts_with("event_tap"), "{text}");
        assert!(
            text.contains("karabiner_elements needs Karabiner-Elements"),
            "{text}"
        );
    }

    #[test]
    fn only_auto_and_the_helper_choice_let_the_helper_seize() {
        assert!(helper_allowed(KeyBackendChoice::Auto));
        assert!(helper_allowed(KeyBackendChoice::HidHelper));
        assert!(!helper_allowed(KeyBackendChoice::EventTap));
        assert!(!helper_allowed(KeyBackendChoice::KarabinerElements));
    }
}
