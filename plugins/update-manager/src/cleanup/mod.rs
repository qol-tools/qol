use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::Path;

use chrono::NaiveDateTime;

use crate::updates::Refusal;

mod consent;
mod measure;
mod platform;

pub(crate) use consent::{asks, Consent};
pub(crate) use platform::leftovers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Noun {
    pub(crate) one: &'static str,
    pub(crate) many: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requires {
    Program(&'static str),
    Path(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Measure {
    Files {
        dirs: &'static [&'static str],
        suffix: &'static str,
    },
    Command {
        argv: &'static [&'static str],
        read: fn(&str) -> Result<Amount, String>,
    },
    Unknown(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Remove {
    Command {
        argv: &'static [&'static str],
        items: bool,
    },
    Script(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WayBack {
    None,
    Keeps,
    Last { to: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    System,
    ByHand,
}

#[derive(Debug)]
pub(crate) struct Leftover {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) what: &'static str,
    pub(crate) noun: Noun,
    pub(crate) requires: Requires,
    pub(crate) measure: Measure,
    pub(crate) remove: Remove,
    pub(crate) admin: bool,
    pub(crate) way_back: WayBack,
    pub(crate) origin: Origin,
    pub(crate) along: &'static [&'static Leftover],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Owner {
    pub(crate) title: &'static str,
    pub(crate) colophon: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Words {
    pub(crate) on: &'static str,
    pub(crate) idle: &'static str,
    pub(crate) off: &'static str,
}

#[derive(Debug)]
pub(crate) struct Switch {
    pub(crate) id: &'static str,
    pub(crate) owner: Owner,
    pub(crate) name: &'static str,
    pub(crate) short: &'static str,
    pub(crate) doing: &'static str,
    pub(crate) when: &'static str,
    pub(crate) words: Words,
    pub(crate) covers: &'static [&'static Leftover],
    pub(crate) requires: Requires,
    pub(crate) read: fn() -> Option<State>,
    pub(crate) set: fn(bool) -> Result<(), Refusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Unknown(&'static str),
    Off,
    On { idle: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Live {
    pub(crate) switch: &'static Switch,
    pub(crate) state: State,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Why {
    NeedsAdmin,
    Cannot(&'static str),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Size {
    Exact(u64),
    Estimate { bytes: u64, why: &'static str },
    Unknown(Why),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) name: String,
    pub(crate) bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Amount {
    pub(crate) count: usize,
    pub(crate) size: Size,
    pub(crate) items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub(crate) leftover: &'static Leftover,
    pub(crate) amount: Option<Amount>,
}

#[derive(Debug, Clone)]
pub(crate) struct Survey {
    pub(crate) at: NaiveDateTime,
    pub(crate) found: Vec<Found>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Total {
    pub(crate) bytes: u64,
    pub(crate) estimate: bool,
    pub(crate) unknown: usize,
}

impl Requires {
    pub(crate) fn missing(self) -> Option<String> {
        match self {
            Self::Program(program) => {
                (!platform::found(program)).then(|| format!("{program} is not installed"))
            }
            Self::Path(path) => (!Path::new(path).exists()).then(|| format!("{path} is not here")),
        }
    }
}

impl PartialEq for Switch {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Switch {}

impl Hash for Switch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PartialOrd for Switch {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Switch {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(other.id)
    }
}

impl Switch {
    pub(crate) fn covers(&self, origin: Origin) -> bool {
        self.covers.iter().any(|kind| kind.origin == origin)
    }
}

impl State {
    pub(crate) fn on(self) -> bool {
        matches!(self, Self::On { .. })
    }

    pub(crate) fn removing(self) -> bool {
        self == Self::On { idle: false }
    }
}

impl Amount {
    pub(crate) fn unknown(why: Why) -> Self {
        Self {
            count: 0,
            size: Size::Unknown(why),
            items: Vec::new(),
        }
    }

    fn itemised(&self) -> bool {
        self.items.iter().all(|item| item.bytes.is_some())
    }
}

impl Survey {
    pub(crate) fn amount(&self, leftover: &Leftover) -> Option<&Amount> {
        self.found
            .iter()
            .find(|found| found.leftover.id == leftover.id)?
            .amount
            .as_ref()
    }

    pub(crate) fn total(&self, take: impl Fn(&Leftover) -> bool) -> Total {
        let mut amounts: Vec<&Amount> = self
            .found
            .iter()
            .filter(|found| take(found.leftover))
            .filter_map(|found| found.amount.as_ref())
            .collect();
        amounts.sort_by_key(|amount| amount.itemised());
        let mut seen = BTreeSet::new();
        let mut total = Total::default();
        for amount in amounts {
            let bytes = match &amount.size {
                Size::Unknown(_) => {
                    total.unknown += 1;
                    continue;
                }
                Size::Exact(bytes) => *bytes,
                Size::Estimate { bytes, .. } => {
                    total.estimate = true;
                    *bytes
                }
            };
            if !amount.itemised() {
                total.bytes += bytes;
                seen.extend(amount.items.iter().map(|item| item.name.as_str()));
                continue;
            }
            for item in &amount.items {
                if seen.insert(item.name.as_str()) {
                    total.bytes += item.bytes.unwrap_or(0);
                }
            }
        }
        total
    }
}

pub(crate) fn leftover(id: &str) -> Option<&'static Leftover> {
    leftovers()
        .iter()
        .copied()
        .find(|leftover| leftover.id == id)
}

pub(crate) fn switch(id: &str) -> Option<&'static Switch> {
    let mut named = platform::switches()
        .iter()
        .filter(|switch| switch.id == id)
        .peekable();
    let first = named.peek().copied();
    named
        .find(|switch| switch.requires.missing().is_none())
        .or(first)
}

pub(crate) fn read_switches() -> Vec<Live> {
    let mut live: Vec<Live> = Vec::new();
    for switch in platform::switches() {
        if live.iter().any(|known| known.switch.id == switch.id)
            || switch.requires.missing().is_some()
        {
            continue;
        }
        if let Some(state) = (switch.read)() {
            live.push(Live { switch, state });
        }
    }
    live
}

pub(crate) fn primary(live: &[Live]) -> Option<&Live> {
    live.iter().find(|live| live.switch.covers(Origin::System))
}

pub(crate) fn find(leftover: &'static Leftover) -> Found {
    let amount = leftover
        .requires
        .missing()
        .is_none()
        .then(|| measure::measure(&leftover.measure));
    Found { leftover, amount }
}

pub(crate) fn survey(at: NaiveDateTime) -> Survey {
    Survey {
        at,
        found: leftovers().iter().copied().map(find).collect(),
    }
}

pub(crate) fn remove(found: &Found, consent: Consent) -> Result<(), Refusal> {
    let leftover = found.leftover;
    let Some(amount) = found.amount.as_ref() else {
        return Err(Refusal::Failed(format!(
            "{} are not on this computer",
            leftover.name
        )));
    };
    if !consent::allows(leftover, consent) {
        return Err(Refusal::Failed(format!(
            "{} {}, so they are removed only when asked for by name",
            leftover.name,
            consent::why(leftover)
        )));
    }
    let items: Vec<String> = amount.items.iter().map(|item| item.name.clone()).collect();
    let takes_items = matches!(leftover.remove, Remove::Command { items: true, .. })
        || matches!(leftover.remove, Remove::Script(_));
    if takes_items && items.is_empty() {
        return Ok(());
    }
    platform::run(&leftover.remove, &items, leftover.admin)
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;
