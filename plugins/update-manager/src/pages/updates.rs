use std::hash::{DefaultHasher, Hash, Hasher};

use qol_config::pages::{Go, Row, Tone};

use super::restart::with_error;
use super::words::{cap, join, size, title};
use super::Sheet;
use crate::session::{position, Act, Op, Session};
use crate::updates::{Held, Update};
use qol_config::text::plural;

pub(super) const WAITING: &str = "waiting";

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

fn warns(session: &Session, update: &Update) -> bool {
    update.restart
        || update.caution.is_some()
        || update.held == Some(Held::Administrator)
        || (update.stops_sound && session.in_use.is_some())
}

pub(super) fn sorted(session: &Session) -> Vec<&Update> {
    let mut list: Vec<&Update> = session.list().iter().collect();
    list.sort_by_key(|update| match (update.installable(), update.restart) {
        (false, _) => 2,
        (true, true) => 0,
        (true, false) => 1,
    });
    list
}

fn shared_tag(list: &[&Update]) -> bool {
    list.first()
        .is_some_and(|first| list.iter().all(|update| update.tag == first.tag))
}

pub(super) fn row(session: &Session, update: &Update, tagged: bool) -> Row {
    let id = format!("update:{}", update.id);
    let value = job_detail(session, update).or_else(|| match update.held {
        Some(held) if !update.installable() => Some(held.phrase().to_string()),
        _ => update.restart.then(|| "restart".to_string()),
    });
    let row = Row::new(id.clone(), update.label.as_str())
        .dim(!update.installable())
        .open(id);
    let row = match value {
        Some(value) => row.value(value),
        None => row,
    };
    if tagged {
        row.tag(update.tag.as_str())
    } else {
        row
    }
}

pub(super) fn install_row(sheet: &mut Sheet, session: &Session) {
    let available = session.available();
    let label = format!("{} waiting", session.list().len());
    if session.job.is_some() || available.is_empty() {
        let held = session.held();
        let row = Row::new("install", label);
        sheet.row(match session.job {
            None if !held.is_empty() => row.value(held_phrase(&held)),
            _ => row,
        });
        return;
    }
    let row = Row::new("install", label).button("Install all");
    if available.iter().any(|update| warns(session, update)) {
        sheet.row(row.open(install_all(session)));
    } else {
        sheet.press(
            row,
            Act::Install(available.iter().map(|update| update.id.clone()).collect()),
        );
    }
}

pub(super) fn problems(session: &Session) -> Vec<&String> {
    session
        .updates
        .iter()
        .flat_map(|updates| updates.problems.iter())
        .chain(session.host.problems.iter())
        .collect()
}

fn skipped(session: &Session) -> bool {
    !session.skips().is_empty()
        || session.busy.contains(&Op::Skips)
        || session.errors.contains_key(&Op::Skips)
}

pub(super) fn listed(session: &Session) -> bool {
    !session.list().is_empty()
        || session.job.is_some()
        || session.failed.is_some()
        || skipped(session)
        || !problems(session).is_empty()
}

fn skipped_row(sheet: &mut Sheet, session: &Session) {
    let busy = session.busy.contains(&Op::Skips);
    if !skipped(session) {
        return;
    }
    sheet.row(with_error(
        Row::new("skipped", "Skipped")
            .value(session.skips().len().to_string())
            .busy(busy)
            .open("skipped"),
        session,
        Op::Skips,
    ));
}

pub(super) fn pages(session: &Session) -> Vec<Sheet> {
    let mut sheets = Vec::new();
    if listed(session) {
        sheets.extend(waiting(session));
    }
    let available = session.available();
    if session.job.is_none() && available.iter().any(|update| warns(session, update)) {
        sheets.push(install(session, &install_all(session), &available));
    }
    for update in session.list() {
        sheets.push(one(session, update));
        if update.count() > 1 {
            sheets.extend(packages(update));
        }
    }
    if let Some(sheet) = failed(session) {
        sheets.push(sheet);
    }
    sheets
}

fn waiting(session: &Session) -> Vec<Sheet> {
    let mut sheet = Sheet::new(WAITING, "waiting");
    sheet.group("problems", "what did not work, and why.");
    if let Some((label, detail)) = failed_line(session) {
        sheet.row(
            Row::new("failed", label)
                .detail(detail)
                .tone(Tone::Attention)
                .open("failed"),
        );
    }
    for (index, problem) in problems(session).into_iter().enumerate() {
        let row = match problem.split_once(": ") {
            Some((label, detail)) => Row::new(format!("problem:{index}"), label).detail(detail),
            None => Row::new(format!("problem:{index}"), problem.as_str()),
        };
        sheet.row(row.tone(Tone::Attention));
    }
    sheet.group("install", "everything that can install now, in one go.");
    if !session.list().is_empty() {
        install_row(&mut sheet, session);
    }
    let list = sorted(session);
    let tagged = !shared_tag(&list);
    let rows = list
        .into_iter()
        .map(|update| row(session, update, tagged))
        .collect();
    let mut sheets = sheet.list(
        "waiting",
        "every update found, those that need a restart first.",
        rows,
    );
    if let Some(first) = sheets.first_mut() {
        first.group("skipped", "updates you chose to leave out.");
        skipped_row(first, session);
    }
    sheets
}

fn version(update: &Update) -> String {
    update
        .label
        .rsplit(' ')
        .next()
        .filter(|word| word.starts_with(|first: char| first.is_ascii_digit()))
        .map_or_else(|| "this version".to_string(), str::to_string)
}

