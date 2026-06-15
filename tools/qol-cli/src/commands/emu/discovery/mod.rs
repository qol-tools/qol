use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;

mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

#[allow(unused_imports)]
pub(crate) use super::arch::Firmware;
pub(crate) use candidate::{Discovered, ImageCandidate};
pub(crate) use config::{parse_emu_dir, parse_image_overrides};
#[allow(unused_imports)]
pub(crate) use filesystem::infer_candidate;
pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) emu_dir: PathBuf,
}

pub(crate) fn discover(context: DiscoveryContext) -> Result<Discovered> {
    let mut environments = Vec::new();
    environments.extend(config::discover(
        context.config_path.as_deref(),
        context.home_dir.as_ref(),
    )?);
    environments.extend(libvirt::discover(
        context.virsh.as_deref(),
        context.libvirt_uris,
    ));
    let environments = dedupe::dedupe_and_sort(environments);
    let mut seen = HashSet::new();
    let entries =
        filesystem::collect_image_paths(std::slice::from_ref(&context.emu_dir), &mut seen);
    Ok(Discovered::partition(environments, &entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discover_partitions_emu_dir_images_into_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        let registered_file = dir.join("registered.qcow2");
        fs::write(&registered_file, b"x").unwrap();
        fs::write(dir.join("win11.qcow2"), b"x").unwrap();

        let config = dir.join("emu.toml");
        fs::write(
            &config,
            format!("[images]\nregistered = \"{}\"\n", registered_file.display()),
        )
        .unwrap();

        let discovered = discover(DiscoveryContext {
            config_path: Some(config),
            home_dir: None,
            virsh: None,
            libvirt_uris: &[],
            emu_dir: dir.clone(),
        })
        .unwrap();

        assert_eq!(
            discovered.environments.len(),
            1,
            "envs: {:?}",
            discovered.environments
        );
        assert_eq!(discovered.environments[0].id, "registered");
        assert_eq!(
            discovered.candidates.len(),
            1,
            "candidates: {:?}",
            discovered.candidates
        );
        assert!(discovered.candidates[0].path.ends_with("win11.qcow2"));
    }
}
