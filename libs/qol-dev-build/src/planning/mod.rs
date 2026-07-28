pub(crate) mod queue;
mod rebuild_reason;
mod selection;
pub mod worktree;

#[cfg(test)]
mod planning_tests;

use std::collections::HashMap;
use std::path::PathBuf;

use super::types::PluginBuildPlan;

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    worktree_branch: Option<&str>,
) -> Vec<PluginBuildPlan> {
    let effective_links = worktree::resolve_worktree_paths(dev_links, worktree_branch);
    selection::select_linked_plugins(&effective_links)
        .into_iter()
        .map(|selection| rebuild_reason::plan_selection(selection, known_fingerprints))
        .collect()
}
