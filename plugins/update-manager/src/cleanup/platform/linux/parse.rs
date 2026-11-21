use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::platform::linux::packages::{base, kernel_abi, version_key, versioned_abi};

const NEWEST_KEPT: usize = 2;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ByHand {
    pub(super) packages: Vec<String>,
    pub(super) kernels: usize,
}

pub(super) fn removals(simulated: &str) -> Vec<String> {
    simulated
        .lines()
        .filter_map(|line| {
            line.strip_prefix("Remv ")
                .or_else(|| line.strip_prefix("Purg "))
        })
        .filter_map(|rest| rest.split_whitespace().next())
        .map(String::from)
        .collect()
}

pub(super) fn plans_beyond(simulated: &str, listed: &[String]) -> bool {
    let listed: BTreeSet<&str> = listed.iter().map(|package| base(package)).collect();
    simulated.lines().any(|line| line.starts_with("Inst "))
        || removals(simulated)
            .iter()
            .any(|package| !listed.contains(base(package)))
}

pub(super) fn kernels(packages: &[String]) -> usize {
    packages
        .iter()
        .filter_map(|package| versioned_abi(package))
        .collect::<BTreeSet<_>>()
        .len()
}

pub(super) fn by_hand(
    manual: &str,
    running: Option<&str>,
    images: impl FnOnce() -> Result<Vec<String>, String>,
) -> Result<Option<ByHand>, String> {
    let manual: Vec<&str> = manual
        .lines()
        .map(str::trim)
        .filter(|package| versioned_abi(package).is_some())
        .collect();
    if manual.is_empty() {
        return Ok(Some(ByHand::default()));
    }
    let Some(running) = running else {
        return Ok(None);
    };
    let mut newest: Vec<String> = images()?
        .iter()
        .filter_map(|package| versioned_abi(package))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    newest.sort_by_key(|abi| Reverse(version_key(abi)));
    let mut kept: BTreeSet<String> = newest.into_iter().take(NEWEST_KEPT).collect();
    kept.insert(running.to_string());
    let packages: Vec<String> = manual
        .iter()
        .filter(|package| versioned_abi(package).is_some_and(|abi| !kept.contains(&abi)))
        .map(|package| package.to_string())
        .collect();
    Ok(Some(ByHand {
        kernels: kernels(&packages),
        packages,
    }))
}

