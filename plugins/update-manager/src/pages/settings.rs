use chrono::NaiveDateTime;
use qol_config::pages::{Go, Row, Tone};

use super::restart::{finish_names, planned_at, with_error};
use super::words::{cap, night_of, on_off};
use super::Sheet;
use crate::session::{Act, Helper, Op, Proposal, Session, MINT_DEFAULT};
use crate::updates::{sources, when, AutoKey, Clock, Hours, RunBy};

pub(super) const AUTO: &str = "auto";
const LOCKED: &str = "Needs an administrator once, to pause the automatic update.";
const ASKING: &str = "Waiting for the password, to hold the automatic update";

fn hours_value(session: &Session) -> String {
    let hours = session.hours();
    format!("{} and {}", hours.from, hours.until)
}

fn could_not(row: Row, refusal: &impl std::fmt::Display) -> Row {
    row.detail(format!("{}.", cap(&refusal.to_string())))
        .value("could not hold")
        .tone(Tone::Attention)
        .attention(true)
        .toggle(true)
}

fn rules(sheet: &mut Sheet, session: &Session) {
    let failed = match &session.rules.helper {
        Helper::Failed(refusal) => Some(refusal),
        Helper::Off | Helper::Starting | Helper::Ready => None,
    };
    let asking = matches!(session.rules.helper, Helper::Off | Helper::Starting);
    let (guard, hold) = (session.rules.guard, session.rules.hold);
    let row = Row::new("hold", "Pause all");
    let (row, act) = match failed {
        Some(refusal) if hold => (could_not(row, refusal), Act::Hold(false)),
        _ if hold && asking => (row.detail(ASKING).toggle(true).busy(true), Act::Hold(false)),
        _ if hold => (row.toggle(true), Act::Hold(false)),
        _ if session.locked() => (row.detail(LOCKED).toggle(false), Act::Hold(true)),
        _ => (row.toggle(false), Act::Hold(true)),
    };
    sheet.press(row, act);
    let row = Row::new("guard", "Wait while sound plays");
    let row = match failed {
        Some(refusal) if guard => could_not(row, refusal),
        _ if guard && asking => row.detail(ASKING).toggle(true).busy(true),
        _ if guard => match session.in_use.as_ref() {
            Some(in_use) => row.detail(format!("{} is playing, so updates wait", in_use.label)),
            None => row,
        }
        .toggle(true),
        _ if session.locked() => row.detail(LOCKED).toggle(false),
        _ => row.toggle(false),
    };
    sheet.press(row, Act::Guard);
    if failed.is_some() && (guard || hold) {
        sheet.press(
            Row::new("retry", "Try again")
                .detail("Asks for the password again, to hold the automatic update")
                .button("retry"),
            Act::Retry,
        );
    }
}

pub(super) fn pages(session: &Session, now: NaiveDateTime) -> Vec<Sheet> {
    let mut sheets = Vec::new();
    if let Some(sheet) = ask(session, now) {
        sheets.push(sheet);
    }
    if session.mint().is_none() {
        return sheets;
    }
    sheets.push(auto(session));
    sheets.push(hours(session, now));
    if !session.locked() {
        sheets.push(from(session));
        sheets.push(until(session));
    }
    sheets
}

fn target(hours: Hours) -> Option<Hours> {
    (hours != MINT_DEFAULT).then_some(hours)
}

fn press_hours(
    sheet: &mut Sheet,
    session: &Session,
    row: Row,
    hours: Option<Hours>,
    go: Option<Go>,
) {
    if session.borrowed {
        let go = match go {
            Some(Go::Back) => Go::Replace("ask".into()),
            _ => Go::Open("ask".into()),
        };
        sheet.press_then(row, Act::Ask(Proposal::Hours(hours)), go);
        return;
    }
    match go {
        Some(go) => sheet.press_then(row, Act::Hours(hours), go),
        None => sheet.press(row, Act::Hours(hours)),
    }
}

pub(super) fn press_auto(sheet: &mut Sheet, session: &Session, row: Row, key: AutoKey, on: bool) {
    if session.borrowed {
        sheet.press_then(row, Act::Ask(Proposal::Auto(key)), Go::Open("ask".into()));
    } else {
        sheet.press(row, Act::Auto(key, on));
    }
}

