use anyhow::Result;

use crate::config::ReconnectConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonAction {
    Ping,
    Kill,
    EnableAdapter,
    DisableAdapter,
    PairDevice,
    TrustDevice,
    UntrustDevice,
    ConnectDevice,
    DisconnectDevice,
    ReclaimDevice,
    RemoveDevice,
    StartSearch,
    StopSearch,
    OpenSystemSettings,
    Devices,
    NearbyDevices,
    ManagedDeviceOptions,
    AdapterOptions,
    SearchStatus,
    AdapterStatus,
    Reconnect,
    ReconnectTrusted,
    Reload,
    Settings,
    HostFixes,
    HostProblems,
    ApplyHostFix,
    HandoffState,
    ReleaseForHandoff,
    ResumeReconnect,
}

pub enum DaemonReply<C> {
    Command(C),
    Handled,
    Data(serde_json::Value),
    Error(String),
}

#[derive(Clone, Copy)]
pub struct DaemonHooks {
    pub open_settings_page: fn() -> Result<()>,
    pub forget_managed: fn(&ReconnectConfig, &str),
}
