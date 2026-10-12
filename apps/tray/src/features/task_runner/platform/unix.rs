pub(super) fn quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if is_shell_safe(value) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn is_shell_safe(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_@%+/=:,.".contains(ch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn quote_escapes_special_values() {
        let cases = [
            ("", "''"),
            ("hello world", "'hello world'"),
            (
                "https://example.com?q=1&x=2",
                "'https://example.com?q=1&x=2'",
            ),
            ("safe_value-123", "safe_value-123"),
            ("a'b", "'a'\"'\"'b'"),
        ];

        for (value, expected) in cases {
            assert_eq!(quote(value), expected, "value: {value:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_quote_keeps_safe_ascii(value in "[A-Za-z0-9_@%+/=:,.-]{1,32}") {
            prop_assert_eq!(quote(&value), value);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn prop_quote_round_trips_through_bash(
            value in "[\x20-\x7e]{0,40}".prop_filter("no NUL", |s| !s.contains('\0')),
        ) {
            let escaped = quote(&value);
            let script = format!("printf '%s' {escaped}");
            let output = std::process::Command::new("bash")
                .arg("-c")
                .arg(&script)
                .output()
                .expect("bash must run");
            prop_assert!(
                output.status.success(),
                "bash failed for value={:?} escaped={:?} stderr={:?}",
                value,
                escaped,
                String::from_utf8_lossy(&output.stderr),
            );
            let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
            prop_assert_eq!(
                &stdout, &value,
                "shell-escaped payload must round-trip; escaped={:?}",
                escaped,
            );
        }

        #[test]
        fn prop_quote_in_command_position_does_not_split_words(
            value in "[\x20-\x7e]{1,40}".prop_filter("no NUL", |s| !s.contains('\0')),
        ) {
            let escaped = quote(&value);
            let script = format!("set -- {escaped}; printf '%d' \"$#\"");
            let output = std::process::Command::new("bash")
                .arg("-c")
                .arg(&script)
                .output()
                .expect("bash must run");
            let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
            prop_assert_eq!(
                &stdout, "1",
                "escaped payload must be a single arg; value={:?} escaped={:?} got={:?}",
                value,
                escaped,
                stdout,
            );
        }
    }
}
