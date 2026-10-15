use chrono::NaiveDateTime;
use qol_config::pages::{Go, Row, Tone};

use super::restart::{finish_names, planned_at, with_error};
use super::words::{cap, night_of, on_off};
use super::Sheet;
use crate::session::{Act, Helper, Op, Proposal, Session, MINT_DEFAULT};
use crate::updates::{day, when, AutoKey, Clock, Hours, RunBy};

const LOCKED: &str = "Needs an administrator once, to pause Mint's run.";
const ASKING: &str = "Waiting for the password, to hold Mint's run";

pub(super) fn hours_value(session: &Session) -> String {
    let hours = session.hours();
    format!("{} to {}", hours.from, hours.until)
}

fn could_not(row: Row, refusal: &impl std::fmt::Display) -> Row {
    row.detail(format!("Could not hold: {refusal}."))
        .value("could not hold")
        .tone(Tone::Attention)
        .attention(true)
        .toggle(true)
}

pub(super) fn rules(sheet: &mut Sheet, session: &Session) {
    if session.mint().is_none() {
        sheet.row(
            Row::info("rules", "Nothing installs on its own here")
                .detail("So there is nothing for qol to hold"),
        );
        return;
    }
    let failed = match &session.rules.helper {
        Helper::Failed(refusal) => Some(refusal),
        _ => None,
    };
    let asking = matches!(session.rules.helper, Helper::Off | Helper::Starting);
    let (guard, hold) = (session.rules.guard, session.rules.hold);
    let row = Row::new("guard", "Wait while in use");
    let row = match failed {
        Some(refusal) if guard => could_not(row, refusal),
        _ if guard => row
            .detail(match session.in_use.as_ref() {
                _ if asking => ASKING.to_string(),
                Some(in_use) => format!("{} is playing, so updates wait", in_use.label),
                None => "Updates wait while something plays".to_string(),
            })
            .toggle(true)
            .busy(asking),
        _ if session.locked() => row.detail(LOCKED).toggle(false),
        _ => row
            .detail("Off. Updates install at their time")
            .toggle(false),
    };
    sheet.press(row, Act::Guard);
    let row = Row::new("hold", "Hold all updates");
    let (row, act) = match failed {
        Some(refusal) if hold => (could_not(row, refusal), Act::Hold(false)),
        _ if hold => (
            row.detail(if asking {
                ASKING
            } else {
                "Nothing installs until you switch it off"
            })
            .toggle(true)
            .busy(asking),
            Act::Hold(false),
        ),
        _ if session.locked() => (row.detail(LOCKED).toggle(false), Act::Hold(true)),
        _ => (
            row.detail("Nothing installs until you switch it off")
                .toggle(false),
            Act::Hold(true),
        ),
    };
    sheet.press(row, act);
    if failed.is_some() && (guard || hold) {
        sheet.press(
            Row::new("retry", "Try again")
                .detail("Asks for the password again, to hold Mint's run")
                .button("retry"),
            Act::Retry,
        );
    }
}

