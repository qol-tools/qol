use std::path::{Path, PathBuf};
use std::time::Duration;

use qol_headless::DoctorCheckResult;
use qol_platform::native::com::{Apartment, ComApartment};
use windows::core::BSTR;
use windows::Win32::Foundation::VARIANT_FALSE;
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NetFwPolicy2, NET_FW_ACTION_ALLOW, NET_FW_IP_PROTOCOL_UDP,
    NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_PROFILE_TYPE2,
    NET_FW_RULE_DIR_IN,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::super::{details, ports_label};
use super::rule::{self, AllowPlan, Ownership, RemovePlan, RULE_NAME};
use crate::config::ServerConfig;

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
    PublicLeftOut,
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
    program: String,
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
        FirewallState::PublicLeftOut => (
            DoctorCheckResult::warn(
                id,
                format!(
                    "This network is public, and PointZ's firewall rule leaves public networks out, so phones here cannot reach {ports}"
                ),
            )
            .with_fix(format!(
                "Run `{} allow-firewall --public` to admit phones on public networks too",
                env!("QOL_PLUGIN_ID")
            )),
            "public_left_out",
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

pub(crate) fn allow(public: bool) -> Result<String, String> {
    let _com = ComApartment::enter(Apartment::SingleThreaded);
    let policy = policy().map_err(|error| error.to_string())?;
    let ownership = ownership()?;
    let recorded = ownership.program()?;
    let program = pointz_program()?;
    let profiles = rule_profiles(public);
    let existing = existing_program(&policy);
    let arguments = match rule::allow_plan(recorded.as_deref(), existing.as_deref()) {
        AllowPlan::Add => rule::add_arguments(&program, &profiles),
        AllowPlan::Repair { recorded } => rule::repair_arguments(&recorded, &program, &profiles),
        AllowPlan::Refuse => {
            return Err(format!(
                "A firewall rule named \"{RULE_NAME}\" exists that PointZ did not create, so PointZ leaves it alone; remove or rename it, then run allow-firewall again"
            ))
        }
    };
    run_elevated(&netsh(), &arguments)?;
    ownership.record(&program)?;
    match read_state() {
        Ok(FirewallState::Allowed | FirewallState::Disabled) => Ok(format!(
            "Windows Firewall now admits PointZ on {}",
            ports_label()
        )),
        Ok(FirewallState::PublicLeftOut) => Ok(format!(
            "Windows Firewall now admits PointZ on {} on private and domain networks; this network is public, so phones on it stay blocked until you run `{} allow-firewall --public`",
            ports_label(),
            env!("QOL_PLUGIN_ID")
        )),
        Ok(_) => Err("The firewall rule was not applied".into()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn remove() -> Result<String, String> {
    let _com = ComApartment::enter(Apartment::SingleThreaded);
    let policy = policy().map_err(|error| error.to_string())?;
    let ownership = ownership()?;
    let recorded = ownership.program()?;
    match rule::remove_plan(recorded.as_deref(), existing_program(&policy).as_deref()) {
        RemovePlan::Nothing => Ok("PointZ created no firewall rule, so it removed nothing".into()),
        RemovePlan::Forget => {
            ownership.forget()?;
            Ok("The firewall rule PointZ created is already gone".into())
        }
        RemovePlan::Delete { recorded } => {
            run_elevated(&netsh(), &rule::delete_arguments(&recorded))?;
            ownership.forget()?;
            Ok(format!(
                "Removed the \"{RULE_NAME}\" firewall rule PointZ created"
            ))
        }
    }
}

fn ownership() -> Result<Ownership, String> {
    qol_config::data_dir()
        .map(|data_dir| Ownership::in_data_dir(&data_dir))
        .ok_or_else(|| "Windows reported no local app data folder for the firewall record".into())
}

fn pointz_program() -> Result<String, String> {
    let program = std::env::current_exe()
        .map_err(|error| format!("Windows did not report the PointZ program path: {error}"))?;
    program
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not a path netsh accepts", program.display()))
}

fn netsh() -> PathBuf {
    qol_process::system_program_path("netsh.exe")
}

fn existing_program(policy: &INetFwPolicy2) -> Option<String> {
    let rule = unsafe { policy.Rules() }
        .and_then(|rules| unsafe { rules.Item(&BSTR::from(RULE_NAME)) })
        .ok()?;
    Some(
        unsafe { rule.ApplicationName() }
            .map(|program| program.to_string())
            .unwrap_or_default(),
    )
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
    let program = pointz_program().unwrap_or_default();
    let facts = rule.ok().and_then(|rule| rule_facts(&rule).ok());
    Ok(classify(facts.as_ref(), active, &program))
}

fn classify(facts: Option<&RuleFacts>, active: i32, program: &str) -> FirewallState {
    match facts {
        Some(facts) if covers(facts, active, program) => FirewallState::Allowed,
        Some(facts) if covers(facts, active & !NET_FW_PROFILE2_PUBLIC.0, program) => {
            FirewallState::PublicLeftOut
        }
        _ => FirewallState::Missing,
    }
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
            program: rule
                .ApplicationName()
                .map(|program| program.to_string())
                .unwrap_or_default(),
        })
    }
}

fn covers(rule: &RuleFacts, active_profiles: i32, program: &str) -> bool {
    let listed: Vec<&str> = rule.ports.split(',').map(str::trim).collect();
    let has_port = |port: u16| listed.contains(&port.to_string().as_str());
    rule.enabled
        && rule.inbound
        && rule.allows
        && rule.udp
        && rule.profiles & active_profiles == active_profiles
        && (rule.program.is_empty() || rule::same_program(&rule.program, program))
        && has_port(ServerConfig::DISCOVERY_PORT)
        && has_port(ServerConfig::COMMAND_PORT)
}

fn rule_profiles(public: bool) -> String {
    PROFILES
        .iter()
        .filter(|(profile, _)| public || *profile != NET_FW_PROFILE2_PUBLIC)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(",")
}

fn run_elevated(program: &Path, arguments: &str) -> Result<(), String> {
    let hidden = qol_platform::native::shell::ShellSpawn {
        elevate: true,
        visible: false,
    };
    let process = qol_platform::native::shell::shell_spawn(program, arguments, hidden)
        .map_err(|error| format!("The administrator prompt was refused or failed: {error}"))?;
    match process.wait_timeout(ELEVATED_WAIT) {
        Ok(Some(0)) => Ok(()),
        Ok(Some(code)) => Err(format!("netsh exited with code {code}")),
        Ok(None) => Err("netsh did not finish in time".into()),
        Err(error) => Err(format!("could not read the netsh exit code: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = r"C:\Users\Ada\AppData\Local\qol\qol-pointz.exe";

    fn allowing_rule() -> RuleFacts {
        RuleFacts {
            enabled: true,
            inbound: true,
            allows: true,
            udp: true,
            profiles: NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_DOMAIN.0,
            ports: rule::ports(),
            program: PROGRAM.into(),
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
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "another program",
                RuleFacts {
                    program: r"C:\Tools\other.exe".into(),
                    ..rule.clone()
                },
                private,
                false,
            ),
            (
                "program in another case",
                RuleFacts {
                    program: PROGRAM.to_uppercase(),
                    ..rule.clone()
                },
                private,
                true,
            ),
            (
                "any program",
                RuleFacts {
                    program: String::new(),
                    ..rule
                },
                private,
                true,
            ),
        ];
        for (label, facts, active, expected) in cases {
            assert_eq!(covers(&facts, active, PROGRAM), expected, "{label}");
        }
    }

    #[test]
    fn new_rules_leave_public_networks_out_unless_asked() {
        assert_eq!(rule_profiles(false), "domain,private");
        assert_eq!(rule_profiles(true), "domain,private,public");
    }

    #[test]
    fn a_private_rule_on_a_public_network_is_reported_as_public_left_out() {
        let private = NET_FW_PROFILE2_PRIVATE.0;
        let public = NET_FW_PROFILE2_PUBLIC.0;
        let rule = allowing_rule();
        let everywhere = RuleFacts {
            profiles: rule.profiles | public,
            ..rule.clone()
        };
        let one_port = RuleFacts {
            ports: ServerConfig::DISCOVERY_PORT.to_string(),
            ..rule.clone()
        };
        let cases = [
            (
                "private network",
                Some(&rule),
                private,
                FirewallState::Allowed,
            ),
            (
                "public network",
                Some(&rule),
                public,
                FirewallState::PublicLeftOut,
            ),
            (
                "public and private networks",
                Some(&rule),
                public | private,
                FirewallState::PublicLeftOut,
            ),
            (
                "public rule",
                Some(&everywhere),
                public,
                FirewallState::Allowed,
            ),
            (
                "broken rule",
                Some(&one_port),
                public,
                FirewallState::Missing,
            ),
            ("no rule", None, public, FirewallState::Missing),
        ];
        for (label, facts, active, expected) in cases {
            assert_eq!(classify(facts, active, PROGRAM), expected, "{label}");
        }
    }

    #[test]
    fn the_check_warns_only_when_a_rule_is_missing_or_unreadable() {
        use qol_headless::DoctorStatus;

        let cases = [
            (FirewallState::Disabled, DoctorStatus::Ok, false),
            (FirewallState::Allowed, DoctorStatus::Ok, false),
            (FirewallState::PublicLeftOut, DoctorStatus::Warn, true),
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
        let public = check_result("firewall", FirewallState::PublicLeftOut);
        assert!(public.fix.unwrap().contains("allow-firewall --public"));
    }
}
