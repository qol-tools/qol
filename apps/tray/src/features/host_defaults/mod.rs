mod catalog;
mod claim;
mod cli;
mod held;
pub(crate) mod inspect;
mod intent;
mod lifecycle;
mod open;
mod remote;
mod service;
mod slot;
mod store;

pub use cli::run_cli;
pub use lifecycle::{release_portable_on_exit, release_portable_on_start};
pub(crate) use remote::{apply_change, is_change_action};
pub(crate) use service::{
    row_for_plugin, row_json, toggle_for_plugin, PluginRow, ROW_ACTION, ROW_QUERY,
};
