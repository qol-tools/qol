const KEY_MAX: usize = 64;
const SLUG_MAX: usize = 40;
const FALLBACK_KEY: &str = "session";
const LOOKUP_KEY_MAX: usize = 80;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Claimed {
    pub(super) key: String,
    pub(super) friendly: String,
}

pub(super) fn is_key(text: &str) -> bool {
    shaped(text) && text.len() <= KEY_MAX
}

pub(super) fn is_lookup_key(text: &str) -> bool {
    shaped(text) && text.len() <= LOOKUP_KEY_MAX
}

fn shaped(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(super) fn parse(description: &str) -> Claimed {
    let description = description.trim();
    if let Some((key, friendly)) = description.split_once(':') {
        let key = key.trim();
        if is_key(key) {
            return Claimed {
                key: key.to_owned(),
                friendly: friendly.trim().to_owned(),
            };
        }
    }
    Claimed {
        key: slug(description),
        friendly: description.to_owned(),
    }
}

pub(super) fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let cut = if slug.len() <= SLUG_MAX {
        slug
    } else if slug.as_bytes()[SLUG_MAX] == b'-' {
        &slug[..SLUG_MAX]
    } else {
        let head = &slug[..SLUG_MAX];
        head.rfind('-').map_or(head, |boundary| &head[..boundary])
    };
    let cut = cut.trim_matches('-');
    if cut.is_empty() {
        FALLBACK_KEY.to_owned()
    } else {
        cut.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_lowercase_ascii_runs_joined_by_one_dash_and_cut_at_a_boundary() {
        let long_word = "x".repeat(60);
        let cases: [(&str, &str); 14] = [
            ("Fix the Login Bug", "fix-the-login-bug"),
            ("  --Fix   the__bug!!  ", "fix-the-bug"),
            ("UPPER Case 42", "upper-case-42"),
            ("ünïcode ça va", "n-code-a-va"),
            ("日本語だけ", "session"),
            ("", "session"),
            ("   ", "session"),
            ("!!!", "session"),
            ("a", "a"),
            // Under the cut: kept whole.
            (
                &"abcd-".repeat(8)[..39],
                "abcd-abcd-abcd-abcd-abcd-abcd-abcd-abcd",
            ),
            // Over 40: cut back to the last '-' inside the first 40.
            (
                "orchestration revisions for the switchboard flow improvements",
                "orchestration-revisions-for-the",
            ),
            // The 41st character is a '-': the first 40 are already whole.
            (
                "aaaaaaaaaa bbbbbbbbbb cccccccccc ddddddd eeeeeeee",
                "aaaaaaaaaa-bbbbbbbbbb-cccccccccc-ddddddd",
            ),
            // No boundary to cut at: a hard cut at 40.
            (&long_word, &long_word[..40]),
            ("7 days", "7-days"),
        ];
        for (input, expected) in cases {
            assert_eq!(slug(input), expected, "{input:?}");
        }
    }

    #[test]
    fn a_slug_never_exceeds_the_cut_and_always_has_a_key_shape() {
        for input in [
            "x".repeat(500),
            "ab ".repeat(100),
            "-".repeat(90),
            "é".repeat(50),
            format!("{} end", "a".repeat(39)),
        ] {
            let slug = slug(&input);
            assert!(slug.len() <= SLUG_MAX, "{slug}");
            assert!(is_key(&slug), "{slug}");
        }
    }

    #[test]
    fn a_description_with_a_key_prefix_keeps_the_key_and_anything_else_is_slugged() {
        let long_key = "k".repeat(64);
        let too_long_key = "k".repeat(65);
        let too_long = format!("{too_long_key}: x");
        let hard_cut = "k".repeat(40);
        let edge = format!("{long_key}: edge");
        let cases: Vec<(String, &str, &str)> = vec![
            (
                "switchboard-flow-improvements: orchestration revisions".into(),
                "switchboard-flow-improvements",
                "orchestration revisions",
            ),
            ("a: b".into(), "a", "b"),
            (
                "  spaced-key :  padded friendly  ".into(),
                "spaced-key",
                "padded friendly",
            ),
            ("key:".into(), "key", ""),
            ("key:   ".into(), "key", ""),
            ("9lives: numbers lead".into(), "9lives", "numbers lead"),
            ("a: b: c".into(), "a", "b: c"),
            (edge, long_key.as_str(), "edge"),
            // Not a key: uppercase, leading dash, underscore, unicode, too long, empty, spaces.
            ("Key: Friendly".into(), "key-friendly", "Key: Friendly"),
            ("-key: x".into(), "key-x", "-key: x"),
            ("my_key: x".into(), "my-key-x", "my_key: x"),
            ("clé: x".into(), "cl-x", "clé: x"),
            (too_long.clone(), hard_cut.as_str(), too_long.as_str()),
            (": only friendly".into(), "only-friendly", ": only friendly"),
            ("two words: x".into(), "two-words-x", "two words: x"),
            (
                "No colon at all".into(),
                "no-colon-at-all",
                "No colon at all",
            ),
            ("   ".into(), "session", ""),
            (String::new(), "session", ""),
        ];
        for (description, key, friendly) in cases {
            assert_eq!(
                parse(&description),
                Claimed {
                    key: key.to_owned(),
                    friendly: friendly.to_owned(),
                },
                "{description:?}"
            );
        }
    }

    #[test]
    fn lookup_keys_allow_the_collision_suffix_but_never_a_path() {
        for key in ["a", "9", "a-b", "a-2", &"k".repeat(80)] {
            assert!(is_lookup_key(key), "{key}");
        }
        for key in [
            "",
            "-a",
            "A",
            "a/b",
            "..",
            "a.b",
            "a_b",
            "é",
            &"k".repeat(81),
        ] {
            assert!(!is_lookup_key(key), "{key}");
        }
    }
}
