use anyhow::Result;
use std::path::PathBuf;

use super::Environment;

mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

#[allow(unused_imports)]
pub(crate) use candidate::{Discovered, ImageCandidate};
pub(crate) use config::parse_emu_dir;
pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) emu_dir: PathBuf,
}

pub(crate) fn discover(context: DiscoveryContext) -> Result<Vec<Environment>> {
    let mut environments = Vec::new();
    environments.extend(config::discover(
        context.config_path.as_deref(),
        context.home_dir.as_ref(),
    )?);
    environments.extend(libvirt::discover(
        context.virsh.as_deref(),
        context.libvirt_uris,
    ));
    environments.extend(filesystem::discover(&context.emu_dir));
    Ok(dedupe::dedupe_and_sort(environments))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn discover_scans_the_single_emu_dir() {
        let dir = std::env::temp_dir().join(format!("qol-emu-ctx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("win11.qcow2"), b"x").unwrap();

        let environments = discover(DiscoveryContext {
            config_path: None,
            home_dir: None,
            virsh: None,
            libvirt_uris: &[],
            emu_dir: dir.clone(),
        })
        .unwrap();

        assert_eq!(environments.len(), 1, "environments: {environments:?}");
        assert_eq!(environments[0].source, "scan");

        let mut registered = HashSet::new();
        registered.insert(environments[0].image_path.clone());
        assert!(registered.iter().any(|p| p.ends_with("win11.qcow2")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
