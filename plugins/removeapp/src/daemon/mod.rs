pub mod actions;

pub fn run() -> anyhow::Result<()> {
    crate::ui::run::run()
}
