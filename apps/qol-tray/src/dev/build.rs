use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    dev_links
        .iter()
        .filter(|(_, path)| path.join("Makefile").exists())
        .map(|(id, path)| build_plugin(id, path))
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

    let child_result = Command::new("cargo")
        .arg("build")
        .arg("--bin")
        .arg("qol-tray")
        .arg("--features")
        .arg("dev")
        .arg("--manifest-path")
        .arg(manifest_path)
        .current_dir(&repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child_result {
        Ok(child) => child,
        Err(e) => {
            let error = format!("Failed to run cargo build: {}", e);
            log::error!("{}", error);
            return BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: error,
            };
        }
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: "Failed to capture cargo stdout".to_string(),
            };
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            return BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: "Failed to capture cargo stderr".to_string(),
            };
        }
    };

    let (tx, rx) = std::sync::mpsc::channel::<String>();

    let stdout_tx = tx.clone();
    let stdout_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = stdout_tx.send(line);
        }
    });

    let stderr_tx = tx.clone();
    let stderr_thread = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = stderr_tx.send(line);
        }
    });

    drop(tx);

    let mut combined = String::new();
    let mut progress = SelfBuildProgress::default();

    for line in rx {
        combined.push_str(&line);
        combined.push('\n');

        if let Some((percent, phase)) = progress.observe(&line) {
            on_progress(percent, phase);
        }
    }

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    match child.wait() {
        Ok(status) => {
            let success = status.success();
            if success {
                if let Some((percent, phase)) = progress.finish_success() {
                    on_progress(percent, phase);
                }
                log::info!("qol-tray build succeeded");
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

#[derive(Default)]
struct SelfBuildProgress {
    percent: u8,
    compile_hits: u32,
}

impl SelfBuildProgress {
    fn observe(&mut self, line: &str) -> Option<(u8, String)> {
        if line.contains("Finished ") {
            return self.update(98, "Finalizing build".to_string());
        }

        if line.contains("Compiling ")
            || line.contains("Checking ")
            || line.contains("Building ")
            || line.contains("Linking ")
        {
            self.compile_hits = self.compile_hits.saturating_add(1);
            let step = self.compile_hits.min(40) as u8;
            let next = 8u8.saturating_add(step.saturating_mul(2));
            return self.update(next, "Compiling crates".to_string());
        }

        None
    }

    fn finish_success(&mut self) -> Option<(u8, String)> {
        self.update(100, "Build complete".to_string())
    }

    fn update(&mut self, percent: u8, phase: String) -> Option<(u8, String)> {
        let capped = percent.min(100);
        if capped <= self.percent {
            return None;
        }
        self.percent = capped;
        Some((capped, phase))
    }
}

fn build_plugin(plugin_id: &str, path: &Path) -> BuildResult {
    log::info!("Building linked plugin: {}", plugin_id);

    let target = select_make_target(path);
    run_make(plugin_id, path, target)
}

fn select_make_target(path: &Path) -> &'static str {
    let makefile_path = path.join("Makefile");
    let Ok(content) = std::fs::read_to_string(makefile_path) else {
        return "dev";
    };

    if has_target(&content, "dev") {
        return "dev";
    }
    if has_target(&content, "build") {
        return "build";
    }
    if has_target(&content, "run") {
        return "run";
    }
    if has_target(&content, "all") {
        return "all";
    }

    "dev"
}

fn has_target(content: &str, target: &str) -> bool {
    let prefix_colon = format!("{}:", target);
    let prefix_space = format!("{} ", target);

    content
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with(&prefix_colon) || line.starts_with(&prefix_space))
}

fn run_make(plugin_id: &str, path: &Path, target: &str) -> BuildResult {
    log::info!("Running make {} for {}", target, plugin_id);

    let output = Command::new("make")
        .arg(target)
        .current_dir(path)
        .output();

    match output {
        Ok(out) => {
            let success = out.status.success();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{}{}", stdout, stderr);

            if success {
                log::info!("Build succeeded: {}", plugin_id);
            } else {
                log::error!("Build failed: {}\n{}", plugin_id, combined);
            }

            BuildResult {
                plugin_id: plugin_id.to_string(),
                success,
                output: combined,
            }
        }
        Err(e) => {
            let error = format!("Failed to run make: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);

            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
            }
        }
    }
}
