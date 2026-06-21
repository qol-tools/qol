use crate::core::Disposal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MatchKind {
    Exact,
    Fuzzy,
}

pub fn effective_disposal(
    match_kind: MatchKind,
    requested: Disposal,
    bundle_trash_override: bool,
) -> Disposal {
    match (match_kind, bundle_trash_override) {
        (MatchKind::Fuzzy, _) => Disposal::Trash,
        (MatchKind::Exact, true) => Disposal::Trash,
        (MatchKind::Exact, false) => requested,
    }
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

    #[test]
    fn effective_disposal_keeps_fuzzy_in_trash_always() {
        let cases = [
            (MatchKind::Exact, Disposal::Delete, false, Disposal::Delete),
            (MatchKind::Exact, Disposal::Trash, false, Disposal::Trash),
            (MatchKind::Exact, Disposal::Delete, true, Disposal::Trash),
            (MatchKind::Fuzzy, Disposal::Delete, false, Disposal::Trash),
            (MatchKind::Fuzzy, Disposal::Delete, true, Disposal::Trash),
        ];
        for (mk, req, override_trash, expected) in cases {
            assert_eq!(
                effective_disposal(mk, req, override_trash),
                expected,
                "mk={mk:?} req={req:?} override={override_trash}"
            );
        }
    }
}
