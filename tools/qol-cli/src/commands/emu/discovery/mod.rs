use anyhow::Result;
use std::path::PathBuf;

use super::Environment;

mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) image_search_roots: Vec<PathBuf>,
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
    environments.extend(filesystem::discover(&context.image_search_roots));
    Ok(dedupe::dedupe_and_sort(environments))
}