pub(super) fn pages(session: &Session, now: NaiveDateTime) -> Vec<Sheet> {
    let mut sheets = vec![changes(session, now)];
    if let Some(sheet) = ask(session, now) {
        sheets.push(sheet);
    }
    if session.mint().is_none() || session.locked() {
        return sheets;
    }
    sheets.push(hours(session, now));
    sheets.push(from(session));
    sheets.push(until(session));
    sheets.push(auto(session));
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

fn press_auto(sheet: &mut Sheet, session: &Session, row: Row, key: AutoKey, on: bool) {
    if session.borrowed {
        sheet.press_then(row, Act::Ask(Proposal::Auto(key)), Go::Open("ask".into()));
    } else {
        sheet.press(row, Act::Auto(key, on));
    }
}

fn ours(session: &Session) -> bool {
    session.mint().is_some_and(|mint| mint.ours)
}

fn hours(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("hours", "update hours");
    let hours = session.hours();
    let busy = session.busy.contains(&Op::Hours);
    sheet.group(
        "update hours",
        "when mint installs system packages on its own.",
    );
    if let Some(error) = session.errors.get(&Op::Hours) {
        sheet.row(
            Row::info("error", "Could not change the hours")
                .detail(error.as_str())
                .tone(Tone::Attention)
                .attention(true),
        );
    }
    sheet.row(
        Row::new("from", "From")
            .detail("Mint's run starts at a random minute between From and Until")
            .value(hours.from.to_string())
            .busy(busy)
            .open("from"),
    );
    sheet.row(
        Row::new("until", "Until")
            .detail("The latest start. A long run can finish after it.")
            .value(hours.until.to_string())
            .busy(busy)
            .open("until"),
    );
    let like = session
        .host
        .runs
        .iter()
        .find(|run| run.by == RunBy::Automatic && run.after_startup.is_some())
        .map(|run| format!("Mint's default, like {}", when(run.start, now)));
    let startup = Row::new("startup", "Also after each startup")
        .detail(if hours.startup {
            like.unwrap_or_else(|| "Mint's default".to_string())
        } else {
            "Off. A missed night waits for the next.".to_string()
        })
        .toggle(hours.startup)
        .busy(busy);
    press_hours(
        &mut sheet,
        session,
        startup,
        target(Hours {
            startup: !hours.startup,
            ..hours
        }),
        None,
    );
    if ours(session) {
        sheet.group(
            "what changes",
            "your setting, written to mint's timer. it stays when qol stops.",
        );
        sheet.row(
            Row::info("timer", "Mint's timer")
                .detail(format!(
                    "Runs {} to {}{}, through a systemd override",
                    hours.from,
                    hours.until,
                    if hours.startup {
                        ""
                    } else {
                        ", and no run after startup"
                    }
                ))
                .value("stays"),
        );
        if let (Some(at), true) = (session.host.restart_at, session.moved()) {
            sheet.row(
                Row::info("moved", format!("{}'s restart", cap(&night_of(at, now))))
                    .detail("Moved after the update hours, and still waits if Mint's run is going")
                    .value(at.format("%H:%M").to_string()),
            );
        }
        let row = Row::new("default", "Set back to Mint's default")
            .detail("Around midnight, and after each startup")
            .button("set back");
        press_hours(&mut sheet, session, row, None, None);
    }
    sheet
}

fn from(session: &Session) -> Sheet {
    let hours = session.hours();
    let mut sheet = Sheet::new("from", "from");
    sheet.group(
        "updates from",
        "mint's run starts at a random minute after this. until moves with it.",
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

fn auto(session: &Session) -> Sheet {
    let auto = session.auto();
    let mut sheet = Sheet::new("auto", "install on their own");
    sheet.group(
        "install on their own",
        if session.borrowed {
            "the owner's switches. a change asks first, and stays after you leave."
        } else {
            "mint's own switches. they stay when qol stops."
        },
    );
    for key in AutoKey::ALL {
        let on = auto.get(key);
        let detail = match key {
            AutoKey::Packages if on => {
                "Mint's automatic upgrades: every package, in the update hours"
            }
            AutoKey::Packages => "Off. Packages install when you say",
            AutoKey::Spices => {
                "Mint's switch: they update when you log in; Cinnamon restarts if any did"
            }
            AutoKey::Flatpak => "Mint's switch: apps and runtimes update when you log in",
        };
        let row = with_error(
            Row::new(key.id(), key.name())
                .detail(detail)
                .toggle(on)
                .busy(session.busy.contains(&Op::Auto(key))),
            session,
            Op::Auto(key),
        );
        press_auto(&mut sheet, session, row, key, !on);
    }
    sheet.row(
        Row::info("firmware", "Firmware")
            .detail("fwupd never installs on its own")
            .value("when you say"),
    );
    sheet
}

fn changes(session: &Session, now: NaiveDateTime) -> Sheet {
    let mut sheet = Sheet::new("changes", "what qol changed");
    sheet.group(
        "stays when qol stops",
        "your settings, written with mint's own tools.",
    );
    let mut any = false;
    if ours(session) {
        any = true;
        let detail = match session.journal.hours {
            Some(change) => format!(
                "Set by you {}. Was {}.",
                day(change.at, now),
                change.was.unwrap_or(MINT_DEFAULT).span()
            ),
            None => "Set through qol".to_string(),
        };
        let row = Row::new("hours", "Update hours")
            .detail(detail)
            .value(hours_value(session))
            .button("set back");
        press_hours(&mut sheet, session, row, None, None);
    }
    if let Some(at) = session.host.restart_at {
        any = true;
        let detail = match session.journal.restart {
            Some(set) => format!(
                "Set by you {}. Happens even if qol has stopped; waits for Mint's run.",
                day(set, now)
            ),
            None => "Happens even if qol has stopped; waits for Mint's run.".to_string(),
        };
        sheet.press(
            Row::new("restart", format!("Restart {}", night_of(at, now)))
                .detail(detail)
                .value(at.format("%H:%M").to_string())
                .button("cancel"),
            Act::Tonight(false),
        );
    }
    let auto = session.auto();
    for (key, was) in session.auto_changed() {
        any = true;
        let detail = match session.journal.auto.iter().find(|change| change.key == key) {
            Some(change) => format!("Set by you {}. Was {}.", day(change.at, now), on_off(was)),
            None => format!("Was {}.", on_off(was)),
        };
        let row = Row::new(
            format!("auto:{}", key.id()),
            format!("{} on their own", key.name()),
        )
        .detail(detail)
        .value(on_off(auto.get(key)))
        .button("set back");
        press_auto(&mut sheet, session, row, key, was);
    }
    if !any {
        sheet.row(
            Row::info("nothing", "Nothing yet")
                .detail("A setting you change here is listed until you set it back"),
        );
    }
    sheet.group(
        "ends when qol stops",
        "qol's own rules. after a crash, they end at the next startup.",
    );
    rules(&mut sheet, session);
    sheet
}

fn lower(key: AutoKey) -> &'static str {
    match key {
        AutoKey::Packages => "system packages",
        AutoKey::Spices => "Cinnamon spices",
        AutoKey::Flatpak => "Flatpak",
    }
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
                Row::info("now", "Now")
                    .detail("Mint's timer, as the owner left it")
                    .value(session.hours().span()),
            );
            sheet.row(
                Row::info("after", "After")
                    .detail("Stays until the owner changes it back")
                    .value(after.span()),
            );
            sheet.group("choose", "esc cancels too.");
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
                    .detail("Writes Mint's timer. It stays after you leave.")
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
                Row::info("at", "At")
                    .detail(match session.run_at() {
                        Some(run) => format!("After {run} ends"),
                        None => cap(&word),
                    })
                    .value(clock.clone()),
            );
            let names = finish_names(session);
            sheet.row(
                Row::info("finishes", "Finishes").detail(if names.is_empty() {
                    "Nothing waits for it".to_string()
                } else {
                    names
                }),
            );
            sheet.group("choose", "esc cancels too.");
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
            let on = session.auto().get(key);
            sheet = Sheet::new("ask", "the owner's switch");
            sheet.group(
                format!("change the owner's {} switch?", lower(key)),
                "this computer is not yours. the change stays after you leave.",
            );
            sheet.row(
                Row::info("now", "Now")
                    .detail("As the owner left it")
                    .value(on_off(on)),
            );
            sheet.row(
                Row::info("after", "After")
                    .detail("Stays until the owner changes it back")
                    .value(on_off(!on)),
            );
            sheet.group("choose", "esc cancels too.");
            sheet.row(
                Row::new("cancel", "Cancel")
                    .detail("Leave the owner's switch as it is")
                    .back(),
            );
            sheet.press_then(
                Row::new("change", "Change the owner's switch")
                    .detail("Writes Mint's own setting. It stays after you leave.")
                    .button("change"),
                Act::Auto(key, !on),
                Go::Back,
            );
        }
    }
    Some(sheet)
}
