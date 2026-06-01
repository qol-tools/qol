pub struct ExportedCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub route: &'static str,
}

/// Brand prefix prepended to every exported command's launcher label, so
/// qol-tray commands read as a group in Spotlight / the app launcher. Labels
/// in `EXPORTED` stay bare; the prefix is applied once at entry-build time.
pub const QOL_COMMAND_PREFIX: &str = "QoL › ";

pub const EXPORTED: &[ExportedCommand] = &[
    ExportedCommand {
        id: "shortcuts-add",
        label: "Add Shortcut",
        route: "shortcuts/add",
    },
    ExportedCommand {
        id: "shortcuts-open",
        label: "Shortcuts",
        route: "shortcuts",
    },
];

/// The launcher display label for a command: brand prefix + bare label.
pub fn command_label(command: &ExportedCommand) -> String {
    format!("{QOL_COMMAND_PREFIX}{}", command.label)
}

pub fn deeplink_url(route: &str, port: u16) -> String {
    let r = route.trim_start_matches('#').trim_start_matches('/');
    format!("http://localhost:{port}/#{r}")
}

/// Internal argv marker the macOS `openURLs` delegate uses to re-exec this
/// binary as a pure courier: it forwards the `qol://` route (next argv) to the
/// already-running daemon and exits, never starting a second daemon. Linux
/// delivers the URL directly as `%u` argv and has no need for this.
pub const URL_COURIER_FLAG: &str = "__url-courier";

/// Extract the in-app route from a `qol://<route>` URL. Returns `None` for any
/// other scheme or a blank route. The scheme match is case-insensitive per
/// RFC 3986 (so `QOL://` works), and a route that is only slashes/whitespace is
/// rejected. The route keeps its querystring verbatim; percent-decoding happens
/// downstream in the same param parser the hash router uses.
pub fn parse_qol_url(input: &str) -> Option<String> {
    let scheme_end = input.find("://")?;
    if !input[..scheme_end].eq_ignore_ascii_case("qol") {
        return None;
    }
    let route = input[scheme_end + 3..]
        .trim()
        .trim_start_matches('/')
        .trim();
    if route.is_empty() {
        None
    } else {
        Some(route.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deeplink_url_builds_localhost_hash_route() {
        assert_eq!(
            deeplink_url("shortcuts/add", 42700),
            "http://localhost:42700/#shortcuts/add"
        );
    }

    #[test]
    fn deeplink_url_strips_leading_hash_and_slash() {
        assert_eq!(
            deeplink_url("#shortcuts", 42700),
            "http://localhost:42700/#shortcuts"
        );
        assert_eq!(
            deeplink_url("/shortcuts", 42700),
            "http://localhost:42700/#shortcuts"
        );
    }

    #[test]
    fn exported_catalog_is_nonempty_with_unique_wellformed_entries() {
        assert!(!EXPORTED.is_empty());
        let mut ids = std::collections::HashSet::new();
        for c in EXPORTED {
            assert!(ids.insert(c.id), "duplicate command id: {}", c.id);
            assert!(!c.label.is_empty());
            assert!(!c.route.is_empty());
            assert!(
                !c.route.starts_with('#') && !c.route.starts_with('/'),
                "route must be bare: {}",
                c.route
            );
            assert!(
                !c.label.contains("QoL"),
                "label must be bare (prefix is applied by command_label): {}",
                c.label
            );
        }
    }

    #[test]
    fn add_shortcut_command_present() {
        assert!(EXPORTED
            .iter()
            .any(|c| c.id == "shortcuts-add" && c.route == "shortcuts/add"));
    }

    #[test]
    fn command_label_applies_brand_prefix() {
        let add = EXPORTED.iter().find(|c| c.id == "shortcuts-add").unwrap();
        assert_eq!(command_label(add), "QoL › Add Shortcut");
    }

    #[test]
    fn parse_qol_url_extracts_route_with_querystring() {
        assert_eq!(
            parse_qol_url("qol://shortcuts/add?type=url&url=https://x&name=X"),
            Some("shortcuts/add?type=url&url=https://x&name=X".to_string())
        );
        assert_eq!(
            parse_qol_url("qol://shortcuts"),
            Some("shortcuts".to_string())
        );
    }

    #[test]
    fn parse_qol_url_strips_leading_slashes() {
        assert_eq!(
            parse_qol_url("qol:///shortcuts"),
            Some("shortcuts".to_string())
        );
    }

    #[test]
    fn parse_qol_url_rejects_other_schemes_and_empty() {
        assert_eq!(parse_qol_url("https://example.com"), None);
        assert_eq!(parse_qol_url("qol://"), None);
        assert_eq!(parse_qol_url("shortcuts"), None);
        assert_eq!(parse_qol_url("qol:///"), None);
    }

    #[test]
    fn parse_qol_url_scheme_is_case_insensitive() {
        assert_eq!(
            parse_qol_url("QOL://shortcuts"),
            Some("shortcuts".to_string())
        );
        assert_eq!(
            parse_qol_url("Qol://shortcuts/add"),
            Some("shortcuts/add".to_string())
        );
        assert_eq!(parse_qol_url("qOl://x?y=z"), Some("x?y=z".to_string()));
    }

    #[test]
    fn parse_qol_url_rejects_whitespace_only_route() {
        assert_eq!(parse_qol_url("qol://   "), None);
        assert_eq!(parse_qol_url("qol://  /  "), None);
    }

    #[test]
    fn parse_qol_url_first_scheme_separator_wins_over_querystring_url() {
        // The `://` inside an embedded http URL must not be mistaken for the scheme.
        assert_eq!(
            parse_qol_url("qol://shortcuts/add?url=https://example.com"),
            Some("shortcuts/add?url=https://example.com".to_string())
        );
    }
}