pub(super) fn kept(manual: &str, running: Option<&str>) -> usize {
    manual
        .lines()
        .filter_map(|package| kernel_abi(package.trim()))
        .filter(|abi| Some(abi.as_str()) != running)
        .count()
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
    let rows: Vec<(&str, &str, bool)> = listing
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let notes = fields.last()?;
            (fields.len() >= 4).then(|| {
                (
                    fields[0],
                    fields[2],
                    notes.split(',').any(|note| note == "disabled"),
                )
            })
        })
        .collect();
    rows.iter()
        .filter(|(name, _, disabled)| {
            *disabled && rows.iter().any(|(other, _, off)| other == name && !off)
        })
        .map(|(name, revision, _)| format!("{name}_{revision}"))
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
    fn kernels_kept_by_hand_leave_out_the_running_one_the_two_newest_and_meta_packages() {
        let manual = "firefox\nlinux-generic\nlinux-image-generic\nlinux-image-6.8.0-90-generic\nlinux-image-6.8.0-111-generic\nlinux-image-6.8.0-111-lowlatency\nlinux-modules-6.8.0-111-generic\nlinux-image-6.8.0-149-lowlatency\nlinux-image-6.8.0-152-generic\nlinux-headers-6.8.0-111\nlinux-lowlatency-headers-6.8.0-100\n";
        let images = || {
            Ok([
                "linux-image-6.8.0-90-generic",
                "linux-image-6.8.0-111-generic",
                "linux-image-6.8.0-111-lowlatency",
                "linux-image-6.8.0-149-lowlatency",
                "linux-image-6.8.0-152-generic",
                "linux-image-6.8.0-153-generic",
                "linux-image-generic",
            ]
            .map(String::from)
            .to_vec())
        };
        let by_hand = by_hand(manual, Some("6.8.0-90"), images)
            .expect("listed")
            .expect("a running kernel");
        assert_eq!(
            by_hand.packages,
            [
                "linux-image-6.8.0-111-generic",
                "linux-image-6.8.0-111-lowlatency",
                "linux-modules-6.8.0-111-generic",
                "linux-image-6.8.0-149-lowlatency",
                "linux-headers-6.8.0-111",
                "linux-lowlatency-headers-6.8.0-100"
            ]
        );
        assert_eq!(by_hand.kernels, 3, "6.8.0-100, 6.8.0-111 and 6.8.0-149");
        assert_eq!(
            kept(manual, Some("6.8.0-90")),
            4,
            "every manual image but the running one"
        );
        assert_eq!(
            kept(manual, None),
            5,
            "an unknown running kernel keeps every image counted"
        );
        assert_eq!(kept("firefox\n", Some("6.8.0-90")), 0);
    }

    #[test]
    fn the_two_newest_guard_reads_unsigned_kernels() {
        let manual = "linux-image-6.8.0-150-generic\nlinux-image-unsigned-6.8.0-152-generic\n";
        let images = || {
            Ok(vec![
                "linux-image-6.8.0-149-lowlatency".to_string(),
                "linux-image-6.8.0-150-generic".to_string(),
                "linux-image-unsigned-6.8.0-152-generic".to_string(),
            ])
        };
        let by_hand = by_hand(manual, Some("6.8.0-149"), images)
            .expect("listed")
            .expect("a running kernel");
        assert!(by_hand.packages.is_empty(), "{:?}", by_hand.packages);
        assert_eq!(kept(manual, Some("6.8.0-149")), 2);
    }

    #[test]
    fn hand_kernels_list_nothing_when_the_running_kernel_or_the_images_are_unknown() {
        let manual = "linux-image-6.8.0-111-generic\n";
        let unread = || -> Result<Vec<String>, String> { panic!("no versioned kernel, no read") };
        assert_eq!(
            by_hand("firefox\n", None, unread),
            Ok(Some(ByHand::default()))
        );
        assert_eq!(by_hand(manual, None, || Ok(Vec::new())), Ok(None));
        assert_eq!(
            by_hand(manual, Some("6.8.0-149"), || Err(
                "database is locked".into()
            )),
            Err("database is locked".to_string())
        );
    }

    #[test]
    fn a_purge_that_takes_or_installs_packages_beyond_the_list_is_caught() {
        let listed = vec!["linux-image-6.8.0-111-generic:amd64".to_string()];
        let alone = "Purg linux-image-6.8.0-111-generic [6.8.0-111.111]\n";
        assert_eq!(removals(alone), ["linux-image-6.8.0-111-generic"]);
        assert!(!plans_beyond(alone, &listed));
        assert!(plans_beyond(
            &format!("{alone}Purg linux-generic [6.8.0.111]\n"),
            &listed
        ));
        assert!(
            plans_beyond(
                &format!(
                    "Inst linux-image-unsigned-6.8.0-111-generic (6.8.0-111.111 Ubuntu:24.04/noble-updates [amd64])\n{alone}"
                ),
                &listed
            ),
            "a purge that swaps in the unsigned image to keep a dependency is caught"
        );
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
                       firefox 120.0     3504   latest/stable  mozilla*    disabled\n\
                       firefox 121.0     3600   latest/stable  mozilla*    -\n\
                       vlc     3.0.20    3777   latest/stable  videolan*   disabled\n";
        assert_eq!(disabled_snaps(listing), ["core20_2015", "firefox_3504"]);
        assert!(disabled_snaps("").is_empty());
    }
}
