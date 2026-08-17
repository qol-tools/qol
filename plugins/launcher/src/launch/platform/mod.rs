#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{daemon_action_args, launch_app};
#[cfg(target_os = "linux")]
pub(super) use linux::{daemon_action_args, launch_app};
#[cfg(target_os = "macos")]
pub(super) use macos::{daemon_action_args, launch_app};
#[cfg(target_os = "windows")]
pub(super) use windows::{daemon_action_args, launch_app};

pub(super) fn daemon_exec_args(exec: &[String]) -> Option<(&str, &str)> {
    match exec {
        [_, verb, target, action] if verb == "exec" => Some((target, action)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::daemon_exec_args;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn daemon_exec_args_accepts_qol_action_shape() {
        let exec = args(&["/opt/qol-tray", "exec", "plugin-monitor", "settings"]);

        assert_eq!(
            daemon_exec_args(&exec),
            Some(("plugin-monitor", "settings"))
        );
    }

    #[test]
    fn daemon_exec_args_rejects_wrong_arg_counts() {
        assert_eq!(daemon_exec_args(&args(&["/opt/qol-tray"])), None);
        assert_eq!(
            daemon_exec_args(&args(&["/opt/qol-tray", "exec", "plugin-monitor"])),
            None
        );
        assert_eq!(
            daemon_exec_args(&args(&[
                "/opt/qol-tray",
                "exec",
                "plugin-monitor",
                "settings",
                "extra",
            ])),
            None
        );
    }

    #[test]
    fn daemon_exec_args_rejects_non_exec_verbs() {
        assert_eq!(
            daemon_exec_args(&args(&[
                "/opt/qol-tray",
                "open",
                "plugin-monitor",
                "settings"
            ])),
            None
        );
        assert_eq!(
            daemon_exec_args(&args(&[
                "/opt/qol-tray",
                "EXEC",
                "plugin-monitor",
                "settings"
            ])),
            None
        );
    }
}
