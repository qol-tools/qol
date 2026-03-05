#![cfg(feature = "dev")]

use std::collections::HashSet;

use super::helpers::validate_plugin_id;

const MAX_MONITORED_PLUGIN_IDS: usize = 128;

pub(super) fn sanitize_monitored_plugin_ids(
    plugin_ids: Vec<String>,
) -> Result<Vec<String>, &'static str> {
    if plugin_ids.len() > MAX_MONITORED_PLUGIN_IDS {
        return Err("Too many plugin IDs");
    }

    let mut unique = HashSet::new();
    let mut sanitized = Vec::new();
    for raw_plugin_id in plugin_ids {
        let plugin_id = raw_plugin_id.trim();
        if plugin_id.is_empty() {
            continue;
        }
        if validate_plugin_id(plugin_id).is_err() {
            return Err("Invalid plugin ID in monitoring list");
        }
        let normalized = plugin_id.to_string();
        if !unique.insert(normalized.clone()) {
            continue;
        }
        sanitized.push(normalized);
    }
    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::strategy::Strategy;
    use std::collections::HashSet;

    #[test]
    fn sanitize_monitored_plugin_ids_rejects_invalid_values() {
        let cases = vec![
            "../oops",
            "plugin/child",
            "plugin child",
            "-plugin",
            "plugin.with.dot",
            "plugin\0null",
        ];
        for case in cases {
            let result = sanitize_monitored_plugin_ids(vec![case.to_string()]);
            assert_eq!(result, Err("Invalid plugin ID in monitoring list"));
        }
    }

    #[test]
    fn sanitize_monitored_plugin_ids_dedupes_skips_empty_and_trims() {
        let result = sanitize_monitored_plugin_ids(vec![
            "plugin-one".to_string(),
            " plugin-one ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "plugin-two".to_string(),
        ]);
        assert_eq!(
            result,
            Ok(vec!["plugin-one".to_string(), "plugin-two".to_string()])
        );
    }

    #[test]
    fn sanitize_monitored_plugin_ids_rejects_input_over_limit() {
        let over_limit = vec!["plugin".to_string(); MAX_MONITORED_PLUGIN_IDS + 1];
        let result = sanitize_monitored_plugin_ids(over_limit);
        assert_eq!(result, Err("Too many plugin IDs"));
    }

    #[test]
    fn sanitize_monitored_plugin_ids_accepts_input_at_limit() {
        let at_limit = (0..MAX_MONITORED_PLUGIN_IDS)
            .map(|index| format!("plugin-{index}"))
            .collect::<Vec<_>>();
        let result = sanitize_monitored_plugin_ids(at_limit.clone());
        assert_eq!(result, Ok(at_limit));
    }

    fn valid_plugin_id_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex("[A-Za-z0-9_][A-Za-z0-9_-]{0,15}").unwrap()
    }

    fn padded_valid_plugin_id_strategy() -> impl Strategy<Value = String> {
        (0usize..=2, valid_plugin_id_strategy(), 0usize..=2).prop_map(
            |(left_padding, plugin_id, right_padding)| {
                format!(
                    "{}{}{}",
                    " ".repeat(left_padding),
                    plugin_id,
                    " ".repeat(right_padding)
                )
            },
        )
    }

    fn invalid_plugin_id_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("../oops".to_string()),
            Just("plugin/child".to_string()),
            Just("plugin child".to_string()),
            Just("-plugin".to_string()),
            Just("plugin.with.dot".to_string()),
            Just("plugin\0null".to_string()),
            Just("a".repeat(65)),
        ]
    }

    fn expected_sanitized(input: &[String]) -> Vec<String> {
        let mut unique = HashSet::new();
        let mut expected = Vec::new();
        for value in input {
            let plugin_id = value.trim();
            if plugin_id.is_empty() {
                continue;
            }
            let normalized = plugin_id.to_string();
            if !unique.insert(normalized.clone()) {
                continue;
            }
            expected.push(normalized);
        }
        expected
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_sanitize_monitored_plugin_ids_keeps_trimmed_unique_valid_values(
            input in prop::collection::vec(
                padded_valid_plugin_id_strategy(),
                0..=MAX_MONITORED_PLUGIN_IDS
            )
        ) {
            let expected = expected_sanitized(&input);
            let actual = sanitize_monitored_plugin_ids(input);
            prop_assert_eq!(actual, Ok(expected));
        }

        #[test]
        fn prop_sanitize_monitored_plugin_ids_rejects_invalid_values(
            mut prefix in prop::collection::vec(
                padded_valid_plugin_id_strategy(),
                0..32
            ),
            invalid in invalid_plugin_id_strategy(),
            suffix in prop::collection::vec(
                padded_valid_plugin_id_strategy(),
                0..32
            )
        ) {
            let mut input = Vec::new();
            input.append(&mut prefix);
            input.push(invalid);
            input.extend(suffix);
            prop_assume!(input.len() <= MAX_MONITORED_PLUGIN_IDS);
            let result = sanitize_monitored_plugin_ids(input);
            prop_assert_eq!(result, Err("Invalid plugin ID in monitoring list"));
        }

        #[test]
        fn prop_sanitize_monitored_plugin_ids_rejects_oversized_payload(
            input in prop::collection::vec(
                valid_plugin_id_strategy(),
                (MAX_MONITORED_PLUGIN_IDS + 1)..(MAX_MONITORED_PLUGIN_IDS + 64)
            )
        ) {
            let result = sanitize_monitored_plugin_ids(input);
            prop_assert_eq!(result, Err("Too many plugin IDs"));
        }
    }
}
