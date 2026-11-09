use chrono::{DateTime, Local, TimeDelta};
use qol_config::pages::{Group, Page, Pages, Row};

use crate::config::ServerConfig;

pub(crate) const ROOT: &str = "main";
pub(crate) const PAIR: &str = "pair";
const SERVER: &str = "server";
const LIST: &str = "pointzerver";
const APP: &str = "app";
const STOP: &str = "stop";
const REMOVE: &str = "remove";
const PHONE: &str = "phone:";
const JUST_NOW: TimeDelta = TimeDelta::minutes(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Serving {
    Running,
    Off,
    PortBusy(u16),
    Failed(u16),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Phone {
    pub id: String,
    pub name: String,
    pub paired_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pairing {
    pub code: String,
    pub seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub serving: Serving,
    pub phones: Vec<Phone>,
    pub listed: bool,
    pub pairing: Option<Pairing>,
    pub hostname: String,
    pub ip: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Memory {
    pairing_from: Option<Vec<String>>,
    focus: Option<String>,
}

impl Memory {
    pub(crate) const fn new() -> Self {
        Self {
            pairing_from: None,
            focus: None,
        }
    }

    pub(crate) fn began(&mut self, snapshot: &Snapshot) {
        self.pairing_from = snapshot.listed.then(|| {
            snapshot
                .phones
                .iter()
                .map(|phone| phone.id.clone())
                .collect()
        });
    }

    pub(crate) fn stopped(&mut self) {
        self.pairing_from = None;
    }

    pub(crate) fn removed(&mut self, phones: &[Phone], id: &str) {
        self.focus = Some(neighbour(phones, id));
    }

    pub(crate) fn observe(&mut self, snapshot: &Snapshot, open: &[String]) -> bool {
        if !open.iter().any(|page| page == LIST) {
            self.focus = None;
        }
        let Some(before) = &self.pairing_from else {
            return false;
        };
        if let Some(joined) = snapshot
            .phones
            .iter()
            .filter(|_| snapshot.listed)
            .find(|phone| !before.contains(&phone.id))
        {
            self.focus = Some(phone_id(&joined.id));
            self.pairing_from = None;
            return true;
        }
        if snapshot.pairing.is_none() {
            self.pairing_from = None;
        }
        false
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Serve(bool),
    Pair,
    Stop,
    Remove(String),
    Nothing,
}

pub(crate) fn act(page: &str, row: &str, set: Option<bool>) -> Act {
    match (page, row, set) {
        (ROOT, SERVER, Some(on)) => Act::Serve(on),
        (ROOT | LIST, PAIR, _) => Act::Pair,
        (PAIR, STOP, _) => Act::Stop,
        (page, REMOVE, _) => match page.strip_prefix(PHONE) {
            Some(id) => Act::Remove(id.to_string()),
            None => Act::Nothing,
        },
        _ => Act::Nothing,
    }
}

pub(crate) fn build(snapshot: &Snapshot, memory: &Memory, now: DateTime<Local>) -> Pages {
    let mut pages = Pages::new(ROOT);
    pages.insert(ROOT, root(snapshot));
    pages.insert(LIST, list(snapshot, memory, now));
    if let Some(pairing) = &snapshot.pairing {
        pages.insert(PAIR, pair(pairing));
    }
    pages.insert(APP, app());
    for phone in &snapshot.phones {
        pages.insert(phone_id(&phone.id), phone_page(phone, now));
    }
    pages
}

pub(crate) fn name_of<'a>(phones: &'a [Phone], id: &str) -> Option<&'a str> {
    phones
        .iter()
        .find(|phone| phone.id == id)
        .map(|phone| phone.name.as_str())
}

fn root(snapshot: &Snapshot) -> Page {
    let on = !matches!(snapshot.serving, Serving::Off | Serving::Unknown);
    let healthy = matches!(snapshot.serving, Serving::Running | Serving::Off);
    Page::new("PointZerver")
        .focus(SERVER)
        .group(
            Group::new(
                "remote control",
                "use your phone as this computer's mouse and keyboard.",
            )
            .row(
                Row::new(SERVER, "PointZerver")
                    .detail(describe(snapshot))
                    .feature(on, count(snapshot.phones.len()), LIST)
                    .attention(!healthy),
            )
            .row(pair_row())
            .row(
                Row::new(APP, "Get the phone app")
                    .detail("Scan to download the PointZ app.")
                    .open(APP),
            ),
        )
        .group(this_computer(snapshot))
}

fn this_computer(snapshot: &Snapshot) -> Group {
    Group::new("this computer", "what the PointZ app connects to.")
        .row(Row::new("hostname", "Hostname").value(snapshot.hostname.clone()))
        .row(
            Row::new("ip", "IP address")
                .value(snapshot.ip.clone().unwrap_or_else(|| "none".into())),
        )
        .row(Row::new("control", "Control port").value(ServerConfig::COMMAND_PORT.to_string()))
        .row(
            Row::new("discovery", "Discovery port").value(ServerConfig::DISCOVERY_PORT.to_string()),
        )
}

fn describe(snapshot: &Snapshot) -> String {
    match snapshot.serving {
        Serving::Running => match &snapshot.ip {
            Some(ip) => format!("Running · phones find {} at {ip}", snapshot.hostname),
            None => format!("Running · phones find {}", snapshot.hostname),
        },
        Serving::Off => "Off · phones cannot reach this computer".into(),
        Serving::PortBusy(port) => format!("Not listening · port {port} is in use"),
        Serving::Failed(port) => format!("Not listening · port {port} would not open"),
        Serving::Unknown => "Not listening · qol did not answer".into(),
    }
}

fn count(phones: usize) -> String {
    match phones {
        1 => "1 phone".into(),
        phones => format!("{phones} phones"),
    }
}

fn list(snapshot: &Snapshot, memory: &Memory, now: DateTime<Local>) -> Page {
    let mut phones = Group::new("phones", "phones that can control this computer.");
    if snapshot.phones.is_empty() {
        phones.push(pair_row());
    }
    for phone in &snapshot.phones {
        phones.push(
            Row::new(phone_id(&phone.id), phone.name.clone())
                .detail(paired(phone.paired_at_ms, now))
                .open(phone_id(&phone.id)),
        );
    }
    let page = Page::new("pointzerver").group(phones);
    match &memory.focus {
        Some(focus) => page.focus(focus.clone()),
        None => page,
    }
}

fn pair_row() -> Row {
    Row::new(PAIR, "Pair a phone")
        .detail("Shows a one-minute code to type into the PointZ app")
        .press_open()
}

fn pair(pairing: &Pairing) -> Page {
    Page::new("pair a phone").leave(STOP, "stop pairing").group(
        Group::new(
            "pair a phone",
            "type the code into the PointZ app on your phone.",
        )
        .row(
            Row::new("code", "Code")
                .value(spaced(&pairing.code))
                .detail(format!("Runs out in {} s", pairing.seconds)),
        ),
    )
}

fn app() -> Page {
    Page::new("get the phone app").group(
        Group::new("get the phone app", "scan it with your phone's camera.").row(
            Row::new("qr", "Get the phone app")
                .detail("Scan to download the PointZ app.")
                .qr(ServerConfig::APP_DOWNLOAD_URL),
        ),
    )
}

fn phone_page(phone: &Phone, now: DateTime<Local>) -> Page {
    let title = phone.name.to_lowercase();
    Page::new(title.clone()).group(
        Group::new(
            title,
            format!("{}.", paired(phone.paired_at_ms, now).to_lowercase()),
        )
        .row(
            Row::new(REMOVE, "Remove this phone")
                .detail("It has to pair again to control this computer")
                .destroy("remove", format!("remove {}", phone.name)),
        ),
    )
}

fn phone_id(id: &str) -> String {
    format!("{PHONE}{id}")
}

fn neighbour(phones: &[Phone], id: &str) -> String {
    let Some(at) = phones.iter().position(|phone| phone.id == id) else {
        return PAIR.into();
    };
    phones
        .get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|before| phones.get(before)))
        .map_or_else(|| PAIR.into(), |phone| phone_id(&phone.id))
}

fn paired(paired_at_ms: u64, now: DateTime<Local>) -> String {
    let Some(at) = i64::try_from(paired_at_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
    else {
        return "Paired".into();
    };
    let at = at.with_timezone(&Local);
    if now.signed_duration_since(at) < JUST_NOW {
        return "Paired just now".into();
    }
    format!("Paired {}", at.format("%-d %B"))
}

fn spaced(code: &str) -> String {
    if code.len() != 6 || !code.is_ascii() {
        return code.into();
    }
    format!("{} {}", &code[..3], &code[3..])
}

#[cfg(test)]
mod tests;
