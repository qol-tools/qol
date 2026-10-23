use std::hash::{DefaultHasher, Hash, Hasher};

use qol_config::pages::{Go, Row, Tone};

use super::words::{join, plural, size, title};
use super::Sheet;
use crate::session::{position, Act, Session};
use crate::updates::{Held, Update};

pub(super) fn total<'a>(updates: impl Iterator<Item = &'a Update>) -> usize {
    updates.map(Update::count).sum()
}

pub(super) fn held_phrase(held: &[&Update]) -> &'static str {
    match shared_hold(held) {
        Some(hold) => hold.phrase(),
        None => "held back",
    }
}

fn shared_hold(held: &[&Update]) -> Option<Held> {
    let first = held.first()?.held?;
    held.iter()
        .all(|update| update.held == Some(first))
        .then_some(first)
}

fn names(updates: &[&Update]) -> String {
    let names: Vec<String> = updates.iter().map(|update| update.name.clone()).collect();
    title(&join(&names))
}

fn labels(updates: &[&Update]) -> String {
    let labels: Vec<String> = updates.iter().map(|update| update.label.clone()).collect();
    join(&labels)
}

pub(super) fn failed_line(session: &Session) -> Option<(String, String)> {
    let failed = session.failed.as_ref()?;
    let label = match failed.updates.as_slice() {
        [one] => format!("{} did not install", one.label),
        many => format!(
            "{} did not install",
            plural(total(many.iter()), "update", "updates")
        ),
    };
    let detail = if failed.failure.done == 0 {
        format!(
            "{}. Nothing changed; {} still {}.",
            failed.failure.reason.label(),
            if failed.updates.len() == 1 {
                "it"
            } else {
                "they"
            },
            if failed.updates.len() == 1 {
                "waits"
            } else {
                "wait"
            }
        )
    } else {
        format!(
            "{}. {} of {} installed first.",
            failed.failure.reason.label(),
            failed.failure.done,
            total(failed.updates.iter())
        )
    };
    Some((label, detail))
}

fn job_detail(session: &Session, update: &Update) -> Option<String> {
    let job = session.job.as_ref()?;
    if !job.updates.iter().any(|queued| queued.id == update.id) {
        return None;
    }
    if !job.started {
        return Some("waits for the password".into());
    }
    Some(match position(&job.updates, job.done) {
        Some((current, done)) if current.id == update.id => format!(
            "installing, {done} of {}",
            plural(update.count(), "package", "packages")
        ),
        _ => "waits its turn".into(),
    })
}

pub(super) fn install_all(session: &Session) -> String {
    let mut hasher = DefaultHasher::new();
    for update in session.available() {
        update.id.hash(&mut hasher);
    }
    format!("install-all:{:016x}", hasher.finish())
}

pub(super) fn pages(session: &Session) -> Vec<Sheet> {
    let mut sheets = Vec::new();
    if session.list().is_empty() && session.job.is_none() {
        return sheets;
    }
    sheets.extend(waiting(session));
    if session.job.is_none() && !session.available().is_empty() {
        let ids: Vec<String> = session
            .available()
            .iter()
            .map(|update| update.id.clone())
            .collect();
        sheets.push(install(session, &install_all(session), &ids));
    }
    for update in session.list() {
        sheets.extend(one(session, update));
        if update.held.is_none() && session.job.is_none() {
            sheets.push(install(
                session,
                &format!("install:{}", update.id),
                std::slice::from_ref(&update.id),
            ));
        }
    }
    if let Some(sheet) = failed(session) {
        sheets.push(sheet);
    }
    sheets
}

