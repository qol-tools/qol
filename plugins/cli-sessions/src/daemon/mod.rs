pub mod actions;
pub mod closed;
pub mod knocks;
pub mod reconcile;
pub mod tab_titles;

pub(crate) fn plugin_id() -> String {
    qol_config::plugin_id_from_env(crate::PLUGIN_ID)
}

pub fn run(show_on_start: bool) -> anyhow::Result<()> {
    crate::ui::run::run(show_on_start)
}

mod screen_analysis;
