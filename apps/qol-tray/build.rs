use std::process::Command;

fn main() {
    qol_build_identity::emit_build_identity();
    println!("cargo:rerun-if-changed=../../libs/qol-config/js");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let hash = if profile == "release" {
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        "dev".to_string()
    };

    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);
    if profile == "release" {
        if let Some(head) = Command::new("git")
            .args(["rev-parse", "--git-path", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
        {
            println!("cargo:rerun-if-changed={head}");
        }
    }
}