fn waiting(session: &Session) -> Vec<Sheet> {
    let mut sheet = Sheet::new("waiting", "waiting");
    let available = session.available();
    if session.job.is_none() && !available.is_empty() {
        sheet.group("install", "asks before anything installs.");
        let detail = if session.locked() {
            "Installing asks for an administrator's password."
        } else {
            "Installing asks for your password."
        };
        sheet.row(
            Row::new(
                "install",
                plural(total(available.iter().copied()), "update", "updates"),
            )
            .detail(detail)
            .button("Install all")
            .open(install_all(session)),
        );
    }
    let mut list: Vec<&Update> = session.list().iter().collect();
    list.sort_by_key(|update| match (update.held, update.restart) {
        (Some(_), _) => 2,
        (None, true) => 0,
        (None, false) => 1,
    });
    let shared = list
        .first()
        .map(|update| update.tag.as_str())
        .filter(|tag| list.iter().all(|update| update.tag == *tag))
        .map(str::to_string);
    let rows = list
        .into_iter()
        .map(|update| {
            let id = format!("update:{}", update.id);
            let row = Row::new(id.clone(), update.label.as_str())
                .detail(job_detail(session, update).unwrap_or_else(|| update.wait()))
                .dim(update.held.is_some())
                .open(id);
            match shared {
                Some(_) => row,
                None => row.tag(update.tag.as_str()),
            }
        })
        .collect();
    let colophon = match &shared {
        Some(tag) => format!("all from {}.", tag.to_lowercase()),
        None => "each tagged with its updater.".to_string(),
    };
    sheet.list("waiting", &colophon, rows)
}

fn one(session: &Session, update: &Update) -> Vec<Sheet> {
    let mut sheet = Sheet::new(format!("update:{}", update.id), update.label.to_lowercase());
    sheet.group("install", "asks before anything installs.");
    if let Some(detail) = job_detail(session, update) {
        sheet.row(
            Row::info("state", super::words::cap(&detail))
                .detail("Each one installs in the order below"),
        );
    } else if let Some(held) = update.held {
        sheet.row(Row::info("state", held.title()).detail(update.why.as_str()));
    } else if session.job.is_some() {
        sheet.row(
            Row::info("state", "Waits for the install that is running").detail(update.why.as_str()),
        );
    } else {
        sheet.row(
            Row::new(
                "install",
                if update.count() == 1 {
                    "Install it now".to_string()
                } else {
                    format!("Install these {} now", update.count())
                },
            )
            .detail(update.why.as_str())
            .button("install")
            .open(format!("install:{}", update.id)),
        );
    }
    let rows = update
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let row = Row::new(format!("package:{index}"), package.name.as_str())
                .detail(package.detail.as_str());
            match package.bytes {
                Some(bytes) => row.value(size(bytes)),
                None => row,
            }
        })
        .collect();
    sheet.list(
        update.unit.heading(),
        "each one, with its download size.",
        rows,
    )
}

