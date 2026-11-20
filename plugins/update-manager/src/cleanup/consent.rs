use super::{Leftover, Origin, WayBack};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consent {
    All,
    Asked,
}

pub(crate) fn asks(leftover: &Leftover) -> bool {
    why_alone(leftover).is_some() || leftover.along.iter().any(|along| asks(along))
}

pub(super) fn allows(leftover: &Leftover, consent: Consent) -> bool {
    consent == Consent::Asked || !asks(leftover)
}

pub(super) fn why(leftover: &Leftover) -> String {
    if let Some(why) = why_alone(leftover) {
        return why;
    }
    leftover
        .along
        .iter()
        .find_map(|along| {
            why_alone(along)
                .map(|why| format!("take {} along, which {why}", along.name.to_lowercase()))
        })
        .unwrap_or_default()
}

fn why_alone(leftover: &Leftover) -> Option<String> {
    match (leftover.way_back, leftover.origin) {
        (WayBack::Last { to }, _) => Some(format!("are the last way back to {to}")),
        (_, Origin::ByHand) => Some("were installed by hand".into()),
        _ => None,
    }
}
