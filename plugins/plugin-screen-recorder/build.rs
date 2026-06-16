use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PRELUDE: &str = "src/platform/macos_swift/prelude.swift";
const REGION_SELECTOR: &str = "src/platform/macos_swift/region_selector.swift";
const STATUS_OVERLAY: &str = "src/platform/macos_swift/status_overlay.swift";

fn main() -> Result<(), Box<dyn Error>> {
    for path in [PRELUDE, REGION_SELECTOR, STATUS_OVERLAY] {
        println!("cargo:rerun-if-changed={path}");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    compile_helper(&out_dir, "region-selector", REGION_SELECTOR)?;
    compile_helper(&out_dir, "status-overlay", STATUS_OVERLAY)?;
    Ok(())
}

fn compile_helper(out_dir: &Path, name: &str, body_path: &str) -> Result<(), Box<dyn Error>> {
    let source = out_dir.join(format!("{name}.swift"));
    let binary = out_dir.join(name);
    let prelude = fs::read_to_string(PRELUDE)?;
    let body = fs::read_to_string(body_path)?;
    fs::write(&source, format!("{prelude}\n{body}"))?;

    let output = Command::new("swiftc")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "swiftc failed for {name}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
    .into())
}