fn install(session: &Session, id: &str, ids: &[String]) -> Sheet {
    let set: Vec<&Update> = session
        .available()
        .into_iter()
        .filter(|update| ids.contains(&update.id))
        .collect();
    let count = total(set.iter().copied());
    let mut sheet = Sheet::new(id, "install now");
    sheet.group(
        format!("install {} now?", plural(count, "update", "updates")),
        "nothing installs until you choose install.",
    );
    let (restart, rest): (Vec<&Update>, Vec<&Update>) =
        set.iter().copied().partition(|update| update.restart);
    if !restart.is_empty() {
        sheet.row(
            Row::info("restart", names(&restart))
                .detail(format!(
                    "{} when the computer restarts. You choose when.",
                    if restart.len() == 1 {
                        "It finishes"
                    } else {
                        "They finish"
                    }
                ))
                .value(total(restart.iter().copied()).to_string()),
        );
    }
    if !rest.is_empty() {
        sheet.row(
            Row::info("rest", names(&rest))
                .detail("Nothing restarts")
                .value(total(rest.iter().copied()).to_string()),
        );
    }
    if let Some(in_use) = session.in_use.as_ref() {
        let stops: Vec<&Update> = set
            .iter()
            .copied()
            .filter(|update| update.stops_sound)
            .collect();
        if stops.is_empty() {
            sheet.row(
                Row::info("in-use", format!("{} keeps playing", in_use.label))
                    .detail("Nothing here restarts Bluetooth or sound"),
            );
        } else {
            sheet.row(
                Row::info("in-use", format!("{} drops for a moment", in_use.label))
                    .detail(format!(
                        "{} restart{} Bluetooth or sound",
                        names(&stops),
                        if stops.len() == 1 { "s" } else { "" }
                    ))
                    .tone(Tone::Attention)
                    .attention(true),
            );
        }
    }
    for (index, (label, detail)) in set
        .iter()
        .filter_map(|update| update.caution.as_ref())
        .enumerate()
    {
        sheet.row(
            Row::info(format!("caution:{index}"), label.as_str())
                .detail(detail.as_str())
                .attention(true),
        );
    }
    let held = session.held();
    if !held.is_empty() && set.len() == session.available().len() {
        sheet.row(
            Row::info(
                "held",
                format!(
                    "{} {} held",
                    labels(&held),
                    if held.len() == 1 { "stays" } else { "stay" }
                ),
            )
            .detail(shared_hold(&held).map_or("Each waits for its own hold", Held::decides))
            .value(total(held.iter().copied()).to_string())
            .dim(true),
        );
    }
    sheet.group("choose", "esc cancels too.");
    sheet.row(
        Row::new("cancel", "Cancel")
            .detail("Back to the page, nothing installed")
            .back(),
    );
    sheet.press_then(
        Row::new(
            "go",
            format!("Install {}", plural(count, "update", "updates")),
        )
        .detail(if session.locked() {
            "Asks for an administrator's password next"
        } else {
            "Asks for your password next"
        })
        .button("install"),
        Act::Install(set.iter().map(|update| update.id.clone()).collect()),
        Go::Back,
    );
    sheet
}

fn failed(session: &Session) -> Option<Sheet> {
    let failed = session.failed.as_ref()?;
    let (label, _) = failed_line(session)?;
    let mut sheet = Sheet::new("failed", "did not install");
    let one = failed.updates.len() == 1;
    sheet.group(
        label.to_lowercase(),
        if failed.failure.done > 0 {
            format!(
                "{} of {} installed before it stopped.",
                failed.failure.done,
                total(failed.updates.iter())
            )
        } else if one {
            "nothing changed. it still waits.".to_string()
        } else {
            "nothing changed. they still wait.".to_string()
        },
    );
    sheet.row(
        Row::info("reason", failed.failure.reason.label())
            .detail(failed.failure.reason.detail())
            .tone(Tone::Attention)
            .attention(true),
    );
    for update in failed.updates.iter().filter(|update| update.restart) {
        if let Some(running) = update.running.as_deref() {
            sheet.row(
                Row::info(
                    format!("running:{}", update.id),
                    format!("{running} keeps running"),
                )
                .detail("What runs now was not touched"),
            );
        }
    }
    if session.job.is_none() {
        let waiting: Vec<String> = failed
            .updates
            .iter()
            .filter(|update| {
                session
                    .update(&update.id)
                    .is_some_and(|known| known.held.is_none())
            })
            .map(|update| update.id.clone())
            .collect();
        if !waiting.is_empty() {
            let count: usize = failed
                .updates
                .iter()
                .filter(|update| waiting.contains(&update.id))
                .map(Update::count)
                .sum();
            sheet.press_then(
                Row::new("retry", "Try again")
                    .detail(format!(
                        "Installs {} {} now",
                        if one { "its" } else { "their" },
                        plural(count, "package", "packages")
                    ))
                    .button("retry"),
                Act::Install(waiting),
                Go::Back,
            );
        }
    }
    if let Some(log) = session.install_log.as_ref() {
        sheet.press(
            Row::new("log", "The install log")
                .detail(log.as_str())
                .button("open"),
            Act::Open(log.clone()),
        );
    }
    Some(sheet)
}
