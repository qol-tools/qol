use chrono::NaiveDateTime;
use qol_config::pages::{Row, Tone};

use super::last::line;
use super::settings::{hours_value, rules};
use super::updates::{failed_line, held_phrase, total};
use super::words::{heading, join, night_of, plural, title};
use super::{Sheet, ROOT};
use crate::session::Session;
use crate::updates::Auto;

pub(super) fn page(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new(ROOT, "Update Manager");
    notes(&mut sheet, session);
    status(&mut sheet, session, now);
    owner(&mut sheet, session);
    sheet.group("while qol runs", "qol's rules. they end when qol stops.");
    rules(&mut sheet, session);
    sheet.group("last run", "what installed, and what it stopped.");
    sheet.row(
        match session.host.runs.first() {
            Some(run) => Row::new("last", heading(run.start, now)).detail(line(run)),
            None => Row::new("last", "No run yet").detail("Mint's log lists no installs"),
        }
        .open("last"),
    );
    sheet.row(
        Row::new("changes", "What qol changed here")
            .detail(changed(session, now))
            .open("changes"),
    );
    sheet
}

fn notes(sheet: &mut Sheet, session: &Session) {
    let problems: Vec<&String> = session
        .updates
        .iter()
        .flat_map(|updates| updates.problems.iter())
        .chain(session.host.problems.iter())
        .collect();
    if !session.locked() && session.failed.is_none() && problems.is_empty() {
        return;
    }
    sheet.group("right now", "what is different on this computer.");
    if session.locked() {
        sheet.row(
            Row::info("locked", "You are not an administrator here")
                .detail(
                    "Installs ask for an administrator's password. Mint's settings are read-only.",
                )
                .attention(true),
        );
    }
    if let Some((label, detail)) = failed_line(session) {
        sheet.row(
            Row::new("failed", label)
                .detail(detail)
                .tone(Tone::Attention)
                .attention(true)
                .open("failed"),
        );
    }
    for (index, problem) in problems.into_iter().enumerate() {
        let row = match problem.split_once(": ") {
            Some((label, detail)) => Row::info(format!("problem:{index}"), label).detail(detail),
            None => Row::info(format!("problem:{index}"), problem.as_str()),
        };
        sheet.row(row.tone(Tone::Attention).attention(true));
    }
}

fn status(sheet: &mut Sheet, session: &Session, now: NaiveDateTime) {
    sheet.group("updates", "what is ready to install or finish.");
    if let Some(job) = session.job.as_ref().filter(|job| !job.started) {
        sheet.row(
            Row::new("waiting", "Waiting for the password")
                .detail(format!(
                    "To install {}",
                    plural(total(job.updates.iter()), "update", "updates")
                ))
                .attention(true)
                .busy(true)
                .open("waiting"),
        );
    } else if let (Some(job), Some((current, done))) =
        (session.job.as_ref(), session.job_position())
    {
        let all = total(job.updates.iter());
        sheet.row(
            Row::new(
                "waiting",
                format!("Installing {}", plural(all, "update", "updates")),
            )
            .detail(format!(
                "{}: {done} of {}",
                current.label,
                plural(current.count(), "package", "packages")
            ))
            .value(format!("{} of {all}", job.done))
            .attention(true)
            .busy(true)
            .open("waiting"),
        );
    } else if session.updates.is_none() {
        sheet.row(Row::info("waiting", "Looking for updates").busy(true));
    } else if session.list().is_empty() {
        let checked = session
            .checked
            .map(|at| format!("Checked at {}", at.format("%H:%M")))
            .unwrap_or_default();
        sheet.row(Row::info("waiting", "Nothing waiting").detail(checked));
    } else {
        let available = session.available();
        let held = session.held();
        let detail = if available.is_empty() {
            let labels: Vec<String> = held.iter().map(|update| update.label.clone()).collect();
            format!("{}, {}", join(&labels), held_phrase(&held))
        } else if held.is_empty() {
            format!("All {} can install", total(available.iter().copied()))
        } else {
            format!(
                "{} can install, {} {}",
                total(available.iter().copied()),
                total(held.iter().copied()),
                held_phrase(&held)
            )
        };
        sheet.row(
            Row::new(
                "waiting",
                format!(
                    "{} waiting",
                    plural(total(session.list().iter()), "update", "updates")
                ),
            )
            .detail(detail)
            .attention(!available.is_empty())
            .open("waiting"),
        );
    }
    let finish = session.finish();
    let row = match finish.as_slice() {
        [] => match session.host.restart_at {
            Some(at) => Row::new(
                "restart",
                format!("Restart {} at {}", night_of(at, now), at.format("%H:%M")),
            )
            .detail("Nothing waits for it"),
            None => return,
        },
        [one] => Row::new("restart", one.label.as_str()).detail(one.short.as_str()),
        [first, rest @ ..] => {
            let names: Vec<String> = rest.iter().map(|row| row.name.clone()).collect();
            Row::new(
                "restart",
                format!("{} and {} more", first.label, rest.len()),
            )
            .detail(format!(
                "{} finish{} at the same restart.",
                title(&join(&names)),
                if rest.len() == 1 { "es" } else { "" }
            ))
        }
    };
    sheet.row(row.attention(true).open("restart"));
}

fn owner(sheet: &mut Sheet, session: &Session) {
    sheet.group(
        "this computer's settings",
        if session.borrowed {
            "the owner's. a change asks first, and stays after you leave."
        } else {
            "mint's own. they stay when qol stops."
        },
    );
    let Some(mint) = session.mint() else {
        sheet.row(
            Row::info("auto", "Mint's Update Manager is not set up here")
                .detail("Updates install when you say"),
        );
        return;
    };
    if session.locked() {
        sheet.row(Row::info("auto", "Install on their own").value(auto_value(mint.auto)));
        sheet.row(Row::info("hours", "Update hours").value(hours_value(session)));
        return;
    }
    sheet.row(
        Row::new("auto", "Install on their own")
            .detail("What Mint installs without asking")
            .value(auto_value(mint.auto))
            .open("auto"),
    );
    sheet.row(
        Row::new("hours", "Update hours")
            .detail(if mint.auto.packages {
                "When Mint's nightly run may start"
            } else {
                "Unused while packages wait for you"
            })
            .value(hours_value(session))
            .open("hours"),
    );
}

fn auto_value(auto: Auto) -> String {
    let on: Vec<String> = [
        (auto.packages, "packages"),
        (auto.spices, "spices"),
        (auto.flatpak, "Flatpak"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name.to_string())
    .collect();
    if on.is_empty() {
        "when you say".into()
    } else {
        join(&on)
    }
}

pub(super) fn changed(session: &Session, now: NaiveDateTime) -> String {
    let mut items: Vec<String> = Vec::new();
    if session.guarding() {
        items.push("Wait while in use".into());
    }
    if session.holding_all() {
        items.push("Hold all updates".into());
    }
    if session.mint().is_some_and(|mint| mint.ours) {
        items.push("Update hours".into());
    }
    if let Some(at) = session.host.restart_at {
        items.push(format!("Restart {}", night_of(at, now)));
    }
    if !session.auto_changed().is_empty() {
        items.push("Install on their own".into());
    }
    if items.is_empty() {
        "Nothing so far".into()
    } else {
        join(&items)
    }
}
