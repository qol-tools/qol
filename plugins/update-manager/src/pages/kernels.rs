use qol_config::pages::Row;

use super::restart::with_error;
use super::settings::press_auto;
use super::Sheet;
use crate::session::{Op, Session};
use crate::updates::AutoKey;

pub(super) const KERNELS: &str = "kernels";
const NONE: &str = "Nothing removes them on its own here";

pub(super) fn row(sheet: &mut Sheet, session: &Session) {
    let row = Row::new(KERNELS, AutoKey::Kernels.name());
    let Some(cleanup) = session.cleanup() else {
        sheet.row(row.detail(NONE).value("kept"));
        return;
    };
    let row = row.value(if cleanup.removing() {
        cleanup.by.when()
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
    let cleanup = session.cleanup()?;
    if session.locked() {
        return None;
    }
    let mut sheet = Sheet::new(KERNELS, "old kernels");
    sheet.group("removes on its own", cleanup.by.colophon());
    let switch = |sheet: &mut Sheet, key: AutoKey, label: &str, detail: String| {
        let on = session.switch(key);
        let row = with_error(
            Row::new(key.id(), label)
                .detail(detail)
                .toggle(on)
                .busy(session.busy.contains(&Op::Auto(key))),
            session,
            Op::Auto(key),
        );
        press_auto(sheet, session, row, key, !on);
    };
    switch(
        &mut sheet,
        AutoKey::Kernels,
        cleanup.by.label(),
        cleanup.detail().into(),
    );
    if let Some(by_hand) = cleanup.by_hand {
        switch(
            &mut sheet,
            AutoKey::ByHand,
            AutoKey::ByHand.name(),
            by_hand.detail(),
        );
    }
    Some(sheet)
}
