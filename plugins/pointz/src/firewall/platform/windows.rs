use std::time::Duration;

use qol_headless::DoctorCheckResult;
use qol_platform::native::com::{Apartment, ComApartment};
use windows::core::{BSTR, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, VARIANT_FALSE, WAIT_OBJECT_0};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NetFwPolicy2, NET_FW_ACTION_ALLOW, NET_FW_IP_PROTOCOL_UDP,
    NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_PROFILE_TYPE2,
    NET_FW_RULE_DIR_IN,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::super::{details, ports_label};
use crate::config::ServerConfig;

const RULE_NAME: &str = "qol PointZ";
const RULE_DESCRIPTION: &str = "Lets phones find and control this PC through qol PointZ";
const ELEVATED_WAIT: Duration = Duration::from_secs(120);
const PROFILES: [(NET_FW_PROFILE_TYPE2, &str); 3] = [
    (NET_FW_PROFILE2_DOMAIN, "domain"),
    (NET_FW_PROFILE2_PRIVATE, "private"),
    (NET_FW_PROFILE2_PUBLIC, "public"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum FirewallState {
    Disabled,
    Allowed,
    Missing,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleFacts {
    enabled: bool,
    inbound: bool,
    allows: bool,
    udp: bool,
    profiles: i32,
    ports: String,
}

pub(crate) fn check(id: &str) -> DoctorCheckResult {
    let _com = ComApartment::enter(Apartment::SingleThreaded);
    let state = read_state().unwrap_or_else(|error| FirewallState::Unreadable(error.to_string()));
    check_result(id, state)
}

fn check_result(id: &str, state: FirewallState) -> DoctorCheckResult {
    let ports = ports_label();
    let (result, label) = match state {
        FirewallState::Disabled => (
            DoctorCheckResult::ok(id, "Windows Firewall is off for the active network"),
            "disabled",
        ),
        FirewallState::Allowed => (
            DoctorCheckResult::ok(
                id,
                format!("Windows Firewall admits {ports} on the active network"),
            ),
            "allowed",
        ),
        FirewallState::Missing => (
            DoctorCheckResult::warn(
                id,
                format!("Windows Firewall may block phones from reaching {ports}"),
            )
            .with_fix(format!(
                "Run `{} allow-firewall` and approve the administrator prompt",
                env!("QOL_PLUGIN_ID")
            )),
            "missing",
        ),
        FirewallState::Unreadable(error) => (
            DoctorCheckResult::warn(
                id,
                format!("Windows Firewall rules could not be read: {error}"),
            )
            .with_fix("Check that the Windows Defender Firewall service is running"),
            "unreadable",
        ),
    };
    result.with_details(details(label))
}

pub(crate) fn allow() -> Result<String, String> {
    let _com = ComApartment::enter(Apartment::SingleThreaded);
    let policy = policy().map_err(|error| error.to_string())?;
    let active = unsafe { policy.CurrentProfileTypes() }.map_err(|error| error.to_string())?;
    let exists = unsafe { policy.Rules() }
        .and_then(|rules| unsafe { rules.Item(&BSTR::from(RULE_NAME)) })
        .is_ok();
    let arguments = netsh_arguments(exists, rule_profiles(active));
    run_elevated("netsh.exe", &arguments)?;
    match read_state() {
        Ok(FirewallState::Allowed | FirewallState::Disabled) => Ok(format!(
            "Windows Firewall now admits PointZ on {}",
            ports_label()
        )),
        Ok(_) => Err("The firewall rule was not applied".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_state() -> windows::core::Result<FirewallState> {
    let policy = policy()?;
    let active = unsafe { policy.CurrentProfileTypes() }?;
    let enabled = PROFILES
        .iter()
        .filter(|(profile, _)| active & profile.0 != 0)
        .any(|(profile, _)| {
            unsafe { policy.get_FirewallEnabled(*profile) }
                .map(|on| on != VARIANT_FALSE)
                .unwrap_or(true)
        });
    if !enabled {
        return Ok(FirewallState::Disabled);
    }
    let rule = unsafe { policy.Rules()?.Item(&BSTR::from(RULE_NAME)) };
    Ok(match rule.ok().and_then(|rule| rule_facts(&rule).ok()) {
        Some(facts) if covers(&facts, active) => FirewallState::Allowed,
        _ => FirewallState::Missing,
    })
}

fn policy() -> windows::core::Result<INetFwPolicy2> {
    unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER) }
}

fn rule_facts(rule: &INetFwRule) -> windows::core::Result<RuleFacts> {
    unsafe {
        Ok(RuleFacts {
            enabled: rule.Enabled()? != VARIANT_FALSE,
            inbound: rule.Direction()? == NET_FW_RULE_DIR_IN,
            allows: rule.Action()? == NET_FW_ACTION_ALLOW,
            udp: rule.Protocol()? == NET_FW_IP_PROTOCOL_UDP.0,
            profiles: rule.Profiles()?,
            ports: rule.LocalPorts()?.to_string(),
        })
    }
}

fn covers(rule: &RuleFacts, active_profiles: i32) -> bool {
    let listed: Vec<&str> = rule.ports.split(',').map(str::trim).collect();
    let has_port = |port: u16| listed.contains(&port.to_string().as_str());
    rule.enabled
        && rule.inbound
        && rule.allows
        && rule.udp
        && rule.profiles & active_profiles == active_profiles
        && has_port(ServerConfig::DISCOVERY_PORT)
        && has_port(ServerConfig::COMMAND_PORT)
}

fn rule_profiles(active: i32) -> String {
    let wanted = active | NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_DOMAIN.0;
    PROFILES
        .iter()
        .filter(|(profile, _)| wanted & profile.0 != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(",")
}

fn ports() -> String {
    format!(
        "{},{}",
        ServerConfig::DISCOVERY_PORT,
        ServerConfig::COMMAND_PORT
    )
}

fn netsh_arguments(exists: bool, profiles: String) -> String {
    let settings = format!(
        "dir=in action=allow protocol=UDP localport={} profile={profiles}",
        ports()
    );
    if exists {
        format!(r#"advfirewall firewall set rule name="{RULE_NAME}" new enable=yes {settings}"#)
    } else {
        format!(
            r#"advfirewall firewall add rule name="{RULE_NAME}" {settings} enable=yes description="{RULE_DESCRIPTION}""#
        )
    }
}

fn run_elevated(program: &str, arguments: &str) -> Result<(), String> {
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(program);
    let parameters = HSTRING::from(arguments);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
        .map_err(|error| format!("The administrator prompt was refused or failed: {error}"))?;
    if info.hProcess.is_invalid() {
        return Err("Windows did not start netsh".into());
    }
    let waited = unsafe { WaitForSingleObject(info.hProcess, ELEVATED_WAIT.as_millis() as u32) };
    let mut code = 1u32;
    let read = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    if waited != WAIT_OBJECT_0 {
        return Err("netsh did not finish in time".into());
    }
    read.map_err(|error| error.to_string())?;
    if code != 0 {
        return Err(format!("netsh exited with code {code}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowing_rule() -> RuleFacts {
        RuleFacts {
            enabled: true,
            inbound: true,
            allows: true,
            udp: true,
            profiles: NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_DOMAIN.0,
            ports: ports(),
        }
    }

    #[test]
    fn a_rule_covers_pointz_only_when_it_admits_both_ports_on_the_active_network() {
        let private = NET_FW_PROFILE2_PRIVATE.0;
        let public = NET_FW_PROFILE2_PUBLIC.0;
        let rule = allowing_rule();
        let cases = [
            ("matching", rule.clone(), private, true),
            ("public network", rule.clone(), public, false),
            (
                "spaced ports",
                RuleFacts {
                    ports: format!(
                        "{}, {}",
                        ServerConfig::DISCOVERY_PORT,
                        ServerConfig::COMMAND_PORT
                    ),
                    ..rule.clone()
                },
                private,
                true,
            ),
            (
                "one port",
                RuleFacts {
                    ports: ServerConfig::DISCOVERY_PORT.to_string(),
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "disabled",
                RuleFacts {
                    enabled: false,
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "blocks",
                RuleFacts {
                    allows: false,
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "tcp",
                RuleFacts {
                    udp: false,
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "outbound",
                RuleFacts {
                    inbound: false,
                    ..rule
                },
                private,
                false,
            ),
        ];
        for (label, facts, active, expected) in cases {
            assert_eq!(covers(&facts, active), expected, "{label}");
        }
    }

    #[test]
    fn new_rules_cover_private_networks_and_whatever_network_is_active() {
        let cases = [
            (NET_FW_PROFILE2_PRIVATE.0, "domain,private"),
            (NET_FW_PROFILE2_PUBLIC.0, "domain,private,public"),
            (0, "domain,private"),
        ];
        for (active, expected) in cases {
            assert_eq!(rule_profiles(active), expected, "active={active}");
        }
    }

    #[test]
    fn the_check_warns_only_when_a_rule_is_missing_or_unreadable() {
        use qol_headless::DoctorStatus;

        let cases = [
            (FirewallState::Disabled, DoctorStatus::Ok, false),
            (FirewallState::Allowed, DoctorStatus::Ok, false),
            (FirewallState::Missing, DoctorStatus::Warn, true),
            (
                FirewallState::Unreadable("service stopped".into()),
                DoctorStatus::Warn,
                true,
            ),
        ];
        for (state, status, has_fix) in cases {
            let label = format!("{state:?}");
            let result = check_result("firewall", state);
            assert_eq!(result.status, status, "{label}");
            assert_eq!(result.fix.is_some(), has_fix, "{label}");
            assert_eq!(result.details.unwrap()["rule_changed"], false, "{label}");
        }
        let missing = check_result("firewall", FirewallState::Missing);
        assert!(missing.fix.unwrap().contains("allow-firewall"));
    }

    #[test]
    fn netsh_adds_a_missing_rule_and_repairs_an_existing_one() {
        let add = netsh_arguments(false, "private".into());
        assert!(add.starts_with(r#"advfirewall firewall add rule name="qol PointZ" dir=in"#));
        assert!(add.contains(&format!("localport={}", ports())));
        assert!(add.contains("protocol=UDP"));
        let repair = netsh_arguments(true, "private".into());
        assert!(
            repair.starts_with(r#"advfirewall firewall set rule name="qol PointZ" new enable=yes"#)
        );
        assert!(!repair.contains("description="));
    }
}
