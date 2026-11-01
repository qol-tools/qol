use chrono::NaiveDateTime;
use qol_config::pages::{Row, Tone};

use super::last::{names, who};
use super::settings::{hours_value, rules};
use super::updates::{failed_line, install_row, row, skipped_row, sorted, total, SHOWN};
use super::words::{heading, join, night_of, title};
use super::{Sheet, ROOT};
use crate::session::Session;
use crate::updates::Auto;
use qol_config::text::plural;

pub(super) fn page(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new(ROOT, "Update Manager");
    notes(&mut sheet, session);
    status(&mut sheet, session, now);
    owner(&mut sheet, session);
    sheet.heading("qol");
    rules(&mut sheet, session);
    sheet.row(
        Row::new("changes", "Changed by qol")
            .value(changed(session).to_string())
            .open("changes"),
    );
    sheet.heading("history");
    sheet.row(
        match session.host.runs.first() {
            Some(run) => Row::new("last", heading(run.start, now))
                .tag(who(run))
                .value(names(run)),
            None => Row::new("last", "No run yet"),
        }
        .open("last"),
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
    sheet.heading("right now");
    if session.locked() {
        sheet.row(
            Row::info("locked", "You are not an administrator here")
                .detail(
                    "Installs ask for an administrator's password. Mint's settings are read-only.",
                )
                .tone(Tone::Attention),
        );
    }
    if let Some((label, detail)) = failed_line(session) {
        sheet.row(
            Row::new("failed", label)
                .detail(detail)
                .tone(Tone::Attention)
                .open("failed"),
        );
    }
    for (index, problem) in problems.into_iter().enumerate() {
        let row = match problem.split_once(": ") {
            Some((label, detail)) => Row::info(format!("problem:{index}"), label).detail(detail),
            None => Row::info(format!("problem:{index}"), problem.as_str()),
        };
        sheet.row(row.tone(Tone::Attention));
    }
}

fn status(sheet: &mut Sheet, session: &Session, now: NaiveDateTime) {
    sheet.heading("updates");
    if let Some(job) = session.job.as_ref().filter(|job| !job.started) {
        sheet.row(
            Row::info("waiting", "Waiting for the password")
                .value(plural(total(job.updates.iter()), "update", "updates"))
                .busy(true),
        );
    } else if let Some(job) = session.job.as_ref() {
        let all = total(job.updates.iter());
        sheet.row(
            Row::info(
                "waiting",
                format!("Installing {}", plural(all, "update", "updates")),
            )
            .value(format!("{} of {all}", job.done))
            .busy(true),
        );
    } else if session.updates.is_none() {
        sheet.row(Row::info("waiting", "Looking for updates").busy(true));
    } else if session.list().is_empty() {
        let row = Row::info("waiting", "Nothing waiting");
        sheet.row(match session.checked {
            Some(at) => row.value(format!("checked {}", at.format("%H:%M"))),
            None => row,
        });
    } else {
        install_row(sheet, session);
    }
    let list = sorted(session);
    for update in list.iter().take(SHOWN) {
        sheet.row(row(session, update, false));
    }
    if list.len() > SHOWN {
        sheet.row(Row::new("more", format!("{} more", list.len() - SHOWN)).open("waiting"));
    }
    skipped_row(sheet, session);
    let finish = session.finish();
    let row = match finish.as_slice() {
        [] => match session.host.restart_at {
            Some(at) => Row::new(
                "restart",
                format!("Restart {} at {}", night_of(at, now), at.format("%H:%M")),
            ),
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
    sheet.row(row.tone(Tone::Attention).open("restart"));
}

fn owner(sheet: &mut Sheet, session: &Session) {
    sheet.heading(if session.borrowed {
        "mint, the owner's"
    } else {
        "mint"
    });
    let Some(mint) = session.mint() else {
        sheet.row(
            Row::info("auto", "Mint's Update Manager is not set up here").value("when you say"),
        );
        return;
    };
    if session.locked() {
        sheet.row(Row::info("auto", "Installs on its own").value(auto_value(mint.auto)));
        sheet.row(Row::info("hours", "Hours").value(hours_value(session)));
        return;
    }
    sheet.row(
        Row::new("auto", "Installs on its own")
            .value(auto_value(mint.auto))
            .open("auto"),
    );
    sheet.row(
        Row::new("hours", "Hours")
            .value(if mint.auto.packages {
                hours_value(session)
            } else {
                "unused".to_string()
            })
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
        on.join(", ")
    }
}

fn changed(session: &Session) -> usize {
    usize::from(session.mint().is_some_and(|mint| mint.ours))
        + usize::from(session.host.restart_at.is_some())
        + session.auto_changed().len()
}
