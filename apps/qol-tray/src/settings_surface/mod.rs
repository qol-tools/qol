mod platform;

use qol_runtime::protocol::NotificationLayout;

const HOST_ARGUMENT: &str = "__qol-settings-surface-host";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreTool {
    AddHotkey,
    AddShortcut,
    Hotkeys,
    Shortcuts,
}

impl CoreTool {
    pub(crate) fn wire_id(self) -> &'static str {
        match self {
            Self::AddHotkey => "__core-hotkeys-add",
            Self::AddShortcut => "__core-shortcuts-add",
            Self::Hotkeys => "__core-hotkeys",
            Self::Shortcuts => "__core-shortcuts",
        }
    }

    pub(crate) fn from_wire_id(value: &str) -> Option<Self> {
        match value {
            "__core-hotkeys-add" => Some(Self::AddHotkey),
            "__core-shortcuts-add" => Some(Self::AddShortcut),
            "__core-hotkeys" => Some(Self::Hotkeys),
            "__core-shortcuts" => Some(Self::Shortcuts),
            _ => None,
        }
    }

    pub(crate) fn fallback_route(self) -> &'static str {
        match self {
            Self::AddHotkey => "hotkeys",
            Self::AddShortcut => "shortcuts/add",
            Self::Hotkeys => "hotkeys",
            Self::Shortcuts => "shortcuts",
        }
    }
}

#[derive(Debug, PartialEq)]
enum HostBoot {
    Warm,
    Open(String),
}

pub fn native_available() -> bool {
    platform::native_available()
}

pub fn request(plugin_id: &str) -> anyhow::Result<bool> {
    let handled = platform::request(plugin_id)?;
    finish_request(plugin_id, handled, crate::paths::open_url)
}

pub(crate) fn request_core_tool(tool: CoreTool) -> anyhow::Result<bool> {
    match platform::request(tool.wire_id()) {
        Ok(true) => Ok(true),
        result => {
            let url = qol_conventions::local_hash_url(
                tool.fallback_route(),
                qol_conventions::DEFAULT_PORT,
            );
            crate::paths::open_url(&url)?;
            let reason = if result.is_ok() {
                "platform_unsupported"
            } else {
                "native_failed"
            };
            qol_runtime::probe!(
                "SURFACE_ACTIVATION",
                "tool={} phase=fallback reason={reason} outcome=opened",
                tool.wire_id()
            );
            Ok(true)
        }
    }
}

fn finish_request(
    plugin_id: &str,
    handled: bool,
    open_url: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    if handled {
        return Ok(true);
    }
    open_url(&qol_conventions::settings_url(plugin_id))?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=fallback reason=platform_unsupported outcome=opened"
    );
    Ok(true)
}

pub fn stop() {
    platform::stop();
}

pub fn apply_theme(native: &str, accent: &str) -> bool {
    platform::apply_theme(native, accent)
}

pub fn show_toast(
    title: &str,
    body: &str,
    level: &str,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
) -> anyhow::Result<bool> {
    platform::show_toast(title, body, level, action, artifact, layout)
}

pub fn prewarm() {
    platform::prewarm();
}

pub fn run_from_current_args() -> Option<anyhow::Result<()>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    requested_boot(&args).map(platform::run)
}

fn requested_boot(args: &[String]) -> Option<HostBoot> {
    match args {
        [argument] if argument == HOST_ARGUMENT => Some(HostBoot::Warm),
        [argument, plugin_id] if argument == HOST_ARGUMENT => {
            Some(HostBoot::Open(plugin_id.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{finish_request, requested_boot, CoreTool, HostBoot, HOST_ARGUMENT};

    #[test]
    fn native_availability_matches_platform_dispatch() {
        let expected = cfg!(any(target_os = "linux", target_os = "macos"));
        assert_eq!(super::native_available(), expected);
    }

    #[test]
    fn hidden_host_arguments_select_warm_or_single_plugin_boot() {
        let cases = [
            (vec![], None),
            (vec!["settings"], None),
            (vec![HOST_ARGUMENT], Some(HostBoot::Warm)),
            (
                vec![HOST_ARGUMENT, "plugin-a"],
                Some(HostBoot::Open("plugin-a".into())),
            ),
            (vec![HOST_ARGUMENT, "plugin-a", "extra"], None),
        ];
        for (args, expected) in cases {
            let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            assert_eq!(requested_boot(&args), expected, "args: {args:?}");
        }
    }

    #[test]
    fn unsupported_native_surface_opens_browser_fallback() {
        let mut opened = None;
        let handled = finish_request("plugin-a", false, |url| {
            opened = Some(url.to_string());
            Ok(())
        })
        .unwrap();

        assert!(handled);
        assert_eq!(opened, Some(qol_conventions::settings_url("plugin-a")));
    }

    #[test]
    fn handled_native_surface_skips_browser_fallback() {
        let handled = finish_request("plugin-a", true, |_| {
            panic!("handled native settings must not open the browser")
        })
        .unwrap();

        assert!(handled);
    }

    #[test]
    fn core_tool_wire_ids_round_trip_without_colliding_with_plugin_ids() {
        for tool in [
            CoreTool::AddHotkey,
            CoreTool::AddShortcut,
            CoreTool::Hotkeys,
            CoreTool::Shortcuts,
        ] {
            assert_eq!(CoreTool::from_wire_id(tool.wire_id()), Some(tool));
            assert!(tool.wire_id().starts_with("__core-"));
        }
        assert_eq!(CoreTool::from_wire_id("plugin-monitor"), None);
    }
}
