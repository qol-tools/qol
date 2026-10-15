use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;

use super::names::{self, base};
use super::{lenient, output, Running};
use crate::updates::{Held, Package, Unit, Update};

const PHASED: &str = "The following upgrades have been deferred due to phasing:";
const KEPT: &str = "The following packages have been kept back:";

#[derive(Debug, Default, PartialEq)]
pub(super) struct Sim {
    pub(super) installs: Vec<Inst>,
    pub(super) phased: Vec<String>,
    pub(super) kept: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(super) struct Inst {
    pub(super) name: String,
    pub(super) old: Option<String>,
    pub(super) new: String,
    pub(super) security: bool,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Policy {
    pub(super) installed: Option<String>,
    pub(super) candidate: Option<String>,
    pub(super) phased: Option<u8>,
    pub(super) security: bool,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Record {
    pub(super) name: String,
    pub(super) source: Option<String>,
    pub(super) size: Option<u64>,
    pub(super) summary: String,
}

pub(super) fn parse_sim(text: &str) -> Sim {
    let mut sim = Sim::default();
    let mut section: Option<bool> = None;
    for line in text.lines() {
        if let Some(names) = line.strip_prefix("  ") {
            match section {
                Some(true) => sim
                    .phased
                    .extend(names.split_whitespace().map(String::from)),
                Some(false) => sim.kept.extend(names.split_whitespace().map(String::from)),
                None => {}
            }
            continue;
        }
        section = match line.trim_end() {
            PHASED => Some(true),
            KEPT => Some(false),
            _ => None,
        };
        if let Some(inst) = line.strip_prefix("Inst ").and_then(parse_inst) {
            sim.installs.push(inst);
        }
    }
    sim
}

fn parse_inst(line: &str) -> Option<Inst> {
    let (name, rest) = line.split_once(' ')?;
    let (old, rest) = match rest.strip_prefix('[') {
        Some(bracketed) => {
            let (old, rest) = bracketed.split_once("] ")?;
            (Some(old.to_string()), rest)
        }
        None => (None, rest),
    };
    let inside = rest.strip_prefix('(')?;
    let inside = &inside[..inside.rfind(')')?];
    let (new, archives) = inside.split_once(' ')?;
    Some(Inst {
        name: name.to_string(),
        old,
        new: new.to_string(),
        security: archives.contains("-security"),
    })
}

pub(super) fn parse_policy(text: &str) -> BTreeMap<String, Policy> {
    let mut policies = BTreeMap::new();
    let mut current: Option<(String, Policy)> = None;
    let mut in_candidate = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            if let Some(name) = line.strip_suffix(':') {
                if let Some((name, policy)) = current.take() {
                    policies.insert(name, policy);
                }
                current = Some((name.to_string(), Policy::default()));
                in_candidate = false;
            }
            continue;
        }
        let Some((_, policy)) = current.as_mut() else {
            continue;
        };
        let trimmed = line.trim();
        if let Some(version) = trimmed.strip_prefix("Installed: ") {
            policy.installed = (version != "(none)").then(|| version.to_string());
        } else if let Some(version) = trimmed.strip_prefix("Candidate: ") {
            policy.candidate = (version != "(none)").then(|| version.to_string());
        } else if line.starts_with("     ") && !line.starts_with("        ")
            || line.starts_with(" *** ")
        {
            let version = trimmed.trim_start_matches("*** ").split_whitespace().next();
            in_candidate = version.is_some() && version == policy.candidate.as_deref();
            if in_candidate {
                policy.phased = trimmed
                    .split_once("(phased ")
                    .and_then(|(_, rest)| rest.split_once('%'))
                    .and_then(|(percent, _)| percent.parse().ok());
            }
        } else if in_candidate && trimmed.contains("-security") {
            policy.security = true;
        }
    }
    if let Some((name, policy)) = current {
        policies.insert(name, policy);
    }
    policies
}

pub(super) fn parse_show(text: &str, native: &str) -> Vec<Record> {
    text.split("\n\n")
        .filter_map(|block| {
            let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
            for line in block.lines() {
                if line.starts_with(' ') {
                    continue;
                }
                if let Some((key, value)) = line.split_once(": ") {
                    fields.entry(key).or_insert(value);
                }
            }
            let package = fields.get("Package")?;
            let arch = fields.get("Architecture").copied().unwrap_or("all");
            let name = if arch == native || arch == "all" {
                package.to_string()
            } else {
                format!("{package}:{arch}")
            };
            Some(Record {
                name,
                source: fields.get("Source").map(|source| {
                    source
                        .split_whitespace()
                        .next()
                        .unwrap_or(source)
                        .to_string()
                }),
                size: fields.get("Size").and_then(|size| size.parse().ok()),
                summary: fields
                    .get("Description-en")
                    .or_else(|| fields.get("Description"))
                    .map(|summary| summary.to_string())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

struct Item {
    name: String,
    version: String,
    installed: Option<String>,
    target: Option<String>,
    security: bool,
    held: Option<Held>,
}

pub(super) fn read(running: &Running, tag: &str) -> Result<Vec<Update>, String> {
    let sim = parse_sim(&output(
        "apt-get",
        &[
            "-s",
            "-o",
            "Debug::NoLocking=1",
            "--no-remove",
            "dist-upgrade",
        ],
    )?);
    let holds: BTreeSet<String> = output("apt-mark", &["showhold"])
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    let held_names: Vec<&str> = sim
        .phased
        .iter()
        .chain(&sim.kept)
        .map(String::as_str)
        .collect();
    let policies = if held_names.is_empty() {
        BTreeMap::new()
    } else {
        let mut args = vec!["policy"];
        args.extend(&held_names);
        parse_policy(&output("apt-cache", &args)?)
    };
    let items = items(&sim, &holds, &policies);
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let native = output("dpkg", &["--print-architecture"])?
        .trim()
        .to_string();
    let queries: Vec<String> = items
        .iter()
        .map(|item| format!("{}={}", item.name, item.version))
        .collect();
    let mut args = vec!["show", "--no-all-versions"];
    args.extend(queries.iter().map(String::as_str));
    let records: BTreeMap<String, Record> = parse_show(&lenient("apt-cache", &args), &native)
        .into_iter()
        .map(|record| (record.name.clone(), record))
        .collect();
    Ok(group(items, &records, running, tag))
}

fn items(sim: &Sim, holds: &BTreeSet<String>, policies: &BTreeMap<String, Policy>) -> Vec<Item> {
    let mut items: Vec<Item> = sim
        .installs
        .iter()
        .map(|inst| Item {
            name: inst.name.clone(),
            version: inst.new.clone(),
            installed: inst.old.clone(),
            target: inst.old.as_ref().map(|_| inst.name.clone()),
            security: inst.security,
            held: None,
        })
        .collect();
    for (name, phased) in sim
        .phased
        .iter()
        .map(|name| (name, true))
        .chain(sim.kept.iter().map(|name| (name, false)))
    {
        let Some(policy) = policies.get(name) else {
            continue;
        };
        let Some(candidate) = policy.candidate.clone() else {
            continue;
        };
        let held = match (phased, policy.phased) {
            (true, percent) => Held::Phasing(percent.unwrap_or(0)),
            (false, _) if holds.contains(name) || holds.contains(base(name)) => Held::Administrator,
            (false, _) => Held::KeptBack,
        };
        items.push(Item {
            name: name.clone(),
            version: candidate,
            installed: policy.installed.clone(),
            target: None,
            security: policy.security,
            held: Some(held),
        });
    }
    items
}

fn group(
    items: Vec<Item>,
    records: &BTreeMap<String, Record>,
    running: &Running,
    tag: &str,
) -> Vec<Update> {
    let mut groups: IndexMap<(String, Option<Held>), Vec<Item>> = IndexMap::new();
    for item in items {
        let source = records
            .get(&item.name)
            .and_then(|record| record.source.clone())
            .unwrap_or_else(|| base(&item.name).to_string());
        groups
            .entry((names::key(&source), item.held))
            .or_default()
            .push(item);
    }
    groups
        .into_iter()
        .map(|((key, held), items)| {
            let source = items
                .iter()
                .find_map(|item| {
                    records
                        .get(&item.name)
                        .and_then(|record| record.source.clone())
                })
                .unwrap_or_else(|| key.clone());
            let lead = items
                .iter()
                .find(|item| base(&item.name) == source)
                .unwrap_or(&items[0]);
            let package_names: Vec<String> = items.iter().map(|item| item.name.clone()).collect();
            let naming = names::naming(&source, &lead.version, &package_names);
            let security = items.iter().any(|item| item.security);
            let running = if names::is_kernel(&source) {
                running.kernel.clone()
            } else if naming.caution.is_some() {
                running.nvidia.clone()
            } else {
                None
            };
            let why = match held {
                Some(Held::Phasing(percent)) => {
                    format!("Ubuntu sends it to {percent}% of computers first")
                }
                Some(Held::Administrator) => match running
                    .clone()
                    .or_else(|| lead.installed.as_deref().map(names::short_version))
                {
                    Some(version) => format!("An administrator holds it at {version}"),
                    None => "An administrator holds it where it is".to_string(),
                },
                Some(Held::KeptBack) => {
                    "Apt keeps it back until its packages fit together".to_string()
                }
                None => format!(
                    "{} {}",
                    if security {
                        "A security fix."
                    } else {
                        "A new version."
                    },
                    if naming.restart {
                        "Needs a restart."
                    } else {
                        "Restarts nothing."
                    }
                ),
            };
            let finish = match (naming.restart, running.as_deref()) {
                (true, Some(version)) => {
                    format!("Installed. Until a restart, {version} keeps running.")
                }
                (true, None) => "Installed. It finishes at the restart.".to_string(),
                (false, _) => "Installed. Nothing restarts.".to_string(),
            };
            Update {
                id: format!("apt:{key}{}", if held.is_some() { ":held" } else { "" }),
                tag: tag.to_string(),
                label: naming.label,
                name: naming.name,
                why,
                restart: naming.restart,
                held,
                unit: Unit::Packages,
                packages: items
                    .iter()
                    .map(|item| {
                        let record = records.get(&item.name);
                        Package {
                            name: item.name.clone(),
                            detail: record
                                .map(|record| record.summary.clone())
                                .unwrap_or_default(),
                            bytes: record.and_then(|record| record.size),
                            target: item.target.clone().unwrap_or_default(),
                        }
                    })
                    .collect(),
                running,
                finish,
                stops_sound: naming.stops_sound,
                caution: naming.caution,
                unattended: held.is_none(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIM: &str = "Calculating upgrade...
The following upgrades have been deferred due to phasing:
  apparmor libapparmor1 libapparmor1:i386
The following packages have been kept back:
  libnvidia-gl-580 libnvidia-gl-580:i386
  nvidia-driver-580
The following packages will be upgraded:
  containerd.io python3-urllib3
2 upgraded, 1 newly installed, 0 to remove and 6 not upgraded.
Inst containerd.io [2.4.1-2~ubuntu.24.04~noble] (2.4.2-1~ubuntu.24.04~noble Docker CE:noble [amd64])
Inst python3-urllib3 [2.0.7-1ubuntu0.7] (2.0.7-1ubuntu0.8 Ubuntu:24.04/noble-security [all])
Inst linux-image-6.8.0-148-lowlatency (6.8.0-148.148.1 Ubuntu:24.04/noble-updates, Ubuntu:24.04/noble-security [amd64])
Conf python3-urllib3 (2.0.7-1ubuntu0.8 Ubuntu:24.04/noble-security [all])
";

    const POLICY: &str = "apparmor:
  Installed: 4.0.1really4.0.1-0ubuntu0.24.04.8
  Candidate: 4.0.1really4.0.1-0ubuntu0.24.04.9
  Version table:
     4.0.1really4.0.1-0ubuntu0.24.04.9 500 (phased 10%)
        500 http://archive.ubuntu.com/ubuntu noble-updates/main amd64 Packages
 *** 4.0.1really4.0.1-0ubuntu0.24.04.8 100
        100 /var/lib/dpkg/status
     4.0.0-beta3-0ubuntu3 500
        500 http://archive.ubuntu.com/ubuntu noble-security/main amd64 Packages
nvidia-driver-580:
  Installed: 580.173.02-0ubuntu0.24.04.1
  Candidate: 580.178.04-0ubuntu0.24.04.1
  Version table:
     580.178.04-0ubuntu0.24.04.1 500
        500 http://archive.ubuntu.com/ubuntu noble-updates/restricted amd64 Packages
        500 http://archive.ubuntu.com/ubuntu noble-security/restricted amd64 Packages
 *** 580.173.02-0ubuntu0.24.04.1 100
        100 /var/lib/dpkg/status
libnvidia-gl-580:i386:
  Installed: 580.173.02-0ubuntu0.24.04.1
  Candidate: 580.178.04-0ubuntu0.24.04.1
  Version table:
     580.178.04-0ubuntu0.24.04.1 500
        500 http://archive.ubuntu.com/ubuntu noble-security/restricted i386 Packages
";

    const SHOW: &str = "Package: python3-urllib3
Source: python-urllib3
Architecture: all
Version: 2.0.7-1ubuntu0.8
Size: 95900
Description-en: HTTP library with thread-safe connection pooling for Python3
 urllib3 supports features left out of urllib and urllib2 libraries.

Package: libnvidia-gl-580
Source: nvidia-graphics-drivers-580 (580.178.04-0ubuntu0.24.04.1)
Architecture: i386
Version: 580.178.04-0ubuntu0.24.04.1
Size: 44529606
Description: NVIDIA OpenGL/GLX/EGL/GLES GLVND libraries and Vulkan ICD

Package: nvidia-driver-580
Source: nvidia-graphics-drivers-580
Architecture: amd64
Version: 580.178.04-0ubuntu0.24.04.1
Size: 510324
Description: NVIDIA driver metapackage

Package: containerd.io
Architecture: amd64
Version: 2.4.2-1~ubuntu.24.04~noble
Size: 30000000
Description: An open and reliable container runtime
";

    #[test]
    fn the_simulation_names_upgrades_new_packages_and_both_holds() {
        let sim = parse_sim(SIM);
        assert_eq!(
            sim.phased,
            ["apparmor", "libapparmor1", "libapparmor1:i386"]
        );
        assert_eq!(
            sim.kept,
            [
                "libnvidia-gl-580",
                "libnvidia-gl-580:i386",
                "nvidia-driver-580"
            ]
        );
        assert_eq!(
            sim.installs[1],
            Inst {
                name: "python3-urllib3".into(),
                old: Some("2.0.7-1ubuntu0.7".into()),
                new: "2.0.7-1ubuntu0.8".into(),
                security: true,
            }
        );
        assert!(!sim.installs[0].security);
        assert_eq!(sim.installs[2].old, None);
        assert!(sim.installs[2].security);
    }

    #[test]
    fn the_policy_reads_the_candidate_phasing_and_security() {
        let policies = parse_policy(POLICY);
        assert_eq!(policies["apparmor"].phased, Some(10));
        assert!(!policies["apparmor"].security);
        assert_eq!(
            policies["nvidia-driver-580"].candidate.as_deref(),
            Some("580.178.04-0ubuntu0.24.04.1")
        );
        assert!(policies["nvidia-driver-580"].security);
        assert_eq!(policies["nvidia-driver-580"].phased, None);
        assert!(policies["libnvidia-gl-580:i386"].security);
    }

    #[test]
    fn records_carry_their_foreign_arch_and_bare_source() {
        let records = parse_show(SHOW, "amd64");
        assert_eq!(records[0].name, "python3-urllib3");
        assert_eq!(records[0].source.as_deref(), Some("python-urllib3"));
        assert_eq!(records[1].name, "libnvidia-gl-580:i386");
        assert_eq!(
            records[1].source.as_deref(),
            Some("nvidia-graphics-drivers-580")
        );
        assert_eq!(
            records[1].summary,
            "NVIDIA OpenGL/GLX/EGL/GLES GLVND libraries and Vulkan ICD"
        );
        assert_eq!(records[3].source, None);
        assert_eq!(records[3].size, Some(30_000_000));
    }

    #[test]
    fn packages_group_by_source_with_holds_apart() {
        let sim = parse_sim(SIM);
        let holds: BTreeSet<String> = ["nvidia-driver-580", "libnvidia-gl-580"]
            .into_iter()
            .map(String::from)
            .collect();
        let records: BTreeMap<String, Record> = parse_show(SHOW, "amd64")
            .into_iter()
            .map(|record| (record.name.clone(), record))
            .collect();
        let running = Running {
            kernel: Some("6.8.0-146".into()),
            nvidia: Some("580.173".into()),
        };
        let updates = group(
            items(&sim, &holds, &parse_policy(POLICY)),
            &records,
            &running,
            "Update Manager",
        );
        let summary: Vec<(String, String, usize, Option<Held>)> = updates
            .iter()
            .map(|update| {
                (
                    update.id.clone(),
                    update.label.clone(),
                    update.count(),
                    update.held,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "apt:containerd.io".into(),
                    "containerd.io 2.4.2".into(),
                    1,
                    None
                ),
                ("apt:python-urllib3".into(), "urllib3 2.0.7".into(), 1, None),
                ("apt:linux".into(), "Linux kernel 6.8.0-148".into(), 1, None),
                (
                    "apt:apparmor:held".into(),
                    "AppArmor 4.0.1".into(),
                    1,
                    Some(Held::Phasing(10))
                ),
                (
                    "apt:nvidia-graphics-drivers-580:held".into(),
                    "NVIDIA driver 580.178".into(),
                    2,
                    Some(Held::Administrator)
                ),
            ]
        );
        assert_eq!(updates[1].why, "A security fix. Restarts nothing.");
        assert_eq!(updates[1].packages[0].target, "python3-urllib3");
        assert_eq!(updates[2].packages[0].target, "");
        assert_eq!(updates[2].why, "A security fix. Needs a restart.");
        assert_eq!(
            updates[2].finish,
            "Installed. Until a restart, 6.8.0-146 keeps running."
        );
        assert_eq!(updates[3].why, "Ubuntu sends it to 10% of computers first");
        assert_eq!(updates[4].why, "An administrator holds it at 580.173");
        assert!(!updates[4].unattended);
    }
}
