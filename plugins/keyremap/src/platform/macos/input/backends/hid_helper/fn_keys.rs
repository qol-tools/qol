use crate::platform::macos::hid_helper::protocol::{
    PAGE_APPLE_VENDOR_KEYBOARD, PAGE_CONSUMER, PAGE_KEYBOARD,
};

const TOP_ROW: [(u16, u16, u16); 11] = [
    (0x3A, PAGE_CONSUMER, 0x70),
    (0x3B, PAGE_CONSUMER, 0x6F),
    (0x3C, PAGE_APPLE_VENDOR_KEYBOARD, 0x10),
    (0x3D, PAGE_APPLE_VENDOR_KEYBOARD, 0x01),
    (0x3E, PAGE_CONSUMER, 0xCF),
    (0x40, PAGE_CONSUMER, 0xB4),
    (0x41, PAGE_CONSUMER, 0xCD),
    (0x42, PAGE_CONSUMER, 0xB3),
    (0x43, PAGE_CONSUMER, 0xE2),
    (0x44, PAGE_CONSUMER, 0xEA),
    (0x45, PAGE_CONSUMER, 0xE9),
];

const FN_NAVIGATION: [(u16, u16); 6] = [
    (0x50, 0x4A),
    (0x4F, 0x4D),
    (0x52, 0x4B),
    (0x51, 0x4E),
    (0x2A, 0x4C),
    (0x28, 0x58),
];

pub(crate) fn translate(
    usage: u16,
    fn_down: bool,
    fn_keys_are_standard: bool,
) -> Option<(u16, u16)> {
    if let Some(&(_, page, media)) = TOP_ROW.iter().find(|(key, ..)| *key == usage) {
        return (fn_down == fn_keys_are_standard).then_some((page, media));
    }
    if !fn_down {
        return None;
    }
    FN_NAVIGATION
        .iter()
        .find(|(key, _)| *key == usage)
        .map(|&(_, target)| (PAGE_KEYBOARD, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_row_is_media_unless_fn_or_the_setting_says_otherwise() {
        assert_eq!(translate(0x3A, false, false), Some((PAGE_CONSUMER, 0x70)));
        assert_eq!(translate(0x3A, true, false), None);
        assert_eq!(translate(0x3A, false, true), None);
        assert_eq!(translate(0x3A, true, true), Some((PAGE_CONSUMER, 0x70)));
        assert_eq!(
            translate(0x3C, false, false),
            Some((PAGE_APPLE_VENDOR_KEYBOARD, 0x10))
        );
        assert_eq!(
            translate(0x3D, false, false),
            Some((PAGE_APPLE_VENDOR_KEYBOARD, 0x01))
        );
        assert_eq!(translate(0x45, false, false), Some((PAGE_CONSUMER, 0xE9)));
    }

    #[test]
    fn f6_stays_f6() {
        assert_eq!(translate(0x3F, false, false), None);
        assert_eq!(translate(0x3F, true, false), None);
    }

    #[test]
    fn fn_with_arrows_backspace_and_return_navigates() {
        assert_eq!(translate(0x50, true, false), Some((PAGE_KEYBOARD, 0x4A)));
        assert_eq!(translate(0x4F, true, false), Some((PAGE_KEYBOARD, 0x4D)));
        assert_eq!(translate(0x52, true, false), Some((PAGE_KEYBOARD, 0x4B)));
        assert_eq!(translate(0x51, true, false), Some((PAGE_KEYBOARD, 0x4E)));
        assert_eq!(translate(0x2A, true, false), Some((PAGE_KEYBOARD, 0x4C)));
        assert_eq!(translate(0x28, true, false), Some((PAGE_KEYBOARD, 0x58)));
        assert_eq!(translate(0x50, false, false), None);
    }
}
