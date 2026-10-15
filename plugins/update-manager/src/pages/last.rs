use chrono::NaiveDateTime;
use qol_config::pages::Row;

use super::restart::finish_names;
use super::words::{cap, heading, minutes, plural};
use super::Sheet;
use crate::session::{Act, Session};
use crate::updates::{Run, RunBy};

const SHOWN: usize = 2;

pub(super) fn line(run: &Run) -> String {
    let who = match &run.by {
        RunBy::Automatic => "Mint".to_string(),
        RunBy::Qol => "You, through qol,".to_string(),
        RunBy::Person(name) => cap(name),
    };
    let what = if run.upgraded > 0 {
        format!("upgraded {}", plural(run.upgraded, "package", "packages"))
    } else {
        format!("added {}", plural(run.added, "package", "packages"))
    };
    match run.bluetooth {
        Some(at) => format!(
            "{who} {what}. Bluetooth restarted at {}.",
            at.format("%H:%M")
        ),
        None => format!("{who} {what}."),
    }
}

fn colophon(run: &Run) -> String {
    match &run.by {
        RunBy::Automatic => match run.after_startup {
            Some(minutes) => format!("mint's automatic upgrades, {minutes} min after startup."),
            None => "mint's automatic upgrades.".to_string(),
        },
        RunBy::Qol => "installed by you, through qol.".to_string(),
        RunBy::Person(name) => format!("installed by {name}."),
    }
}

pub(super) fn page(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("last", "last run");
    if session.host.runs.is_empty() {
        sheet.group("last run", "mint's log lists no installs.");
        sheet.row(Row::info("none", "No run yet").detail("Nothing installed since the log began"));
    }
    for (index, run) in session.host.runs.iter().take(SHOWN).enumerate() {
        sheet.group(heading(run.start, now).to_lowercase(), colophon(run));
        if run.upgraded > 0 {
            let groups: Vec<String> = run
                .groups
                .iter()
                .map(|(name, count)| format!("{name} {count}"))
                .collect();
            sheet.row(
                Row::info(
                    format!("upgraded:{index}"),
                    format!("{} upgraded", plural(run.upgraded, "package", "packages")),
                )
                .detail(groups.join(", "))
                .value(run.upgraded.to_string()),
            );
        }
        if run.added > 0 {
            sheet.row(
                Row::info(
                    format!("added:{index}"),
                    format!("{} added", plural(run.added, "package", "packages")),
                )
                .detail(run.added_detail.as_str())
                .value(run.added.to_string()),
            );
        }
        if let Some(at) = run.bluetooth {
            sheet.row(
                Row::info(
                    format!("bluetooth:{index}"),
                    format!("Bluetooth restarted at {}", at.format("%H:%M")),
                )
                .detail("The bluez upgrade restarted it; connected devices dropped")
                .attention(true),
            );
        }
        if let Some(end) = run.end {
            sheet.row(
                Row::info(
                    format!("took:{index}"),
                    format!("Took {}", minutes(run.start, end)),
                )
                .detail(format!(
                    "{} to {}",
                    run.start.format("%H:%M"),
                    end.format("%H:%M")
                )),
            );
        }
        if index == 0 && !session.finish().is_empty() {
            let one = session.finish().len() == 1;
            sheet.row(
                Row::info("finish", "Restart to finish")
                    .detail(format!(
                        "{} finish{} at the restart",
                        finish_names(session),
                        if one { "es" } else { "" }
                    ))
                    .value("waiting"),
            );
        }
    }
    if let Some(log) = session.host.log.as_ref() {
        sheet.group("log", "what apt wrote down.");
        sheet.press(
            Row::new("log", "Mint's log")
                .detail(log.as_str())
                .button("open"),
            Act::Open(log.clone()),
        );
    }
    sheet
}
