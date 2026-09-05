pub mod actions;
pub mod reconcile;

pub fn run(show_on_start: bool) -> anyhow::Result<()> {
    crate::ui::run::run(show_on_start)
}

mod screen_analysis;
