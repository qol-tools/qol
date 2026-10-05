mod catalog;
mod document;
mod fetch;
#[cfg(test)]
mod install_tests;
mod registry;
mod stage;

pub(crate) use catalog::list_plugins;
pub(crate) use document::IndexedRelease;
pub(crate) use registry::download_asset;
pub(crate) use stage::{latest_version, load_config_contract, stage_release};

const CORE_INDEX_URL: &str = "https://qol-tools.github.io/qol/plugins/index.json";
const CORE_INDEX_PUBLIC_KEY: &str = "RWQXjlL4UxK0KgNZ/nL+lMhWd3Z1bFAu+V+xJyj5WctJS3LKrWzv7SqX";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexLocation {
    pub(crate) url: String,
    pub(crate) public_key: String,
}

pub(crate) fn core_index() -> IndexLocation {
    IndexLocation {
        url: CORE_INDEX_URL.to_string(),
        public_key: CORE_INDEX_PUBLIC_KEY.to_string(),
    }
}
