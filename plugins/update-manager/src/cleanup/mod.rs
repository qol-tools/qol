use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::SystemTime;

use chrono::NaiveDateTime;

use crate::platform::Refusal;

mod consent;
mod measure;
mod platform;

pub(crate) use consent::{why, Consent};
pub(crate) use platform::{leftovers, stamp};

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
        dir: &'static str,
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
    Command(&'static [&'static str]),
    Packages,
    Script(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WayBack {
    None,
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

impl Why {
    pub(crate) fn reason(&self) -> String {
        match self {
            Why::NeedsAdmin => "needs an administrator to measure".into(),
            Why::Cannot(reason) => (*reason).into(),
            Why::Failed(reason) => format!("could not measure: {reason}"),
        }
    }
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
    pub(crate) present: usize,
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
    pub(crate) stamp: Vec<Option<SystemTime>>,
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

impl PartialEq for Leftover {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Leftover {}

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
            present: 0,
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

    pub(crate) fn failed(&self) -> bool {
        self.found.iter().any(|found| {
            matches!(
                found.amount.as_ref().map(|amount| &amount.size),
                Some(Size::Unknown(Why::Failed(_)))
            )
        })
    }

    pub(crate) fn total(&self) -> Total {
        let mut amounts: Vec<&Amount> = self
            .found
            .iter()
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
                let counted: u64 = amount
                    .items
                    .iter()
                    .filter(|item| seen.contains(item.name.as_str()))
                    .filter_map(|item| item.bytes)
                    .sum();
                total.bytes += bytes.saturating_sub(counted);
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

pub(crate) fn switch(id: &str) -> Option<&'static Switch> {
    named(platform::switches(), id)
}

fn named(switches: &'static [Switch], id: &str) -> Option<&'static Switch> {
    let mut rows = switches.iter().filter(|switch| switch.id == id).peekable();
    let first = rows.peek().copied();
    rows.find(|switch| switch.requires.missing().is_none())
        .or(first)
}

pub(crate) fn read_switches() -> Vec<Live> {
    live(platform::switches())
}

fn live(switches: &'static [Switch]) -> Vec<Live> {
    let mut live: Vec<Live> = Vec::new();
    for switch in switches {
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

pub(crate) fn set_switch(switch: &Switch, on: bool) -> Result<(), Refusal> {
    set_live(&read_switches(), switch, on)
}

fn set_live(live: &[Live], switch: &Switch, on: bool) -> Result<(), Refusal> {
    let live = live
        .iter()
        .find(|live| live.switch.id == switch.id)
        .ok_or_else(|| Refusal::Failed(format!("{} is not on this computer", switch.name)))?;
    (live.switch.set)(on)
}

pub(crate) fn primary(live: &[Live]) -> Option<&Live> {
    live.iter().find(|live| live.switch.covers(Origin::System))
}

pub(crate) fn find(leftover: &'static Leftover) -> Found {
    find_by(leftover, &mut |argv| platform::output(argv))
}

fn find_by(leftover: &'static Leftover, output: &mut measure::Output) -> Found {
    let amount = leftover
        .requires
        .missing()
        .is_none()
        .then(|| measure::measure_by(&leftover.measure, output));
    Found { leftover, amount }
}

pub(crate) fn survey(at: NaiveDateTime) -> Survey {
    let mut output = measure::once(platform::output);
    Survey {
        at,
        stamp: stamp(),
        found: leftovers()
            .iter()
            .copied()
            .map(|leftover| find_by(leftover, &mut output))
            .collect(),
    }
}

pub(crate) fn empty(found: &Found) -> bool {
    found
        .amount
        .as_ref()
        .is_none_or(|amount| amount.count == 0 && !matches!(amount.size, Size::Unknown(_)))
}

pub(crate) fn sweep(kinds: &[&'static Leftover], consent: Consent) -> Result<(), Refusal> {
    sweep_by(kinds, consent, find, remove)
}

fn sweep_by(
    kinds: &[&'static Leftover],
    consent: Consent,
    find: impl Fn(&'static Leftover) -> Found,
    remove: impl Fn(&Found, Consent) -> Result<(), Refusal>,
) -> Result<(), Refusal> {
    let mut failed = Vec::new();
    for kind in kinds.iter().copied() {
        let found = find(kind);
        if empty(&found) {
            continue;
        }
        match remove(&found, consent) {
            Ok(()) => {}
            Err(refusal @ (Refusal::Dismissed | Refusal::Denied | Refusal::TimedOut)) => {
                failed.push(format!(
                    "{}: {refusal}, so nothing more was removed",
                    kind.name
                ));
                break;
            }
            Err(refusal) => failed.push(format!("{}: {refusal}", kind.name)),
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Refusal::Failed(failed.join(". ")))
    }
}

pub(crate) fn remove(found: &Found, consent: Consent) -> Result<(), Refusal> {
    remove_by(found, consent, platform::run)
}

fn remove_by(
    found: &Found,
    consent: Consent,
    run: impl FnOnce(&Remove, &[String], bool) -> Result<(), Refusal>,
) -> Result<(), Refusal> {
    let leftover = found.leftover;
    let Some(amount) = found.amount.as_ref() else {
        return Err(Refusal::Failed("they are not on this computer".into()));
    };
    consent::allows(leftover, consent).map_err(|why| {
        Refusal::Failed(format!(
            "they {why}, so they are removed only when confirmed"
        ))
    })?;
    let unlisted = |why: &Why| {
        Err(Refusal::Failed(format!(
            "they could not be listed, so nothing was removed: {}",
            why.reason()
        )))
    };
    if let Size::Unknown(why @ Why::Failed(_)) = &amount.size {
        return unlisted(why);
    }
    let items: Vec<String> = amount.items.iter().map(|item| item.name.clone()).collect();
    let takes_items = matches!(leftover.remove, Remove::Packages | Remove::Script(_));
    if takes_items && items.is_empty() {
        return match &amount.size {
            Size::Unknown(why) => unlisted(why),
            Size::Exact(_) | Size::Estimate { .. } => Ok(()),
        };
    }
    run(&leftover.remove, &items, leftover.admin)
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;
