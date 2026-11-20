use qol_config::pages::Row;
use qol_config::text::plural;

use super::restart::with_error;
use super::settings::press_auto;
use super::Sheet;
use crate::cleanup::{primary, Live, Origin, State};
use crate::session::{Op, Session};
use crate::updates::AutoKey;

pub(super) const KERNELS: &str = "kernels";
const NAME: &str = "Old kernels";
const NONE: &str = "Nothing removes them on its own here";

pub(super) fn row(sheet: &mut Sheet, session: &Session) {
    let row = Row::new(KERNELS, NAME);
    let Some(live) = primary(session.switches()) else {
        sheet.row(row.detail(NONE).value("kept"));
        return;
    };
    let row = row.value(if live.state.removing() {
        live.switch.when
    } else {
        "kept"
    });
    sheet.row(if session.locked() {
        row
    } else {
        row.open(KERNELS)
    });
}

pub(super) fn page(session: &Session) -> Option<Sheet> {
    let first = session.switches().first()?;
    if session.locked() {
        return None;
    }
    let mut sheet = Sheet::new(KERNELS, "old kernels");
    sheet.group("removes on its own", first.switch.owner.colophon);
    for live in session.switches() {
        let key = AutoKey::Switch(live.switch);
        let on = live.state.on();
        let row = with_error(
            Row::new(key.id(), live.switch.name)
                .detail(detail(live, session))
                .toggle(on)
                .busy(session.busy.contains(&Op::Auto(key))),
            session,
            Op::Auto(key),
        );
        press_auto(&mut sheet, session, row, key, !on);
    }
    Some(sheet)
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
            let count = survey.amount(kind)?.count;
            (count > 0).then(|| {
                format!(
                    "{} {} never removed.",
                    plural(count, kind.noun.one, kind.noun.many),
                    if count == 1 { "is" } else { "are" }
                )
            })
        })
}
