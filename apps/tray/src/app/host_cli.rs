#![allow(clippy::print_stdout, clippy::print_stderr)]

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
    ResidentPolicy(Vec<String>),
    ResidentPolicyHidden(Vec<String>),
    Invalid,
}

pub(super) fn from_env() -> Invocation {
    classify(std::env::args().skip(1).collect())
}

pub(super) fn dispatch(invocation: Invocation) -> Option<i32> {
    match invocation {
        Invocation::Daemon => None,
        Invocation::Help => {
            print_usage();
            Some(0)
        }
        Invocation::Version => {
            println!("qol-tray {}", super::qol_tray_version());
            Some(0)
        }
        Invocation::WriteMode(value) => {
            let exit = write_mode_flag(&value);
            if exit != 0 {
                Some(exit)
            } else {
                None
            }
        }
        Invocation::Headless(args) => Some(qol_tray::doctor::run_host_cli(args)),
        Invocation::ResidentPolicy(args) => {
            Some(qol_tray::features::resident_policy::run_cli(&args))
        }
        Invocation::ResidentPolicyHidden(args) => {
            Some(qol_tray::features::resident_policy::run_hidden(&args))
        }
        Invocation::Exec { target, action } => Some(
            match qol_plugin_api::host_exec::run_exec(&target, &action) {
                Ok(()) => 0,
                Err(message) => {
                    eprintln!("{message}");
                    1
                }
            },
        ),
        Invocation::Open(route) => Some(forward_route(&route)),
        Invocation::UrlCourier(route) => Some(courier_forward_with_retry(&route)),
        Invocation::Url(route) => {
            if super::is_already_running() {
                Some(forward_route(&route))
            } else {
                let _ = super::PENDING_COLD_ROUTE.set(route);
                None
            }
        }
        Invocation::Invalid => {
            eprintln!("Invalid qol-tray invocation. Run `qol-tray help` for supported forms.");
            Some(2)
        }
    }
}

fn write_mode_flag(value: &str) -> i32 {
    let mode = match qol_tray::mode::ModeFlag::parse_cli(value) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{}", msg);
            return 1;
        }
    };
    if let Err(e) = qol_tray::mode::ModeConfig::set(mode) {
        eprintln!("Failed to write mode.json: {}", e);
        return 1;
    }
    println!("mode.json set to {:?}", mode);
    0
}

fn print_usage() {
    println!("qol-tray {}", super::qol_tray_version());
    println!();
    println!("USAGE:");
    println!("    qol-tray                              Run the tray daemon");
    println!(
        "    qol-tray exec <plugin_id> <action>    Trigger a plugin action via the running daemon"
    );
    println!("    qol-tray exec shortcut <id>           Run a shortcut via the running daemon");
    println!(
        "    qol-tray open <route>                 Open the app at an in-app route (e.g. shortcuts/add)"
    );
    println!("    qol-tray doctor                       Run read-only host and plugin checks");
    println!(
        "    qol-tray resident-policy <op>        Inspect or manage the durable NVIDIA residency policy; residency --resident|--portable toggles this device"
    );
    println!("    qol-tray --write-mode=<dev|prod>      Write mode.json then run the tray");
    println!("    qol-tray --version, -V                Print version and exit");
    println!("    qol-tray help, --help, -h             Print this message and exit");
}

/// Navigate an already-open UI tab to `route`, falling back to opening a fresh
/// browser tab. Shared by `qol-tray open` and the `qol://` courier.
fn forward_route(route: &str) -> i32 {
    if super::navigated_open_tab(route) {
        return 0;
    }
    let url = qol_tray::local_http::browser_url(route, qol_conventions::DEFAULT_PORT);
    match qol_tray::paths::open_url(&url) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Failed to open {url}: {e}");
            1
        }
    }
}

/// A macOS courier is spawned by the running daemon's URL delegate, but that
/// daemon's HTTP server may still be binding. Wait briefly (up to ~2s) for it to
/// accept connections so we navigate the live tab instead of opening a dead one,
/// then forward.
fn courier_forward_with_retry(route: &str) -> i32 {
    super::wait_for_server_ready();
    forward_route(route)
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
    if matches!(args.first().map(String::as_str), Some("resident-policy")) {
        return Invocation::ResidentPolicy(args[1..].to_vec());
    }
    if args
        .first()
        .is_some_and(|arg| arg.starts_with("__resident-policy-"))
    {
        return Invocation::ResidentPolicyHidden(args);
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
    fn classifies_resident_policy_routes_and_hidden_commands() {
        assert_eq!(
            classify(args(&["resident-policy", "status"])),
            Invocation::ResidentPolicy(args(&["status"]))
        );
        assert_eq!(
            classify(args(&["resident-policy", "help"])),
            Invocation::ResidentPolicy(args(&["help"]))
        );
        assert_eq!(
            classify(args(&[
                "__resident-policy-disable",
                "--policy",
                "nvidia-driver-version-pin",
            ])),
            Invocation::ResidentPolicyHidden(args(&[
                "__resident-policy-disable",
                "--policy",
                "nvidia-driver-version-pin",
            ]))
        );
        assert_eq!(
            classify(args(&[
                "__resident-policy-enable",
                "--policy",
                "nvidia-driver-version-pin",
                "--owner",
                "qol-resident-abc",
                "--bogus",
            ])),
            Invocation::ResidentPolicyHidden(args(&[
                "__resident-policy-enable",
                "--policy",
                "nvidia-driver-version-pin",
                "--owner",
                "qol-resident-abc",
                "--bogus",
            ])),
            "the hidden route carries the raw argv for strict parsing at the boundary"
        );
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
