use super::super::{PanelLook, PanelTone};

pub(crate) fn current() -> PanelLook {
    PanelLook {
        tone: PanelTone::Dark,
        update_accent: None,
    }
}
