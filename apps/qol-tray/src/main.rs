mod app;
mod surfaces;

qol_conventions::declare_build_identity!(Host);

fn main() -> anyhow::Result<()> {
    register_build_identity();
    app::run()
}
