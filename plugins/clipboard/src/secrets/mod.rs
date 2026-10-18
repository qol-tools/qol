use std::time::Duration;

pub const SENSITIVE_FORMATS: [&str; 3] = [
    "x-kde-passwordManagerHint",
    "ExcludeClipboardContentFromMonitorProcessing",
    "org.nspasteboard.ConcealedType",
];

const MASK: &str = "••••••••";
const PASSWORD_CHARS: std::ops::RangeInclusive<usize> = 8..=128;
const LONG_PASSWORD_CHARS: usize = 12;

pub fn flagged_by_source<S: AsRef<str>>(formats: &[S]) -> bool {
    formats
        .iter()
        .any(|format| SENSITIVE_FORMATS.contains(&format.as_ref()))
}

// Browser password managers copy through navigator.clipboard and set no source marker, so a
// single token shaped like a generated password counts as a secret too: mixed case plus a digit
// or symbol, with both or at least 12 characters, and not a path, address, file name or
// identifier.
pub fn looks_like_password(text: &str) -> bool {
    let text = text.trim();
    let chars = text.chars().count();
    if !PASSWORD_CHARS.contains(&chars)
        || text.contains(char::is_whitespace)
        || text.contains("://")
        || text.starts_with(['/', '~', '.'])
        || looks_like_email(text)
        || has_file_extension(text)
        || looks_like_identifier(text)
    {
        return false;
    }
    let upper = text.chars().any(|c| c.is_uppercase());
    let lower = text.chars().any(|c| c.is_lowercase());
    let digit = text.chars().any(|c| c.is_ascii_digit());
    let symbol = text.chars().any(|c| !c.is_alphanumeric());
    upper && lower && (digit || symbol) && ((digit && symbol) || chars >= LONG_PASSWORD_CHARS)
}

fn looks_like_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._%+-".contains(c))
}

// Names such as `Release-2024`, `Foo_Bar_2` or `MyClass_v2`: words joined by separators, each
// letters then digits, cased as a name is. Generated passwords interleave digits, use other
// symbols or flip case every few letters, and passphrases such as
// `Correct-Horse-Battery-Staple` capitalise three or more words.
fn looks_like_identifier(text: &str) -> bool {
    let words: Vec<&str> = text.split(['-', '_', '.']).collect();
    let capitalised = words
        .iter()
        .filter(|word| word.starts_with(|c: char| c.is_uppercase()))
        .count();
    capitalised < 3
        && words.iter().all(|word| {
            let letters = word.trim_end_matches(|c: char| c.is_ascii_digit());
            letters.chars().all(|c| c.is_ascii_alphabetic()) && cased_as_a_name(letters)
        })
}

// All capitals, or at most one capital per three letters, as in `getElementById`.
fn cased_as_a_name(letters: &str) -> bool {
    let upper = letters.chars().filter(char::is_ascii_uppercase).count();
    upper == letters.len() || upper * 3 <= letters.len()
}

fn has_file_extension(text: &str) -> bool {
    text.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (1..=4).contains(&extension.len())
            && extension.chars().all(|c| c.is_ascii_lowercase())
    })
}

pub fn mask() -> &'static str {
    MASK
}

pub fn remaining(copied_at_ms: u64, now_ms: u64, ttl: Duration) -> Option<Duration> {
    let deadline = copied_at_ms.saturating_add(ttl.as_millis() as u64);
    (deadline > now_ms).then(|| Duration::from_millis(deadline - now_ms))
}

pub fn expired(copied_at_ms: u64, now_ms: u64, ttl: Duration) -> bool {
    remaining(copied_at_ms, now_ms, ttl).is_none()
}

pub fn countdown(remaining: Duration) -> String {
    let seconds = remaining.as_millis().div_ceil(1000);
    format!("clears in {seconds} s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_markers_flag_a_copy_as_secret() {
        let cases: &[(&[&str], bool)] = &[
            (
                &["TARGETS", "UTF8_STRING", "x-kde-passwordManagerHint"],
                true,
            ),
            (&["ExcludeClipboardContentFromMonitorProcessing"], true),
            (
                &["public.utf8-plain-text", "org.nspasteboard.ConcealedType"],
                true,
            ),
            (&["TARGETS", "UTF8_STRING", "text/plain"], false),
            (&[], false),
        ];
        for (formats, flagged) in cases {
            assert_eq!(flagged_by_source(formats), *flagged, "formats: {formats:?}");
        }
    }

    #[test]
    fn password_shaped_tokens_count_as_secrets() {
        let cases = [
            ("vT9#qL2!mZx8", true),
            ("Kq7mWp2xRt9vLs", true),
            ("Xy7@kq.Lm!", true),
            ("ghp_A1b2C3d4E5f6G7h8I9j0", true),
            ("  vT9#qL2!mZx8\n", true),
            ("Correct-Horse-Battery-Staple", true),
            ("kLmNoPqRsTuV-wXyZaBc", true),
            ("Windows11", false),
            ("Release-2024", false),
            ("Foo_Bar_2", false),
            ("MyClass_v2", false),
            ("HTTP_Server_2", false),
            ("HelloWorld2024", false),
            ("feature-Clipboard-Secrets", false),
            ("hunter2", false),
            ("getElementById", false),
            ("3f2a9c1e7b4d", false),
            ("550e8400-e29b-41d4-a716-446655440000", false),
            ("sha256:9f86d081884c7d659a2feaa0c55ad015", false),
            ("Kev.Smith@proton.me", false),
            ("MyReport2024.pdf", false),
            ("/home/Me/Notes-2024", false),
            ("https://Example.com/a1", false),
            ("Meet at Nine 9!", false),
            ("Ab1!", false),
        ];
        for (text, secret) in cases {
            assert_eq!(looks_like_password(text), secret, "text: {text:?}");
        }
    }

    #[test]
    fn secrets_expire_after_the_ttl() {
        let ttl = Duration::from_secs(30);
        let copied = 1_000_000;
        let cases = [
            (copied, Some(Duration::from_secs(30))),
            (copied + 8_000, Some(Duration::from_secs(22))),
            (copied + 29_999, Some(Duration::from_millis(1))),
            (copied + 30_000, None),
            (copied + 90_000, None),
        ];
        for (now, left) in cases {
            assert_eq!(remaining(copied, now, ttl), left, "now: {now}");
            assert_eq!(expired(copied, now, ttl), left.is_none(), "now: {now}");
        }
    }

    #[test]
    fn countdown_rounds_up_to_whole_seconds() {
        let cases = [
            (Duration::from_secs(22), "clears in 22 s"),
            (Duration::from_millis(21_001), "clears in 22 s"),
            (Duration::from_millis(1), "clears in 1 s"),
        ];
        for (left, label) in cases {
            assert_eq!(countdown(left), label);
        }
    }

    #[test]
    fn mask_has_a_fixed_width() {
        assert_eq!(mask().chars().count(), 8);
    }
}