fn warnings(sheet: &mut Sheet, session: &Session, update: &Update) {
    if update.restart {
        sheet.row(Row::new("restart", "Finishes at a restart").tone(Tone::Attention));
    }
    if let Some((label, _)) = update.caution.as_ref() {
        sheet.row(Row::new("caution", label.as_str()).tone(Tone::Attention));
    }
    if update.held == Some(Held::Administrator) {
        sheet.row(
            Row::new("release", "Its hold comes off")
                .detail(update.why.as_str())
                .tone(Tone::Attention),
        );
    }
    if let Some(in_use) = session.in_use.as_ref().filter(|_| update.stops_sound) {
        sheet.row(
            Row::new("in-use", format!("{} drops for a moment", in_use.label))
                .tone(Tone::Attention),
        );
    }
}

fn one(session: &Session, update: &Update) -> Sheet {
    let mut sheet = Sheet::new(format!("update:{}", update.id), update.label.to_lowercase());
    let idle = session.job.is_none();
    if idle && update.installable() && warns(session, update) {
        sheet.group(
            "before you install",
            "what this update does besides installing.",
        );
        warnings(&mut sheet, session, update);
    }
    sheet.group("choose", "what to do with this update.");
    if let Some(detail) = job_detail(session, update) {
        sheet.row(Row::new("state", cap(&detail)));
    } else if let Some(held) = update.held.filter(|_| !update.installable()) {
        sheet.row(Row::new("state", held.title()).detail(update.why.as_str()));
    } else if !idle {
        sheet.row(Row::new("state", "Waits for the install that is running"));
    } else {
        let value = match update.bytes() {
            Some(bytes) if update.count() == 1 => size(bytes),
            _ => plural(update.count(), update.unit.one(), update.unit.many()),
        };
        sheet.press(
            Row::new("install", "Install")
                .value(value)
                .button("install"),
            Act::Install(vec![update.id.clone()]),
        );
    }
    if idle && update.skippable() && session.mint().is_some() {
        let busy = session.busy.contains(&Op::Skips);
        sheet.press_then(
            Row::new("skip", format!("Skip {}", version(update)))
                .button("skip")
                .busy(busy),
            Act::Skip(update.id.clone()),
            Go::Back,
        );
        sheet.press_then(
            Row::new("never", "Never update").button("never").busy(busy),
            Act::Never(update.id.clone()),
            Go::Back,
        );
    }
    if update.count() > 1 {
        sheet.row(
            Row::new("packages", cap(update.unit.heading()))
                .value(update.count().to_string())
                .open(format!("packages:{}", update.id)),
        );
    }
    sheet
}

fn packages(update: &Update) -> Vec<Sheet> {
    let sheet = Sheet::new(
        format!("packages:{}", update.id),
        update.label.to_lowercase(),
    );
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
        "everything this update installs.",
        rows,
    )
}

fn install(session: &Session, id: &str, set: &[&Update]) -> Sheet {
    let count = total(set.iter().copied());
    let mut sheet = Sheet::new(id, "install all");
    sheet.group(
        "before you install",
        "what installing them all does besides installing.",
    );
    let restart: Vec<&Update> = set
        .iter()
        .copied()
        .filter(|update| update.restart)
        .collect();
    if !restart.is_empty() {
        sheet.row(
            Row::new("restart", names(&restart))
                .value("restart")
                .tone(Tone::Attention),
        );
    }
    if let Some(in_use) = session.in_use.as_ref() {
        let stops: Vec<&Update> = set
            .iter()
            .copied()
            .filter(|update| update.stops_sound)
            .collect();
        if !stops.is_empty() {
            sheet.row(
                Row::new("in-use", format!("{} drops for a moment", in_use.label))
                    .detail(format!(
                        "{} restart{} Bluetooth or sound",
                        names(&stops),
                        if stops.len() == 1 { "s" } else { "" }
                    ))
                    .tone(Tone::Attention),
            );
        }
    }
    for (index, (label, _)) in set
        .iter()
        .filter_map(|update| update.caution.as_ref())
        .enumerate()
    {
        sheet.row(Row::new(format!("caution:{index}"), label.as_str()).tone(Tone::Attention));
    }
    for update in set
        .iter()
        .filter(|update| update.held == Some(Held::Administrator))
    {
        sheet.row(
            Row::new(format!("release:{}", update.id), "Its hold comes off")
                .detail(update.label.as_str())
                .tone(Tone::Attention),
        );
    }
    let held = session.held();
    if !held.is_empty() {
        sheet.row(
            Row::new(
                "held",
                format!(
                    "{} {} held",
                    labels(&held),
                    if held.len() == 1 { "stays" } else { "stay" }
                ),
            )
            .dim(true),
        );
    }
    sheet.group("choose", "install them all now, or go back.");
    sheet.press_then(
        Row::new(
            "go",
            format!("Install {}", plural(count, "update", "updates")),
        )
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
            "some installed before it stopped. the rest still wait.".to_string()
        } else if one {
            "nothing changed. it still waits.".to_string()
        } else {
            "nothing changed. they still wait.".to_string()
        },
    );
    sheet.row(
        Row::new("reason", failed.failure.reason.label())
            .detail(failed.failure.reason.detail())
            .tone(Tone::Attention)
            .attention(true),
    );
    for update in failed.updates.iter().filter(|update| update.restart) {
        if let Some(running) = update.running.as_deref() {
            sheet.row(
                Row::new(
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
            .filter(|update| session.update(&update.id).is_some_and(Update::installable))
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