fn ours(session: &Session) -> bool {
    session.mint().is_some_and(|mint| mint.ours)
}

fn auto(session: &Session) -> Sheet {
    let auto = session.auto();
    let locked = session.locked();
    let mut sheet = Sheet::new(AUTO, "installs on its own");
    sheet.group("what", "what may install by itself.");
    for source in sources() {
        let key = source.key;
        let on = auto.get(key);
        let row = Row::new(key.id(), source.name).detail(source.when);
        if locked {
            sheet.row(row.value(on_off(on)));
            continue;
        }
        let row = with_error(
            row.toggle(on).busy(session.busy.contains(&Op::Auto(key))),
            session,
            Op::Auto(key),
        );
        press_auto(&mut sheet, session, row, key, !on);
    }
    sheet.group(
        "when",
        "when those installs may run, and what holds them back.",
    );
    let busy = session.busy.contains(&Op::Hours);
    sheet.row(
        with_error(
            Row::new("hours", "Between")
                .value(hours_value(session))
                .busy(busy),
            session,
            Op::Hours,
        )
        .open("hours"),
    );
    rules(&mut sheet, session);
    if ours(session) && !session.locked() {
        let row = Row::new("default", "Set back to the default")
            .detail("Around midnight, and after each startup")
            .button("set back")
            .busy(busy);
        press_hours(&mut sheet, session, row, None, None);
    }
    sheet
}

fn hours(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("hours", "between");
    let hours = session.hours();
    let locked = session.locked();
    let busy = session.busy.contains(&Op::Hours);
    sheet.group(
        "between",
        "the automatic update starts at a random minute in this window.",
    );
    if let Some(error) = session.errors.get(&Op::Hours) {
        sheet.row(
            Row::new("error", "Could not change the hours")
                .detail(error.as_str())
                .tone(Tone::Attention)
                .attention(true),
        );
    }
    let from = Row::new("from", "From")
        .detail("The earliest start")
        .value(hours.from.to_string())
        .busy(busy);
    let until = Row::new("until", "Until")
        .detail("The latest start. A long run can finish after it.")
        .value(hours.until.to_string())
        .busy(busy);
    if locked {
        sheet.row(from);
        sheet.row(until);
    } else {
        sheet.row(from.open("from"));
        sheet.row(until.open("until"));
    }
    let like = session
        .host
        .runs
        .iter()
        .find(|run| run.by == RunBy::Automatic && run.after_startup.is_some())
        .map(|run| format!("The default, like {}", when(run.start, now)));
    let startup = Row::new("startup", "Also after each startup").detail(if hours.startup {
        like.unwrap_or_else(|| "The default".to_string())
    } else {
        "A missed night waits for the next.".to_string()
    });
    if locked {
        sheet.row(startup.value(on_off(hours.startup)));
    } else {
        press_hours(
            &mut sheet,
            session,
            startup.toggle(hours.startup).busy(busy),
            target(Hours {
                startup: !hours.startup,
                ..hours
            }),
            None,
        );
    }
    if let (Some(at), true, true) = (session.host.restart_at, session.moved(), ours(session)) {
        sheet.row(
            Row::new("moved", format!("{}'s restart", cap(&night_of(at, now))))
                .detail("Moved after these hours, and still waits if the automatic update is going")
                .value(at.format("%H:%M").to_string()),
        );
    }
    sheet
}

fn from(session: &Session) -> Sheet {
    let hours = session.hours();
    let mut sheet = Sheet::new("from", "from");
    sheet.group(
        "updates from",
        "the automatic update starts at a random minute after this. until moves with it.",
    );
    for hour in 0..24 {
        let clock = Clock::hm(hour, 0);
        let id = format!("from:{clock}");
        if clock == hours.from {
            sheet.focus(id.clone());
        }
        let row = Row::new(id, clock.to_string()).check(clock == hours.from);
        let picked = Hours {
            from: clock,
            until: clock.plus(hours.length()),
            ..hours
        };
        press_hours(&mut sheet, session, row, target(picked), Some(Go::Back));
    }
    sheet
}

