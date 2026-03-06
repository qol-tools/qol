use serde::Serialize;
use std::path::Path;

use super::search;
use super::source::{classify_sources, ClassifiedSource, LinkState};

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredPlugin {
    pub id: String,
    pub name: String,
    pub path: String,
    pub already_linked: bool,
    pub installed_not_linked: bool,
}

pub fn discover_plugins(
    config: &crate::dev::DevConfig,
    plugins_dir: &Path,
) -> Vec<DiscoveredPlugin> {
    let config_dir = plugins_dir.parent().unwrap_or(plugins_dir);
    let dev_links = crate::dev::load_dev_links(config_dir);
    let search_paths = config.effective_search_paths();
    let plugin_dirs = search::find_plugin_dirs(&search_paths);
    let link_state = LinkState::new(plugins_dir, &dev_links);

    shape_discovered_plugins(classify_sources(&plugin_dirs, &link_state))
}

fn shape_discovered_plugins(sources: Vec<ClassifiedSource>) -> Vec<DiscoveredPlugin> {
    let mut discovered: Vec<DiscoveredPlugin> = Vec::new();

    for source in sources {
        if source.already_linked {
            continue;
        }
        discovered.push(source.into());
    }

    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    discovered
}

impl From<ClassifiedSource> for DiscoveredPlugin {
    fn from(source: ClassifiedSource) -> Self {
        Self {
            id: source.id,
            name: source.name,
            path: source.path.to_string_lossy().to_string(),
            already_linked: source.already_linked,
            installed_not_linked: source.installed_not_linked,
        }
    }
}
