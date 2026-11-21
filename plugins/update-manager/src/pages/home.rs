use chrono::NaiveDateTime;
use qol_config::pages::{Row, Tone};

use super::cleanup;
use super::settings::AUTO;
use super::updates::{failed_line, held_phrase, listed, problems, total, WAITING};
use super::words::{join, night_of, title};
use super::{Sheet, ROOT};
use crate::session::Session;
use crate::updates::{day, sources};
use qol_config::text::plural;

const STATUS: &str = "status";

pub(super) fn page(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new(ROOT, "Update Manager");
    sheet.group(
        "updates",
        "what is waiting to install, and what it needs from you.",
    );
    status(&mut sheet, session);
    restart(&mut sheet, session, now);
    sheet.group(
        "in the background",
        "what happens without you, and what it did.",
    );
    installs(&mut sheet, session);
    cleanup::row(&mut sheet, session);
    history(&mut sheet, session, now);
    sheet
}

fn status(sheet: &mut Sheet, session: &Session) {
    let waiting = session.list().len();
    let row = if let Some(job) = session.job.as_ref().filter(|job| !job.started) {
        Row::new(STATUS, "Waiting for the password")
            .value(plural(total(job.updates.iter()), "update", "updates"))
            .busy(true)
    } else if let Some(job) = session.job.as_ref() {
        let all = total(job.updates.iter());
        Row::new(
            STATUS,
            format!("Installing {}", plural(all, "update", "updates")),
        )
        .value(format!("{} of {all}", job.done))
        .busy(true)
    } else if let Some((label, detail)) = failed_line(session) {
        Row::new(STATUS, label).detail(detail).tone(Tone::Attention)
    } else if session.updates.is_none() {
        Row::new(STATUS, "Looking for updates").busy(true)
    } else if waiting == 0 {
        let row = Row::new(STATUS, "Nothing waiting");
        match session.checked {
            Some(at) => row.value(format!("checked {}", at.format("%H:%M"))),
            None => row,
        }
    } else if session.locked() {
        Row::new(STATUS, "You are not an administrator")
            .detail("Installs ask for an administrator's password")
            .value(format!("{waiting} waiting"))
            .tone(Tone::Attention)
    } else {
        let row = Row::new(STATUS, format!("{waiting} waiting"));
        let held = session.held();
        if session.available().is_empty() && !held.is_empty() {
            row.value(held_phrase(&held))
        } else {
            row
        }
    };
    let row = match problems(session).first() {
        Some(problem) if session.job.is_none() && session.failed.is_none() => {
            row.detail(problem.as_str()).tone(Tone::Attention)
        }
        _ => row,
    };
    sheet.row(if listed(session) {
        row.open(WAITING)
    } else {
        row
    });
}

fn restart(sheet: &mut Sheet, session: &Session, now: NaiveDateTime) {
    let planned = session
        .host
        .restart_at
        .map(|at| (night_of(at, now), at.format("%H:%M").to_string()));
    let finish = session.finish();
    let row = match (finish.as_slice(), planned) {
        ([], None) => return,
        ([], Some((night, clock))) => Row::new("restart", format!("Restart {night} at {clock}")),
        (rows, planned) => {
            let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
            let row = Row::new("restart", "Restart to finish").detail(format!(
                "{} finish{} at the restart",
                title(&join(&names)),
                if rows.len() == 1 { "es" } else { "" }
            ));
            match planned {
                Some((night, clock)) => row.value(format!("{night} {clock}")),
                None => row,
            }
        }
    };
    sheet.row(row.tone(Tone::Attention).open("restart"));
}

fn installs(sheet: &mut Sheet, session: &Session) {
    if session.mint().is_none() {
        return;
    }
    let auto = session.auto();
    let value = if session.holding_all() {
        "paused"
    } else if sources().iter().any(|source| auto.get(source.key)) {
        "on"
    } else {
        "off"
    };
    sheet.row(
        Row::new(AUTO, "Installs on its own")
            .value(value)
            .open(AUTO),
    );
}

fn history(sheet: &mut Sheet, session: &Session, now: NaiveDateTime) {
    let row = Row::new("last", "History");
    let row = match session.host.runs.first() {
        Some(run) => row.value(format!(
            "{} {}",
            day(run.start, now),
            run.start.format("%H:%M")
        )),
        None => row.value("nothing yet"),
    };
    sheet.row(row.open("last"));
}
