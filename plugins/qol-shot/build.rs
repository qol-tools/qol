use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PRELUDE: &str = "src/platform/macos_swift/prelude.swift";
const STATUS_OVERLAY: &str = "src/platform/macos_swift/status_overlay.swift";
const RECORDING_OVERLAY: &str = "src/platform/macos_swift/recording_overlay.swift";
const CLIPBOARD_WRITER: &str = "src/platform/macos_swift/clipboard_writer.swift";
const VIDEO_COMPOSER: &str = "src/platform/macos_swift/video_composer.swift";

fn main() -> Result<(), Box<dyn Error>> {
    qol_conventions::build::emit_plugin_id();

    for path in [
        PRELUDE,
        STATUS_OVERLAY,
        RECORDING_OVERLAY,
        CLIPBOARD_WRITER,
        VIDEO_COMPOSER,
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    let target = env::var("TARGET")?;
    let swift_target = swift_target(&target)?;
    compile_helper(&out_dir, "status-overlay", STATUS_OVERLAY, swift_target)?;
    compile_helper(
        &out_dir,
        "recording-overlay",
        RECORDING_OVERLAY,
        swift_target,
    )?;
    compile_helper(&out_dir, "clipboard-writer", CLIPBOARD_WRITER, swift_target)?;
    compile_helper(&out_dir, "video-composer", VIDEO_COMPOSER, swift_target)?;
    Ok(())
}

fn compile_helper(
    out_dir: &Path,
    name: &str,
    body_path: &str,
    swift_target: &str,
) -> Result<(), Box<dyn Error>> {
    let source = out_dir.join(format!("{name}.swift"));
    let binary = out_dir.join(name);
    let prelude = fs::read_to_string(PRELUDE)?;
    let body = fs::read_to_string(body_path)?;
    fs::write(&source, format!("{prelude}\n{body}"))?;

    let output = Command::new("swiftc")
        .arg("-target")
        .arg(swift_target)
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

fn swift_target(cargo_target: &str) -> Result<&'static str, Box<dyn Error>> {
    match cargo_target {
        "aarch64-apple-darwin" => Ok("arm64-apple-macosx13.0"),
        "x86_64-apple-darwin" => Ok("x86_64-apple-macosx13.0"),
        target => Err(format!("unsupported macOS Swift helper target: {target}").into()),
    }
}
