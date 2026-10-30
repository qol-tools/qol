use qol_config::pages::{Go, Row, Tone};

use super::Sheet;
use crate::session::{Act, Op, Session};

pub(super) fn page(session: &Session) -> Option<Sheet> {
    let skips = session.skips();
    let busy = session.busy.contains(&Op::Skips);
    let error = session.errors.get(&Op::Skips);
    if skips.is_empty() && !busy && error.is_none() {
        return None;
    }
    let mut sheet = Sheet::new("skipped", "skipped");
    if let Some(error) = error {
        sheet.heading("right now");
        sheet.row(
            Row::info("error", "Could not change the skipped list")
                .detail(error.as_str())
                .tone(Tone::Attention),
        );
    }
    let (versions, never): (Vec<&String>, Vec<&String>) =
        skips.iter().partition(|entry| entry.contains('='));
    let last = skips.len() == 1;
    if !never.is_empty() {
        sheet.heading("never");
        for entry in never {
            let row = Row::new(format!("never:{entry}"), entry.as_str());
            again(&mut sheet, row, entry, busy, last);
        }
    }
    if !versions.is_empty() {
        sheet.heading("this version");
        for entry in versions {
            let (source, version) = entry.split_once('=').unwrap_or((entry.as_str(), ""));
            let row = Row::new(format!("skip:{entry}"), source).value(version);
            again(&mut sheet, row, entry, busy, last);
        }
    }
    if skips.is_empty() {
        sheet.heading("skipped");
        sheet.row(Row::info("none", "Nothing skipped").busy(busy));
    }
    Some(sheet)
}

fn again(sheet: &mut Sheet, row: Row, entry: &str, busy: bool, last: bool) {
    let row = row.button("update again").busy(busy);
    let act = Act::Unskip(entry.to_string());
    if last {
        sheet.press_then(row, act, Go::Back);
    } else {
        sheet.press(row, act);
    }
}
