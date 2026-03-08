use launcher::ui::key_to_input_char;
use proptest::prelude::*;

mod common;
use common::config;

fn dense_config() -> ProptestConfig {
    let mut cfg = config();
    cfg.cases = 2000;
    cfg
}

proptest! {
    #![proptest_config(dense_config())]

    #[test]
    fn prop_multi_char_keys_never_insert_text(
        key in "[a-z]{2,12}",
        shift in any::<bool>()
    ) {
        prop_assert_eq!(
            key_to_input_char(&key, shift),
            None,
            "multi-char key '{}' must not produce text input",
            key
        );
    }

    #[test]
    fn prop_allowed_single_chars_round_trip_without_shift(
        key in "[a-zA-Z0-9_.-]"
    ) {
        let ch = key.chars().next().unwrap();
        prop_assert_eq!(key_to_input_char(&key, false), Some(ch));
    }

    #[test]
    fn prop_allowed_single_chars_shift_behavior_for_letters(
        key in "[a-zA-Z]"
    ) {
        let ch = key.chars().next().unwrap();
        prop_assert_eq!(key_to_input_char(&key, true), Some(ch.to_ascii_uppercase()));
    }

    #[test]
    fn prop_disallowed_single_ascii_chars_are_rejected(
        ch in any::<char>().prop_filter(
                "single-char input must be ASCII punctuation/symbol that launcher should reject",
                |c| c.is_ascii()
                    && !c.is_ascii_alphanumeric()
                    && *c != '-'
                    && *c != '_'
                    && *c != '.'
                    && !c.is_whitespace()
                    && !c.is_control()
        ),
        shift in any::<bool>()
    ) {
        let key = ch.to_string();
        prop_assert_eq!(key_to_input_char(&key, shift), None);
    }
}
