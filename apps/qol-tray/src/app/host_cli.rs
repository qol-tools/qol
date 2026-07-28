#[derive(Debug, PartialEq, Eq)]
pub(super) enum Invocation {
    Daemon,
    Help,
    Version,
    WriteMode(String),
    Headless(Vec<String>),
    Exec { target: String, action: String },
    Open(String),
    UrlCourier(String),
    Url(String),
    Invalid,
}

pub(super) fn from_env() -> Invocation {
    classify(std::env::args().skip(1).collect())
}

fn classify(args: Vec<String>) -> Invocation {
    if args.is_empty() {
        return Invocation::Daemon;
    }
    if matches!(args.as_slice(), [argument] if matches!(argument.as_str(), "help" | "--help" | "-h"))
    {
        return Invocation::Help;
    }

    let tokens = args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .map(|arg| match arg.as_str() {
            "--help" | "-h" => "help",
            token => token,
        })
        .collect::<Vec<_>>();
    if matches!(tokens.first(), Some(&"doctor")) {
        return Invocation::Headless(args);
    }
    let contains_json = args.iter().any(|arg| arg == "--json");
    match args.as_slice() {
        [command, target, action] if command == "exec" => {
            if contains_json {
                return Invocation::Invalid;
            }
            return Invocation::Exec {
                target: target.clone(),
                action: action.clone(),
            };
        }
        [command, route] if command == "open" && route != "help" => {
            if contains_json {
                return Invocation::Invalid;
            }
            return Invocation::Open(route.clone());
        }
        _ => {}
    }
    if tokens.contains(&"help") {
        return Invocation::Headless(args);
    }
    if contains_json || tokens.contains(&"doctor") {
        return Invocation::Invalid;
    }

    match args.as_slice() {
        [flag] if matches!(flag.as_str(), "--version" | "-V") => Invocation::Version,
        [mode] if mode.starts_with("--write-mode=") => {
            Invocation::WriteMode(mode["--write-mode=".len()..].to_string())
        }
        [courier, url] if courier == qol_tray::commands::URL_COURIER_FLAG => {
            qol_tray::commands::parse_qol_url(url)
                .map(Invocation::UrlCourier)
                .unwrap_or(Invocation::Invalid)
        }
        [url] => qol_tray::commands::parse_qol_url(url)
            .map(Invocation::Url)
            .unwrap_or(Invocation::Invalid),
        _ => Invocation::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, Invocation};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn classifies_every_supported_headless_form() {
        for values in [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["--help", "doctor"],
            vec!["doctor", "--help"],
            vec!["help", "open"],
            vec!["open", "help"],
            vec!["help", "exec"],
            vec!["exec", "help"],
            vec!["-h", "doctor"],
            vec!["doctor", "-h"],
            vec!["help", "status"],
            vec!["status", "help"],
            vec!["--help", "status"],
            vec!["status", "--help"],
            vec!["help", "status", "doctor"],
        ] {
            assert_eq!(
                classify(args(&values)),
                Invocation::Headless(args(&values)),
                "headless route was not recognized: {values:?}",
            );
        }
    }

    #[test]
    fn classifies_daemon_flags_and_operational_routes_exactly() {
        let cases = [
            (vec![], Invocation::Daemon),
            (vec!["help"], Invocation::Help),
            (vec!["--help"], Invocation::Help),
            (vec!["-h"], Invocation::Help),
            (vec!["--version"], Invocation::Version),
            (vec!["-V"], Invocation::Version),
            (
                vec!["--write-mode=dev"],
                Invocation::WriteMode("dev".to_string()),
            ),
            (
                vec!["exec", "plugin-test", "toggle"],
                Invocation::Exec {
                    target: "plugin-test".to_string(),
                    action: "toggle".to_string(),
                },
            ),
            (
                vec!["exec", "shortcut", "shortcut-id"],
                Invocation::Exec {
                    target: "shortcut".to_string(),
                    action: "shortcut-id".to_string(),
                },
            ),
            (
                vec!["exec", "plugin-test", "doctor"],
                Invocation::Exec {
                    target: "plugin-test".to_string(),
                    action: "doctor".to_string(),
                },
            ),
            (
                vec!["exec", "plugin-test", "help"],
                Invocation::Exec {
                    target: "plugin-test".to_string(),
                    action: "help".to_string(),
                },
            ),
            (
                vec!["open", "settings"],
                Invocation::Open("settings".to_string()),
            ),
            (
                vec!["open", "doctor"],
                Invocation::Open("doctor".to_string()),
            ),
            (
                vec![qol_tray::commands::URL_COURIER_FLAG, "qol://shortcuts/add"],
                Invocation::UrlCourier("shortcuts/add".to_string()),
            ),
            (
                vec!["qol://shortcuts/add"],
                Invocation::Url("shortcuts/add".to_string()),
            ),
        ];

        for (values, expected) in cases {
            assert_eq!(classify(args(&values)), expected, "{values:?}");
        }
    }

    #[test]
    fn rejects_every_malformed_route_before_operational_dispatch() {
        for values in [
            vec!["--bogus", "doctor"],
            vec!["--write-mode=dev", "doctor"],
            vec!["status", "doctor"],
            vec!["--doctor"],
            vec!["--json"],
            vec!["unknown"],
            vec!["--write-mode=dev", "extra"],
            vec!["qol://shortcuts/add", "doctor"],
            vec![qol_tray::commands::URL_COURIER_FLAG],
            vec![qol_tray::commands::URL_COURIER_FLAG, "https://example.com"],
            vec!["--json", "exec", "plugin-test", "toggle"],
            vec!["exec", "plugin-test", "toggle", "--json"],
            vec!["--json", "open", "settings"],
            vec!["open", "settings", "--json"],
            vec!["open", "settings", "extra"],
            vec!["exec", "plugin-test", "toggle", "extra"],
        ] {
            assert_eq!(
                classify(args(&values)),
                Invocation::Invalid,
                "malformed host route was accepted: {values:?}",
            );
        }
    }
}
