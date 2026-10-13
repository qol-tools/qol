use super::super::super::source::{resolve_source_for_plugin, PluginSource};
use super::super::types::AppState;
use crate::updates::jobs::Operation;

mod install;
mod uninstall;
mod update;

pub(in super::super) async fn run_operation(
    state: &AppState,
    id: &str,
    operation: Operation,
) -> Result<(), String> {
    match operation {
        Operation::Install => install::install_plugin(state, id).await,
        Operation::Update => update::update_plugin(state, id).await,
        Operation::Remove => uninstall::uninstall_plugin(state, id).await,
        Operation::Host {
            confirm_after_restart,
            update_plugins,
        } => {
            crate::updates::install_host_update(
                state.daemon.events.clone(),
                state.plugin_manager.clone(),
                confirm_after_restart,
                update_plugins,
            )
            .await
        }
    }
}

fn source_for(id: &str) -> Result<PluginSource, String> {
    resolve_source_for_plugin(id).ok_or_else(|| format!("No plugin source provides {}", id))
}
