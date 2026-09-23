qol_conventions::declare_build_identity!(ResidentPolicy);

fn main() {
    if qol_process::process_tree_guardian_requested() {
        if let Err(error) = qol_process::run_process_tree_guardian_entry() {
            eprintln!("resident-policy: process-tree guardian entry failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    register_build_identity();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exit = match qol_host_fixes::policy::nvidia::run_resident_policy_cli_traced(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("resident-policy: {error:#}");
            1
        }
    };
    std::process::exit(exit);
}
