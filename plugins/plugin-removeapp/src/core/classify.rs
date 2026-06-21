#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MatchKind {
    Exact,
    Fuzzy,
}

pub fn normalize_entry(entry: &str) -> &str {
    entry
        .strip_suffix(".plist")
        .or_else(|| entry.strip_suffix(".savedState"))
        .unwrap_or(entry)
}

pub fn belongs_to(entry: &str, bid: &str) -> bool {
    let e = normalize_entry(entry);
    e == bid || e.starts_with(&format!("{bid}."))
}

pub fn owner_of<'a>(entry: &str, bids: &'a [String]) -> Option<&'a str> {
    bids.iter()
        .filter(|b| belongs_to(entry, b))
        .max_by_key(|b| b.len())
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn belongs_to_matches_exact_and_dot_boundary_only() {
        let cases = [
            ("com.acme.foo", "com.acme.foo", true),
            ("com.acme.foo.helper", "com.acme.foo", true),
            ("com.acme.foo.plist", "com.acme.foo", true),
            ("com.acme.foo.savedState", "com.acme.foo", true),
            ("com.acme.foobar", "com.acme.foo", false),
            ("com.acme.fo", "com.acme.foo", false),
        ];
        for (entry, bid, expected) in cases {
            assert_eq!(belongs_to(entry, bid), expected, "entry={entry} bid={bid}");
        }
    }

    #[test]
    fn owner_of_picks_longest_matching_bundle_id() {
        let bids = vec!["com.acme.foo".to_string(), "com.acme.foo.bar".to_string()];
        let cases = [
            ("com.acme.foo.helper", Some("com.acme.foo")),
            ("com.acme.foo.bar.cache", Some("com.acme.foo.bar")),
            ("com.acme.foobar", None),
        ];
        for (entry, expected) in cases {
            assert_eq!(owner_of(entry, &bids), expected, "entry={entry}");
        }
    }
}
