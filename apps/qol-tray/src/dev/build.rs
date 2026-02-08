use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
}

pub fn build_linked_plugins(plugins_dir: &Path) -> Vec<BuildResult> {
    let mut results = Vec::new();

    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return results;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let plugin_id = entry.file_name().to_string_lossy().to_string();

        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };

        if !metadata.file_type().is_symlink() {
            continue;
        }

        let target = match std::fs::read_link(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if !target.join("Makefile").exists() {
            continue;
        }

        let result = build_plugin(&plugin_id, &target);
        results.push(result);
    }

    results
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
