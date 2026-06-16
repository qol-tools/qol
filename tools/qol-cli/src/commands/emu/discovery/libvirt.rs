use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::media::BootMedia;
use super::super::{arch::GuestArch, sanitize_id, Environment, Firmware};

pub(crate) fn discover(virsh: Option<&Path>, uris: &[&str]) -> Vec<Environment> {
    let Some(virsh) = virsh else {
        return Vec::new();
    };
    let mut environments = Vec::new();
    for uri in uris {
        let Some(domains) = virsh_domains(virsh, uri) else {
            continue;
        };
        for domain in domains {
            let Some(image_path) = virsh_first_disk(virsh, uri, &domain) else {
                continue;
            };
            environments.push(Environment {
                id: sanitize_id(&domain),
                name: domain,
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path,
                source: format!("libvirt:{uri}"),
                firmware: Firmware::for_arch(GuestArch::X86_64),
                media: BootMedia::Disk,
            });
        }
    }
    environments
}

fn virsh_domains(virsh: &Path, uri: &str) -> Option<Vec<String>> {
    let output = Command::new(virsh)
        .args(["--connect", uri, "list", "--all", "--name"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn virsh_first_disk(virsh: &Path, uri: &str, domain: &str) -> Option<PathBuf> {
    let output = Command::new(virsh)
        .args(["--connect", uri, "domblklist", "--details", domain])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_virsh_disk_source(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_virsh_disk_source(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 4 && parts[1] == "disk" {
            Some(PathBuf::from(parts[3..].join(" ")))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_virsh_disk_source() {
        let output = " Type   Device   Target   Source\n------------------------------------------------\n file   disk     vda      /var/lib/libvirt/images/win11.qcow2\n file   cdrom    sda      -\n";
        assert_eq!(
            parse_virsh_disk_source(output),
            Some(PathBuf::from("/var/lib/libvirt/images/win11.qcow2"))
        );
    }

    #[test]
    fn parses_virsh_disk_source_with_spaces() {
        let output = " Type   Device   Target   Source\n------------------------------------------------\n file   disk     vda      /home/me/Virtual Machines/win11.qcow2\n";
        assert_eq!(
            parse_virsh_disk_source(output),
            Some(PathBuf::from("/home/me/Virtual Machines/win11.qcow2"))
        );
    }
}
