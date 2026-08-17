mod app;

qol_conventions::declare_build_identity!(Host);

fn main() -> anyhow::Result<()> {
    qol_runtime::probe!("HOST_ENTRY", "phase=start");
    register_build_identity();
    app::run()
}
