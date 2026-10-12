const CMD_METACHARACTERS: &[char] = &['^', '&', '|', '<', '>', '(', ')', '"', '!'];

pub(super) fn quote(value: &str) -> Result<String, String> {
    if value.contains(['\r', '\n']) {
        return Err("cmd cannot pass a line break".to_string());
    }
    Ok(escape_for_cmd(&quote_for_argv(value)))
}

fn quote_for_argv(value: &str) -> String {
    if !value.is_empty() && !value.contains([' ', '\t', '"']) {
        return value.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        let escapes = if character == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        quoted.push_str(&"\\".repeat(escapes));
        quoted.push(character);
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn escape_for_cmd(argument: &str) -> String {
    let mut escaped = String::with_capacity(argument.len() * 2);
    let mut characters = argument.chars().peekable();
    while let Some(character) = characters.next() {
        if CMD_METACHARACTERS.contains(&character) {
            escaped.push('^');
        }
        escaped.push(character);
        if character == '%'
            && characters
                .peek()
                .is_some_and(|next| !CMD_METACHARACTERS.contains(next))
        {
            escaped.push('^');
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_special_values() {
        let cases = [
            ("", "^\"^\""),
            ("safe_value-123", "safe_value-123"),
            ("hello world", "^\"hello world^\""),
            (
                "https://example.com?q=1&x=2",
                "https://example.com?q=1^&x=2",
            ),
            ("x & echo INJECTED &", "^\"x ^& echo INJECTED ^&^\""),
            ("say \"hi\"", "^\"say \\^\"hi\\^\"^\""),
            (r"C:\my dir\", r#"^"C:\my dir\\^""#),
            (r"C:\dir\", r"C:\dir\"),
            ("100%", "100%"),
            ("%PATH%", "%^PATH%"),
            ("%PATH:~0,1%", "%^PATH:~0,1%"),
            ("%%", "%^%"),
            ("%&", "%^&"),
            ("a^b", "a^^b"),
            ("(a|b)<c>!", "^(a^|b^)^<c^>^!"),
        ];

        for (value, expected) in cases {
            assert_eq!(quote(value).as_deref(), Ok(expected), "value: {value:?}");
        }
    }

    #[test]
    fn quote_rejects_line_breaks() {
        for value in ["a\nb", "a\r\nb", "trailing\r"] {
            assert!(quote(value).is_err(), "value: {value:?}");
        }
    }

    #[cfg(target_os = "windows")]
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

        #[test]
        fn prop_quote_reaches_the_program_as_one_argv_quoted_word(
            value in "[\x20-\x7e]{0,40}",
        ) {
            let escaped = quote(&value).expect("printable ASCII is quotable");
            let output = super::super::shell_command(&format!("echo x{escaped}"))
                .expect("cmd command")
                .output()
                .expect("cmd must run");
            let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
            proptest::prop_assert_eq!(
                stdout,
                format!("x{}\r\n", quote_for_argv(&value)),
                "escaped={:?}",
                escaped,
            );
        }
    }
}
