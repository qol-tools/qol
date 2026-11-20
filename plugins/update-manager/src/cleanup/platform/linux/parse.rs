use std::collections::{BTreeMap, BTreeSet};

use crate::platform::linux::packages::{base, kernel_abi};

pub(super) fn removals(simulated: &str) -> Vec<String> {
    simulated
        .lines()
        .filter_map(|line| line.strip_prefix("Remv "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(String::from)
        .collect()
}

pub(super) fn versioned_abi(package: &str) -> Option<String> {
    let rest = base(package).strip_prefix("linux-")?;
    let parts: Vec<&str> = rest.split('-').collect();
    parts.windows(2).find_map(|pair| {
        let numbers: Vec<&str> = pair[0].split('.').collect();
        let dotted = numbers.len() == 3 && numbers.iter().all(|part| part.parse::<u32>().is_ok());
        (dotted && pair[1].parse::<u32>().is_ok()).then(|| format!("{}-{}", pair[0], pair[1]))
    })
}

pub(super) fn kernels(packages: &[String]) -> usize {
    packages
        .iter()
        .filter_map(|package| versioned_abi(package))
        .collect::<BTreeSet<_>>()
        .len()
}

const NEWEST_KEPT: usize = 2;

pub(super) fn installed(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| line.strip_prefix("installed\t"))
        .map(String::from)
        .collect()
}

fn version(abi: &str) -> Vec<u32> {
    abi.split(['.', '-'])
        .filter_map(|part| part.parse().ok())
        .collect()
}

pub(super) fn by_hand(manual: &str, running: &str, installed: &[String]) -> (Vec<String>, usize) {
    let mut images: Vec<String> = installed
        .iter()
        .filter_map(|package| kernel_abi(package))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    images.sort_by_key(|abi| std::cmp::Reverse(version(abi)));
    let mut kept: BTreeSet<String> = images.into_iter().take(NEWEST_KEPT).collect();
    kept.insert(running.to_string());
    let packages: Vec<String> = manual
        .lines()
        .map(str::trim)
        .filter(|package| versioned_abi(package).is_some_and(|abi| !kept.contains(&abi)))
        .map(String::from)
        .collect();
    let images = packages
        .iter()
        .filter_map(|package| kernel_abi(package))
        .count();
    (packages, images)
}

pub(super) fn sizes(listing: &str) -> BTreeMap<String, u64> {
    listing
        .lines()
        .filter_map(|line| {
            let (package, kib) = line.split_once('\t')?;
            Some((
                package.trim().to_string(),
                kib.trim().parse::<u64>().ok()? * 1024,
            ))
        })
        .collect()
}

pub(super) fn disabled_snaps(listing: &str) -> Vec<String> {
    listing
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let notes = fields.last()?;
            (fields.len() >= 4 && notes.split(',').any(|note| note == "disabled"))
                .then(|| format!("{}_{}", fields[0], fields[2]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_removals_name_each_package_once() {
        let text = "NOTE: This is only a simulation!\n\
                    Reading package lists...\n\
                    Remv linux-image-6.8.0-111-generic [6.8.0-111.111]\n\
                    Remv linux-modules-6.8.0-111-generic [6.8.0-111.111]\n\
                    Remv libfoo1 [1.2-3]\n";
        let removed = removals(text);
        assert_eq!(
            removed,
            [
                "linux-image-6.8.0-111-generic",
                "linux-modules-6.8.0-111-generic",
                "libfoo1"
            ]
        );
        assert_eq!(kernels(&removed), 1);
    }

    #[test]
    fn only_versioned_kernel_packages_carry_an_abi() {
        assert_eq!(
            versioned_abi("linux-hwe-6.8-headers-6.8.0-111").as_deref(),
            Some("6.8.0-111")
        );
        assert_eq!(
            versioned_abi("linux-modules-extra-6.8.0-111-generic:amd64").as_deref(),
            Some("6.8.0-111")
        );
        assert_eq!(versioned_abi("linux-image-generic"), None);
        assert_eq!(versioned_abi("linux-firmware"), None);
        assert_eq!(versioned_abi("libfoo1"), None);
    }

    #[test]
    fn kernels_kept_by_hand_leave_out_the_running_one_the_two_newest_and_meta_packages() {
        let manual = "firefox\nlinux-generic\nlinux-image-generic\nlinux-image-6.8.0-90-generic\nlinux-image-6.8.0-111-generic\nlinux-image-6.8.0-111-lowlatency\nlinux-modules-6.8.0-111-generic\nlinux-image-6.8.0-149-lowlatency\nlinux-image-6.8.0-152-generic\nlinux-headers-6.8.0-111\n";
        let installed: Vec<String> = [
            "linux-image-6.8.0-90-generic",
            "linux-image-6.8.0-111-generic",
            "linux-image-6.8.0-111-lowlatency",
            "linux-image-6.8.0-149-lowlatency",
            "linux-image-6.8.0-152-generic",
            "linux-image-6.8.0-153-generic",
            "linux-image-generic",
        ]
        .map(String::from)
        .to_vec();
        let (packages, count) = by_hand(manual, "6.8.0-90", &installed);
        assert_eq!(
            packages,
            [
                "linux-image-6.8.0-111-generic",
                "linux-image-6.8.0-111-lowlatency",
                "linux-modules-6.8.0-111-generic",
                "linux-image-6.8.0-149-lowlatency",
                "linux-headers-6.8.0-111"
            ]
        );
        assert_eq!(count, 3);
        assert_eq!(by_hand("", "6.8.0-149", &installed).1, 0);
        assert_eq!(
            super::installed("not-installed\tlinux-image-6.17.0-1008-nvidia\ninstalled\tlinux-image-6.8.0-149-lowlatency\n"),
            ["linux-image-6.8.0-149-lowlatency"]
        );
        assert!(version("6.8.0-152") > version("6.8.0-149"));
        assert!(version("6.10.0-1") > version("6.8.0-152"));
    }

    #[test]
    fn dpkg_sizes_are_kibibytes() {
        let sizes = sizes("linux-image-6.8.0-111-generic\t14000\nbroken\tx\n");
        assert_eq!(
            sizes.get("linux-image-6.8.0-111-generic"),
            Some(&(14000 * 1024))
        );
        assert_eq!(sizes.len(), 1);
    }

    #[test]
    fn disabled_snap_revisions_are_the_ones_kept_for_going_back() {
        let listing = "Name    Version   Rev    Tracking       Publisher   Notes\n\
                       core20  20230801  2015   latest/stable  canonical*  base,disabled\n\
                       core20  20240111  2182   latest/stable  canonical*  base\n\
                       firefox 120.0     3504   latest/stable  mozilla*    disabled\n";
        assert_eq!(disabled_snaps(listing), ["core20_2015", "firefox_3504"]);
        assert!(disabled_snaps("").is_empty());
    }
}
