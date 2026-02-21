use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    dev_links
        .iter()
        .filter(|(_, path)| path.join("Cargo.toml").exists())
        .map(|(id, path)| build_cargo_plugin(id, path))
        .collect()
}

pub fn build_qol_tray_self_with_progress<F>(mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = repo_root.join("Cargo.toml");

    if !manifest_path.is_file() {
        return BuildResult {
            plugin_id: "qol-tray".to_string(),
            success: false,
            output: format!("Cargo.toml not found at {}", manifest_path.display()),
        };
    }

    log::info!("Building qol-tray from {}", repo_root.display());
    on_progress(2, "Preparing build".to_string());

    let mut child = match Command::new("cargo")
        .args([
            "build",
            "--bin",
            "qol-tray",
            "--features",
            "dev",
            "--message-format",
            "json",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .current_dir(&repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: format!("Failed to run cargo build: {}", e),
            }
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (artifact_tx, artifact_rx) = std::sync::mpsc::channel::<(u32, String)>();
    let stdout_handle = std::thread::spawn(move || {
        let mut done = 0u32;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                if msg["reason"].as_str() == Some("compiler-artifact") {
                    done += 1;
                    let name = msg["target"]["name"]
                        .as_str()
                        .unwrap_or("crate")
                        .to_string();
                    let _ = artifact_tx.send((done, name));
                }
            }
        }
        done
    });

    let (text_tx, text_rx) = std::sync::mpsc::channel::<String>();
    let stderr_handle = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = text_tx.send(line);
        }
    });

    let last_count = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    let mut predicted = if last_count == 0 { 50u32 } else { last_count };
    let mut last_percent = 2u8;

    for (done, name) in artifact_rx {
        if done > predicted {
            predicted = done + done / 4 + 1;
        }
        let percent = ((done as f32 / predicted as f32) * 93.0) as u8 + 2;
        let percent = percent.min(95);
        if percent > last_percent {
            on_progress(percent, format!("Compiling {}", name));
            last_percent = percent;
        }
    }

    let actual_done = stdout_handle.join().unwrap_or(0);
    let _ = stderr_handle.join();
    let combined = text_rx.into_iter().collect::<Vec<_>>().join("\n");

    match child.wait() {
        Ok(status) => {
            let success = status.success();
            if success {
                LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
                on_progress(100, "Build complete".to_string());
                log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
            } else {
                log::error!("qol-tray build failed\n{}", combined);
            }
            BuildResult {
                plugin_id: "qol-tray".to_string(),
                success,
                output: combined,
            }
        }
        Err(e) => {
            let error = format!("Failed while waiting for cargo build: {}", e);
            log::error!("{}", error);
            BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: error,
            }
        }
    }
}

fn build_cargo_plugin(plugin_id: &str, path: &Path) -> BuildResult {
    log::info!("Building linked plugin via cargo: {}", plugin_id);

    let output = Command::new("cargo")
        .args(["build"])
        .current_dir(path)
        .output();

    match output {
        Ok(out) => {
            let success = out.status.success();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{}{}", stdout, stderr);

            if success {
                log::info!("Cargo build succeeded for {}", plugin_id);
            } else {
                log::error!("Cargo build failed for {}:\n{}", plugin_id, combined);
            }

            BuildResult {
                plugin_id: plugin_id.to_string(),
                success,
                output: combined,
            }
        }
        Err(e) => {
            let error = format!("Failed to run cargo build: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);

            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
            }
        }
    }
}
