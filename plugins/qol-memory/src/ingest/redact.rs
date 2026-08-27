use std::sync::OnceLock;

use regex::Regex;

const LONG_PATTERN: &str = r"(?-u:\b)[A-Za-z0-9_\-]{32,}(?-u:\b)";
const SECRET_PATTERN: &str =
    r"(?i)(?:Bearer|Token|api[_-]?key|password|passwd|secret|private[_-]?key)\s*[:=]\s*\S+";
const KEY_PATTERN: &str = r"sk-[A-Za-z0-9]{20,}";
const PEM_PATTERN: &str = r"-----BEGIN[\s\S]*?END [A-Z ]*-----";
const EMAIL_PATTERN: &str = r"[A-Za-z0-9_.+-]+@[A-Za-z0-9_.-]+\.[A-Za-z0-9_]{2,}";
const ENV_PATTERN: &str = r"\.env[\s\S]*";

struct Rules {
    long: Regex,
    secrets: Regex,
    keys: Regex,
    pem: Regex,
    emails: Regex,
    env: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        long: Regex::new(LONG_PATTERN).expect("long word regex"),
        secrets: Regex::new(SECRET_PATTERN).expect("secret assignment regex"),
        keys: Regex::new(KEY_PATTERN).expect("api key regex"),
        pem: Regex::new(PEM_PATTERN).expect("pem regex"),
        emails: Regex::new(EMAIL_PATTERN).expect("email regex"),
        env: Regex::new(ENV_PATTERN).expect("env file regex"),
    })
}

pub fn redact(text: &str) -> String {
    let rules = rules();
    let step = rules.long.replace_all(text, "[REDACTED]");
    let step = rules.secrets.replace_all(&step, "$$1=[REDACTED]");
    let step = rules.keys.replace_all(&step, "[REDACTED-KEY]");
    let step = rules.pem.replace_all(&step, "[REDACTED-PEM]");
    let step = rules.emails.replace_all(&step, "[EMAIL]");
    rules.env.replace_all(&step, ".env [REDACTED]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_matches_js_vectors() {
        let cases = [
            (format!("{} tail", "x".repeat(32)), "[REDACTED] tail"),
            ("password=hunter2".to_string(), "$1=[REDACTED]"),
            (
                "key sk-abcdefghijklmnopqrst end".to_string(),
                "key [REDACTED-KEY] end",
            ),
            (
                "-----BEGIN KEY-----\nabc\n-----END KEY-----".to_string(),
                "[REDACTED-PEM]",
            ),
            (
                "mail bob.smith+tag@example.com today".to_string(),
                "mail [EMAIL] today",
            ),
            (
                "cp .env.example .env && run".to_string(),
                "cp .env [REDACTED]",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(redact(&input), expected);
        }
    }
}
