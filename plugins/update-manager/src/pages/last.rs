use chrono::NaiveDateTime;
use qol_config::pages::{Row, Tone};

use super::words::{heading, minutes};
use super::Sheet;
use crate::session::{Act, Session};
use crate::updates::{Run, RunBy};
use qol_config::text::plural;

const SHOWN: usize = 8;

pub(super) fn who(run: &Run) -> String {
    match &run.by {
        RunBy::Automatic => "mint".to_string(),
        RunBy::Qol => "you".to_string(),
        RunBy::Person(name) => name.clone(),
    }
}

pub(super) fn names(run: &Run) -> String {
    if run.upgraded == 0 {
        return format!("{} added", plural(run.added, "package", "packages"));
    }
    let names: Vec<&str> = run.groups.iter().map(|(name, _)| name.as_str()).collect();
    names.join(", ")
}

pub(super) fn pages(session: &Session, now: NaiveDateTime) -> Vec<Sheet> {
    let mut sheets = vec![page(session, now)];
    for (index, run) in session.host.runs.iter().take(SHOWN).enumerate() {
        sheets.push(one(index, run, now));
    }
    sheets
}

fn page(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("last", "history");
    sheet.heading("history");
    if session.host.runs.is_empty() {
        sheet.row(Row::info("none", "No run yet").value("nothing in the log"));
    }
    for (index, run) in session.host.runs.iter().take(SHOWN).enumerate() {
        let id = format!("run:{index}");
        let row = Row::new(id.clone(), heading(run.start, now))
            .tag(who(run))
            .value(names(run))
            .open(id);
        sheet.row(match run.bluetooth {
            Some(_) => row.tone(Tone::Attention),
            None => row,
        });
    }
    if let Some(log) = session.host.log.as_ref() {
        sheet.press(
            Row::new("log", "Log").button("open"),
            Act::Open(log.clone()),
        );
    }
    sheet
}

fn one(index: usize, run: &Run, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new(
        format!("run:{index}"),
        heading(run.start, now).to_lowercase(),
    );
    sheet.heading(match &run.by {
        RunBy::Automatic => "by mint".to_string(),
        RunBy::Qol => "by you".to_string(),
        RunBy::Person(name) => format!("by {name}"),
    });
    for (name, count) in &run.groups {
        sheet.row(
            Row::info(format!("group:{name}"), name.as_str())
                .value(plural(*count, "package", "packages")),
        );
    }
    if run.added > 0 {
        let row = Row::info("added", "Added").value(plural(run.added, "package", "packages"));
        sheet.row(match run.added_detail.as_str() {
            "" => row,
            detail => row.detail(detail),
        });
    }
    if let Some(at) = run.bluetooth {
        sheet.row(
            Row::info("bluetooth", "Bluetooth restarted")
                .value(at.format("%H:%M").to_string())
                .tone(Tone::Attention),
        );
    }
    let took = Row::info("took", "Took");
    sheet.row(match run.end {
        Some(end) => took.value(minutes(run.start, end)),
        None => took.value("still going"),
    });
    if let Some(after) = run.after_startup {
        sheet.row(Row::info("startup", "After startup").value(format!("{after} min")));
    }
    sheet
}
