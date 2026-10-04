#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ModifierMapping(Vec<((u16, u16), (u16, u16))>);

impl ModifierMapping {
    pub(super) fn from_pairs(pairs: impl IntoIterator<Item = (i64, i64)>) -> Self {
        Self(
            pairs
                .into_iter()
                .filter_map(|(source, target)| Some((decode(source)?, decode(target)?)))
                .filter(|(source, target)| source != target)
                .collect(),
        )
    }

    pub(super) fn apply(&self, page: u16, usage: u16) -> (u16, u16) {
        self.0
            .iter()
            .find(|(source, _)| *source == (page, usage))
            .map_or((page, usage), |&(_, target)| target)
    }
}

fn decode(value: i64) -> Option<(u16, u16)> {
    let page = u16::try_from(value >> 32).ok()?;
    let usage = u16::try_from(value & 0xFFFF_FFFF).ok()?;
    Some((page, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn packed(page: i64, usage: i64) -> i64 {
        (page << 32) | usage
    }

    fn settings() -> ModifierMapping {
        ModifierMapping::from_pairs([
            (packed(0x07, 0xE2), packed(0x07, 0xE3)),
            (packed(0x07, 0xE3), packed(0x07, 0xE2)),
            (packed(0x07, 0x39), packed(0xFF, 0x03)),
            (packed(0xFF, 0x03), packed(0x07, 0xE4)),
            (packed(0xFF00, 0x03), packed(0x07, 0xE0)),
            (packed(0x07, 0xE0), packed(0x07, 0xE0)),
        ])
    }

    #[test]
    fn system_settings_pairs_move_each_key_once() {
        let mapping = settings();
        assert_eq!(mapping.apply(0x07, 0xE2), (0x07, 0xE3));
        assert_eq!(mapping.apply(0x07, 0xE3), (0x07, 0xE2));
        assert_eq!(mapping.apply(0x07, 0x39), (0xFF, 0x03));
        assert_eq!(mapping.apply(0xFF, 0x03), (0x07, 0xE4));
        assert_eq!(mapping.apply(0xFF00, 0x03), (0x07, 0xE0));
    }

    #[test]
    fn unmapped_and_identity_keys_pass_unchanged() {
        let mapping = settings();
        assert_eq!(mapping.apply(0x07, 0x04), (0x07, 0x04));
        assert_eq!(mapping.apply(0x07, 0xE0), (0x07, 0xE0));
        assert_eq!(ModifierMapping::default().apply(0x07, 0xE2), (0x07, 0xE2));
    }

    #[test]
    fn values_outside_a_page_and_usage_are_dropped() {
        let mapping =
            ModifierMapping::from_pairs([(-1, packed(0x07, 0xE0)), (packed(0x1_0000, 1), 0)]);
        assert_eq!(mapping, ModifierMapping::default());
    }
}
