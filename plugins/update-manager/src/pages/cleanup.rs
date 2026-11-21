use std::cmp::Reverse;

use qol_config::pages::{Row, Tone};
use qol_config::text::{format_bytes, plural};

use super::restart::with_error;
use super::settings::press_auto;
use super::Sheet;
use crate::cleanup::{empty, leftovers, why, Consent, Found, Live, Origin, Size, State, Survey};
use crate::session::{Act, Op, Session};
use crate::updates::AutoKey;

pub(super) const CLEANUP: &str = "cleanup";
const MORE: &str = "cleanup:more";
const SHOWN: usize = 6;

fn here(session: &Session) -> bool {
    !leftovers().is_empty() || !session.switches().is_empty()
}

pub(super) fn row(sheet: &mut Sheet, session: &Session) {
    if !here(session) {
        return;
    }
    let row = Row::new(CLEANUP, "Cleanup");
    let row = match session.survey.as_ref() {
        Some(survey) => {
            let list = listed(survey);
            row.value(if list.is_empty() {
                "nothing to remove".to_string()
            } else {
                worth(survey, &list)
            })
        }
        None => row.busy(true),
    };
    sheet.row(row.open(CLEANUP));
}

pub(super) fn pages(session: &Session) -> Vec<Sheet> {
    if !here(session) {
        return Vec::new();
    }
    let busy = session.busy.contains(&Op::Remove);
    let mut sheet = Sheet::new(CLEANUP, "cleanup");
    sheet.group("free space", "what can go now, largest first.");
    if let Some(error) = session.errors.get(&Op::Remove) {
        sheet.row(
            Row::new("error", "Could not remove everything")
                .detail(error.as_str())
                .tone(Tone::Attention)
                .attention(true),
        );
    }
    let mut more = None;
    match session.survey.as_ref() {
        None => sheet.row(Row::new("measuring", "Measuring what can go").busy(true)),
        Some(survey) => {
            let list = listed(survey);
            if list.is_empty() {
                sheet.row(Row::new("nothing", "Nothing to remove"));
            }
            let all: Vec<&Found> = list
                .iter()
                .copied()
                .filter(|found| why(found.leftover).is_none())
                .collect();
            if !all.is_empty() {
                let row = Row::new("all", "Remove all")
                    .value(worth(survey, &all))
                    .busy(busy);
                let row = if all.len() < list.len() {
                    row.detail("Leaves out what asks first")
                } else {
                    row
                };
                sheet.press(
                    row.destroy("remove", "remove all"),
                    Act::Remove(
                        all.iter().map(|found| found.leftover).collect(),
                        Consent::All,
                    ),
                );
            }
            for found in list.iter().take(SHOWN) {
                kind(&mut sheet, found, busy);
            }
            if list.len() > SHOWN {
                sheet.row(Row::new("more", format!("{} more", list.len() - SHOWN)).open(MORE));
                let mut rest = Sheet::new(MORE, "more");
                rest.group("more", "the smaller ones, largest first.");
                for found in &list[SHOWN..] {
                    kind(&mut rest, found, busy);
                }
                more = Some(rest);
            }
        }
    }
    switches(&mut sheet, session);
    let mut sheets = vec![sheet];
    sheets.extend(more);
    sheets
}

fn listed(survey: &Survey) -> Vec<&Found> {
    let mut list: Vec<&Found> = survey.found.iter().filter(|found| !empty(found)).collect();
    list.sort_by_key(|found| Reverse(bytes(found)));
    list
}

fn bytes(found: &Found) -> Option<u64> {
    match &found.amount.as_ref()?.size {
        Size::Exact(bytes) | Size::Estimate { bytes, .. } => Some(*bytes),
        Size::Unknown(_) => None,
    }
}

fn worth(survey: &Survey, found: &[&Found]) -> String {
    let total = Survey {
        at: survey.at,
        stamp: Vec::new(),
        found: found.iter().map(|found| (*found).clone()).collect(),
    }
    .total();
    let size = format_bytes(total.bytes);
    match (total.unknown, total.bytes, total.estimate) {
        (0, _, false) => size,
        (0, _, true) => format!("about {size}"),
        (_, 0, _) => "size unknown".to_string(),
        (_, _, false) => format!("{size} and more"),
        (_, _, true) => format!("about {size} and more"),
    }
}

fn size(found: &Found) -> String {
    match found.amount.as_ref().map(|amount| &amount.size) {
        Some(Size::Exact(bytes)) => format_bytes(*bytes),
        Some(Size::Estimate { bytes, .. }) => format!("about {}", format_bytes(*bytes)),
        Some(Size::Unknown(_)) | None => "size unknown".to_string(),
    }
}

fn kind(sheet: &mut Sheet, found: &Found, busy: bool) {
    let leftover = found.leftover;
    let count = found.amount.as_ref().map_or(0, |amount| amount.count);
    let base = if count > 0 {
        plural(count, leftover.noun.one, leftover.noun.many)
    } else {
        leftover.what.to_string()
    };
    let (detail, consent) = match why(leftover) {
        None => (base, Consent::All),
        Some(why) => (format!("{base}. Asks first: they {why}"), Consent::Asked),
    };
    sheet.press(
        Row::new(format!("kind:{}", leftover.id), leftover.name)
            .detail(detail)
            .value(size(found))
            .busy(busy)
            .destroy("remove", format!("remove {}", leftover.name.to_lowercase())),
        Act::Remove(vec![leftover], consent),
    );
}

fn switches(sheet: &mut Sheet, session: &Session) {
    let Some(first) = session.switches().first() else {
        return;
    };
    sheet.group("removes on its own", first.switch.owner.colophon);
    for live in session.switches() {
        let key = AutoKey::Switch(live.switch);
        let on = live.state.on();
        let row = Row::new(key.id(), live.switch.name).detail(detail(live, session));
        if session.locked() {
            sheet.row(row.value(if live.state.removing() {
                live.switch.when
            } else {
                "kept"
            }));
            continue;
        }
        let row = with_error(
            row.toggle(on).busy(session.busy.contains(&Op::Auto(key))),
            session,
            Op::Auto(key),
        );
        press_auto(sheet, session, row, key, !on);
    }
}

fn detail(live: &Live, session: &Session) -> String {
    let words = live.switch.words;
    match live.state {
        State::On { idle: false } => words.on.into(),
        State::On { idle: true } => words.idle.into(),
        State::Off | State::Unknown(_) => kept(live, session).unwrap_or_else(|| words.off.into()),
    }
}

fn kept(live: &Live, session: &Session) -> Option<String> {
    let survey = session.survey.as_ref()?;
    live.switch
        .covers
        .iter()
        .filter(|kind| kind.origin == Origin::ByHand)
        .find_map(|kind| {
            let count = survey.amount(kind)?.present;
            (count > 0).then(|| {
                format!(
                    "{} {} never removed.",
                    plural(count, kind.noun.one, kind.noun.many),
                    if count == 1 { "is" } else { "are" }
                )
            })
        })
}
