mod config;
mod discovery;
mod linking;

pub use config::DevConfig;
pub use discovery::discover_plugins;
pub use linking::{create_link, list_linked_plugins, remove_link, LinkedPlugin, LinkRequest};
