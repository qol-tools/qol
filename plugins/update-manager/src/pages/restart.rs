use chrono::NaiveDateTime;
use qol_config::pages::{Go, Row, Tone};

use super::words::{join, night_of, title};
use super::Sheet;
use crate::session::{Act, Op, Proposal, Session};

pub(super) fn finish_word(session: &Session) -> Option<String> {
    match session.finish().as_slice() {
        [] => None,
        [one] => Some(format!("{} finishes", one.label.to_lowercase())),
        many => Some(format!("{} updates finish", many.len())),
    }
}

pub(super) fn finish_names(session: &Session) -> String {
    let names: Vec<String> = session.finish().into_iter().map(|row| row.name).collect();
    title(&join(&names))
}

pub(super) fn planned_at(session: &Session, now: NaiveDateTime) -> NaiveDateTime {
    session
        .host
        .restart_at
        .unwrap_or_else(|| session.planned().next_after(now))
}

pub(super) fn with_error(row: Row, session: &Session, op: Op) -> Row {
    match session.errors.get(&op) {
        Some(error) => row
            .detail(error.as_str())
            .tone(Tone::Attention)
            .attention(true),
        None => row,
    }
}

pub(super) fn pages(session: &Session, now: NaiveDateTime) -> Vec<Sheet> {
    if session.finish().is_empty() && session.host.restart_at.is_none() {
        return Vec::new();
    }
    vec![restart(session, now), time(session), restart_now(session)]
}

fn restart(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("restart", "restart");
    sheet.group(
        "restart",
        match finish_word(session) {
            Some(word) => format!("{word} when the computer restarts."),
            None => "nothing is left to finish.".to_string(),
        },
    );
    let run_at = session.run_at();
    let time = if session.moved() {
        "After your update hours. It waits if Mint's run is still going.".to_string()
    } else if let Some(run) = run_at.as_deref() {
        format!("If {run} is still going then, the restart waits for it")
    } else {
        "When the restart happens".to_string()
    };
    sheet.row(
        Row::new("time", "Time")
            .detail(time)
            .value(session.planned().to_string())
            .open("time"),
    );
    tonight(&mut sheet, session, now);
    let now_detail = match session.in_use.as_ref() {
        Some(in_use) => format!(
            "Asks first. {} stops playing and open apps close.",
            in_use.label
        ),
        None => "Asks first. Open apps close.".to_string(),
    };
    sheet.row(with_error(
        Row::new("now", "Restart now")
            .detail(now_detail)
            .button("restart")
            .busy(session.busy.contains(&Op::RestartNow))
            .open("restart-now"),
        session,
        Op::RestartNow,
    ));
    sheet.group("what finishes", "installed, waiting for the restart.");
    for (index, row) in session.finish().into_iter().enumerate() {
        sheet.row(Row::info(format!("finish:{index}"), row.label).detail(row.finish));
    }
    if let (Some(_), Some(run)) = (session.host.restart_at, run_at.as_deref()) {
        for update in session
            .available()
            .into_iter()
            .filter(|update| update.restart && update.unattended && session.job.is_none())
        {
            sheet.row(
                Row::info(format!("next:{}", update.id), update.label.as_str()).detail(format!(
                    "{run} installs it before the restart, so it finishes too"
                )),
            );
        }
    }
    sheet
}

fn tonight(sheet: &mut Sheet, session: &Session, now: NaiveDateTime) {
    let set = session.host.restart_at.is_some();
    let at = planned_at(session, now);
    let run_at = session.run_at();
    let detail = if set && session.moved() {
        "Set. Moved after your update hours. Switch off to cancel.".to_string()
    } else if set {
        format!(
            "Set.{} Switch off to cancel.",
            run_at
                .as_deref()
                .map(|run| format!(" After {run} ends."))
                .unwrap_or_default()
        )
    } else if session.locked() {
        "Asks for an administrator's password once, so it can wait for Mint's run.".to_string()
    } else {
        format!(
            "{}Stays set if qol stops.",
            run_at
                .as_deref()
                .map(|run| format!("After {run} ends. "))
                .unwrap_or_default()
        )
    };
    let row = with_error(
        Row::new(
            "tonight",
            format!("Restart {} at {}", night_of(at, now), at.format("%H:%M")),
        )
        .detail(detail)
        .toggle(set)
        .busy(session.busy.contains(&Op::Tonight)),
        session,
        Op::Tonight,
    );
    if session.borrowed && !set {
        sheet.press_then(row, Act::Ask(Proposal::Tonight), Go::Open("ask".into()));
    } else {
        sheet.press(row, Act::Tonight(!set));
    }
}

fn time(session: &Session) -> Sheet {
    let mut sheet = Sheet::new("time", "time");
    sheet.group(
        "restart at",
        match session.run_at() {
            Some(run) => format!(
                "after {}. it waits if the run is still going.",
                run.to_lowercase()
            ),
            None => "the computer restarts at this time.".to_string(),
        },
    );
    let planned = session.planned();
    for clock in session.choices() {
        let id = format!("time:{clock}");
        if clock == planned {
            sheet.focus(id.clone());
        }
        sheet.press_then(
            Row::new(id, clock.to_string()).check(clock == planned),
            Act::Time(clock),
            Go::Back,
        );
    }
    sheet
}

fn restart_now(session: &Session) -> Sheet {
    let mut sheet = Sheet::new("restart-now", "restart now");
    sheet.group(
        "restart now?",
        match finish_word(session) {
            Some(word) => format!("{word} with it. about a minute."),
            None => "about a minute.".to_string(),
        },
    );
    if let Some(in_use) = session.in_use.as_ref() {
        sheet.row(
            Row::info("in-use", format!("{} stops playing", in_use.label))
                .detail(if in_use.bluetooth {
                    "It plays over Bluetooth"
                } else {
                    "It plays on this computer"
                })
                .value("stops")
                .attention(true),
        );
    }
    sheet.row(
        Row::info("apps", "Open apps close")
            .detail("Save your work first.")
            .value("closes"),
    );
    sheet.row(if session.borrowed {
        Row::info("qol", "qol stops with it").detail("Start it again after the restart")
    } else if session.rules.guard || session.rules.hold {
        Row::info("qol", "qol starts again after it").detail("Its rules start off again")
    } else {
        Row::info("qol", "qol starts again after it").detail("With its rules as you left them")
    });
    sheet.group("choose", "esc cancels too.");
    sheet.row(
        Row::new("cancel", "Cancel")
            .detail("Back to the page, nothing restarts")
            .back(),
    );
    sheet.press(
        Row::new("go", "Restart now")
            .detail("Right away")
            .button("restart"),
        Act::RestartNow,
    );
    sheet
}
