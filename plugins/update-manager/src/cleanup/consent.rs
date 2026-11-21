use super::{Leftover, Origin, WayBack};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consent {
    All,
    Asked,
}

pub(super) fn allows(leftover: &Leftover, consent: Consent) -> Result<(), String> {
    match (consent, why(leftover)) {
        (Consent::All, Some(why)) => Err(why),
        _ => Ok(()),
    }
}

pub(crate) fn why(leftover: &Leftover) -> Option<String> {
    match (leftover.way_back, leftover.origin) {
        (WayBack::Last { to }, _) => Some(format!("are the last way back to {to}")),
        (_, Origin::ByHand) => Some("were installed by hand".into()),
        (WayBack::None, Origin::System) => leftover.along.iter().find_map(|along| {
            why(along).map(|why| format!("take {} along, which {why}", along.name.to_lowercase()))
        }),
    }
}
