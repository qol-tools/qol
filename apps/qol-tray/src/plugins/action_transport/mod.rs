use std::path::Path;

mod protocol;

#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(not(any(unix, windows)))]
use unsupported as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonActionDispatch {
    Handled,
    Fallback,
    Error(String),
    Unavailable,
}

pub fn dispatch_daemon_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return DaemonActionDispatch::Fallback;
    }
    platform::dispatch_action(endpoint, action_id)
}

fn payload_candidates(action_id: &str) -> Vec<String> {
    let mut payloads = vec![format!("action:{action_id}\n"), format!("{action_id}\n")];
    if action_id == "open" {
        payloads.push("show".to_string());
    }
    payloads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_candidates_for_open_include_show_alias() {
        let payloads = payload_candidates("open");
        assert_eq!(
            payloads,
            vec![
                "action:open\n".to_string(),
                "open\n".to_string(),
                "show".to_string()
            ]
        );
    }

    #[test]
    fn payload_candidates_for_non_open_exclude_show_alias() {
        let payloads = payload_candidates("reload");
        assert_eq!(
            payloads,
            vec!["action:reload\n".to_string(), "reload\n".to_string()]
        );
    }
}