fn until(session: &Session) -> Sheet {
    let hours = session.hours();
    let mut sheet = Sheet::new("until", "until");
    sheet.group("until", "the latest start. a long run can finish after it.");
    for step in 1..24 {
        let clock = hours.from.plus(step * 60);
        let id = format!("until:{clock}");
        if clock == hours.until {
            sheet.focus(id.clone());
        }
        let row = Row::new(id, clock.to_string()).check(clock == hours.until);
        let picked = Hours {
            until: clock,
            ..hours
        };
        press_hours(&mut sheet, session, row, target(picked), Some(Go::Back));
    }
    sheet
}

fn ask(session: &Session, now: NaiveDateTime) -> Option<Sheet> {
    let proposal = session.ask?;
    let mut sheet;
    match proposal {
        Proposal::Hours(hours) => {
            let after = hours.unwrap_or(MINT_DEFAULT);
            sheet = Sheet::new("ask", "the owner's hours");
            sheet.group(
                "change the owner's update hours?",
                "this computer is not yours. the change stays after you leave.",
            );
            sheet.row(
                Row::new("now", "Now")
                    .detail("As the owner left it")
                    .value(session.hours().span()),
            );
            sheet.row(
                Row::new("after", "After")
                    .detail("Stays until the owner changes it back")
                    .value(after.span()),
            );
            sheet.group("choose", "change them, or leave them as they are.");
            sheet.row(
                Row::new("cancel", "Cancel")
                    .detail("Leave the owner's hours as they are")
                    .back(),
            );
            if !session.rules.hold {
                sheet.press_then(
                    Row::new("hold", "Hold updates while I'm here")
                        .detail(
                            "Nothing installs until qol stops. The owner's hours stay as they are.",
                        )
                        .button("hold"),
                    Act::Hold(true),
                    Go::Back,
                );
            }
            sheet.press_then(
                Row::new("change", "Change the owner's hours")
                    .detail("Writes the system's timer. It stays after you leave.")
                    .button("change"),
                Act::Hours(hours),
                Go::Back,
            );
        }
        Proposal::Tonight => {
            let at = planned_at(session, now);
            let word = night_of(at, now);
            let clock = at.format("%H:%M").to_string();
            sheet = Sheet::new("ask", "the owner's restart");
            sheet.group(
                format!("restart the owner's computer {word}?"),
                "this computer is not yours. the restart happens even after you leave.",
            );
            sheet.row(
                Row::new("at", "At")
                    .detail(match session.run_at() {
                        Some(run) => format!("After {run} ends"),
                        None => cap(&word),
                    })
                    .value(clock.clone()),
            );
            let names = finish_names(session);
            sheet.row(
                Row::new("finishes", "Finishes").detail(if names.is_empty() {
                    "Nothing waits for it".to_string()
                } else {
                    names
                }),
            );
            sheet.group("choose", "set the restart, or leave it.");
            sheet.row(
                Row::new("cancel", "Cancel")
                    .detail(format!("No restart {word}"))
                    .back(),
            );
            sheet.press_then(
                Row::new("set", format!("Restart {word} at {clock}"))
                    .detail("A one-time timer. It stays if qol stops; switch it off to cancel.")
                    .button("set"),
                Act::Tonight(true),
                Go::Back,
            );
        }
        Proposal::Auto(key) => {
            let on = session.switch(key);
            sheet = Sheet::new("ask", "the owner's switch");
            sheet.group(
                format!("change the owner's {} switch?", key.short()),
                "this computer is not yours. the change stays after you leave.",
            );
            sheet.row(
                Row::new("now", "Now")
                    .detail("As the owner left it")
                    .value(on_off(on)),
            );
            sheet.row(
                Row::new("after", "After")
                    .detail("Stays until the owner changes it back")
                    .value(on_off(!on)),
            );
            sheet.group("choose", "change it, or leave it as it is.");
            sheet.row(
                Row::new("cancel", "Cancel")
                    .detail("Leave the owner's switch as it is")
                    .back(),
            );
            sheet.press_then(
                Row::new("change", "Change the owner's switch")
                    .detail(if key.removes() {
                        "Writes the system's own cleanup setting. It stays after you leave."
                    } else {
                        "Writes the system's own install setting. It stays after you leave."
                    })
                    .button("change"),
                Act::Auto(key, !on),
                Go::Back,
            );
        }
    }
    Some(sheet)
}
