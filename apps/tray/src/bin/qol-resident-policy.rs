qol_conventions::declare_build_identity!(ResidentPolicy);

fn main() {
    qol_log::init_stderr();
    if qol_process::process_tree_guardian_requested() {
        std::process::exit(qol_host_fixes::policy::cli::run_guardian());
    }
    register_build_identity();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(qol_host_fixes::policy::cli::run_standalone(&args));
}
