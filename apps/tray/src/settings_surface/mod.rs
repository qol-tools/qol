mod platform;

use std::time::Duration;

use qol_runtime::protocol::NotificationLayout;

const HOST_ARGUMENT: &str = "__qol-settings-surface-host";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreTool {
    AddHotkey,
    AddShortcut,
    Hotkeys,
    Shortcuts,
    Updates,
    LinkedDevices,
    Profiles,
    Plugins,
}

impl CoreTool {
    pub(crate) fn wire_id(self) -> &'static str {
        match self {
            Self::AddHotkey => "__core-hotkeys-add",
            Self::AddShortcut => "__core-shortcuts-add",
            Self::Hotkeys => "__core-hotkeys",
            Self::Shortcuts => "__core-shortcuts",
            Self::Updates => "__core-updates",
            Self::LinkedDevices => "__core-linked-devices",
            Self::Profiles => "__core-profiles",
            Self::Plugins => "__core-plugins",
        }
    }

    pub(crate) fn from_wire_id(value: &str) -> Option<Self> {
        match value {
            "__core-hotkeys-add" => Some(Self::AddHotkey),
            "__core-shortcuts-add" => Some(Self::AddShortcut),
            "__core-hotkeys" => Some(Self::Hotkeys),
            "__core-shortcuts" => Some(Self::Shortcuts),
            "__core-updates" => Some(Self::Updates),
            "__core-linked-devices" => Some(Self::LinkedDevices),
            "__core-profiles" => Some(Self::Profiles),
            "__core-plugins" => Some(Self::Plugins),
            _ => None,
        }
    }

    pub(crate) fn page_wire_id(self) -> &'static str {
        match self {
            Self::AddHotkey | Self::Hotkeys => "__core-hotkeys",
            Self::AddShortcut | Self::Shortcuts => "__core-shortcuts",
            Self::Updates => "__core-updates",
            Self::LinkedDevices => "__core-linked-devices",
            Self::Profiles => "__core-profiles",
            Self::Plugins => "__core-plugins",
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
    platform::request(plugin_id)
}

pub(crate) fn request_core_tool(tool: CoreTool) -> anyhow::Result<bool> {
    platform::request(tool.wire_id())
}

pub fn open_updates() -> anyhow::Result<bool> {
    request_core_tool(CoreTool::Updates)
}

pub fn stop() {
    platform::stop();
}

pub fn apply_theme(native: &str, accent: &str) -> bool {
    platform::apply_theme(native, accent)
}

pub fn plugins_changed() -> bool {
    platform::plugins_changed()
}

#[derive(Clone, Copy)]
pub struct ToastSource<'a> {
    pub group: &'a str,
    pub name: &'a str,
}

pub fn show_toast(
    source: ToastSource<'_>,
    title: &str,
    body: &str,
    level: &str,
    action: Option<(&str, &str)>,
    artifact: Option<&str>,
    layout: Option<NotificationLayout>,
) -> anyhow::Result<bool> {
    platform::show_toast(source, title, body, level, action, artifact, layout)
}

pub fn prewarm() {
    platform::prewarm();
}

pub fn wait_until_ready(timeout: Duration) -> bool {
    platform::wait_until_ready(timeout)
}

pub fn run_from_current_args() -> Option<anyhow::Result<()>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    requested_boot(&args).map(|boot| {
        qol_log::init_stderr();
        platform::run(boot)
    })
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
    use super::{requested_boot, CoreTool, HostBoot, HOST_ARGUMENT};

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
    fn core_tool_wire_ids_round_trip_without_colliding_with_plugin_ids() {
        for tool in [
            CoreTool::AddHotkey,
            CoreTool::AddShortcut,
            CoreTool::Hotkeys,
            CoreTool::Shortcuts,
            CoreTool::Updates,
            CoreTool::LinkedDevices,
            CoreTool::Profiles,
            CoreTool::Plugins,
        ] {
            assert_eq!(CoreTool::from_wire_id(tool.wire_id()), Some(tool));
            assert!(tool.wire_id().starts_with("__core-"));
        }
        assert_eq!(CoreTool::from_wire_id("qol-monitor"), None);
    }
}
