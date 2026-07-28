pub(crate) fn redact_secrets(input: &str) -> String {
    const MARKERS: &[&str] = &[
        "bearer ",
        "\"access_token\":\"",
        "\"access_token\": \"",
        "\"api_key\":\"",
        "\"api_key\": \"",
        "\"client_secret\":\"",
        "\"client_secret\": \"",
        "\"password\":\"",
        "\"password\": \"",
        "\"refresh_token\":\"",
        "\"refresh_token\": \"",
        "\"secret\":\"",
        "\"secret\": \"",
        "\"token\":\"",
        "\"token\": \"",
        "access_token=",
        "api-key=",
        "api_key=",
        "apikey=",
        "authorization:",
        "client_secret=",
        "github_pat_",
        "password=",
        "refresh_token=",
        "secret=",
        "token=",
    ];

    let mut output = input.to_string();
    for marker in MARKERS {
        let mut search_from = 0;
        loop {
            let lower = output.to_ascii_lowercase();
            let Some(relative) = lower[search_from..].find(marker) else {
                break;
            };
            let value_start = search_from + relative + marker.len();
            let value_end = output[value_start..]
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '&' | ',' | ';' | '"' | '\'' | ')')
                })
                .map_or(output.len(), |length| value_start + length);
            if value_end == value_start {
                search_from = value_start;
                continue;
            }
            output.replace_range(value_start..value_end, "[REDACTED]");
            search_from = value_start + "[REDACTED]".len();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_headers_and_key_value_secrets() {
        let cases = [
            (
                "GET https://example.test/?access_token=abc123&x=1",
                "GET https://example.test/?access_token=[REDACTED]&x=1",
            ),
            (
                "Authorization: Bearer abc123",
                "Authorization: Bearer [REDACTED]",
            ),
            (
                "Authorization:Bearer abc123",
                "Authorization:[REDACTED] [REDACTED]",
            ),
            ("api_key=secret-value next", "api_key=[REDACTED] next"),
            (
                r#"{"access_token":"abc123","ok":true}"#,
                r#"{"access_token":"[REDACTED]","ok":true}"#,
            ),
            (
                r#"{"access_token": "abc123", "ok": true}"#,
                r#"{"access_token": "[REDACTED]", "ok": true}"#,
            ),
            ("github_pat_abc123", "github_pat_[REDACTED]"),
            ("ordinary error", "ordinary error"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact_secrets(input), expected, "input={input}");
        }
    }
}
