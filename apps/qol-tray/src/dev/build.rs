use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use walkdir::WalkDir;

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);
const DEV_BUILD_STATE_FILE: &str = "dev-build-fingerprints.json";
const DEFAULT_PLUGIN_ARTIFACT_COUNT: u32 = 18;

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BuildFingerprintState {
    #[serde(default)]
    fingerprints: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PluginBuildPlan {
    pub plugin_id: String,
    pub path: PathBuf,
    pub has_cargo: bool,
    pub needs_rebuild: bool,
    pub current_fingerprint: Option<String>,
    pub last_built_fingerprint: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct BuildRun {
    pub plans: Vec<PluginBuildPlan>,
    pub results: Vec<BuildResult>,
    pub fingerprints: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PluginBuildProgress {
    pub plugin_id: String,
    pub status: String,
    pub percent: u8,
    pub phase: String,
}

pub fn load_build_fingerprints(config_dir: &Path) -> HashMap<String, String> {
    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return HashMap::new();
    };
    serde_json::from_str::<BuildFingerprintState>(&content)
        .map(|state| state.fingerprints)
        .unwrap_or_default()
}

pub fn save_build_fingerprints(
    config_dir: &Path,
    fingerprints: &HashMap<String, String>,
) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(config_dir) {
        return Err(format!(
            "Failed to create config directory {}: {}",
            config_dir.display(),
            e
        ));
    }

    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let tmp_path = config_dir.join(".dev-build-fingerprints.tmp");
    let state = BuildFingerprintState {
        fingerprints: fingerprints.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize build fingerprints: {}", e))?;

    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write build fingerprint temp file: {}", e))?;
    std::fs::rename(&tmp_path, &state_path)
        .map_err(|e| format!("Failed to finalize build fingerprint file: {}", e))
}

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
) -> Vec<PluginBuildPlan> {
    let mut links: Vec<_> = dev_links.iter().collect();
    links.sort_by(|(a, _), (b, _)| a.cmp(b));

    links
        .into_iter()
        .map(|(plugin_id, path)| {
            let has_cargo = path.join("Cargo.toml").is_file();
            let last_built_fingerprint = known_fingerprints.get(plugin_id).cloned();

            if !has_cargo {
                return PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: false,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: "Cargo.toml missing".to_string(),
                };
            }

            match fingerprint_plugin(path) {
                Ok(current_fingerprint) => {
                    let needs_rebuild = last_built_fingerprint
                        .as_ref()
                        .map(|known| known != &current_fingerprint)
                        .unwrap_or(true);
                    let reason = if needs_rebuild {
                        if last_built_fingerprint.is_some() {
                            "Source changed".to_string()
                        } else {
                            "No successful build recorded".to_string()
                        }
                    } else {
                        "Up to date".to_string()
                    };

                    PluginBuildPlan {
                        plugin_id: plugin_id.clone(),
                        path: path.clone(),
                        has_cargo,
                        needs_rebuild,
                        current_fingerprint: Some(current_fingerprint),
                        last_built_fingerprint,
                        reason,
                    }
                }
                Err(error) => PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: true,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: format!("Fingerprint unavailable: {}", error),
                },
            }
        })
        .collect()
}

pub fn build_linked_plugins_with_progress<F>(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
    mut on_progress: F,
) -> BuildRun
where
    F: FnMut(PluginBuildProgress),
{
    let plans = plan_linked_plugin_builds(dev_links, known_fingerprints);
    let mut fingerprints = known_fingerprints.clone();
    let mut results = Vec::new();

    for plan in &plans {
        on_progress(PluginBuildProgress {
            plugin_id: plan.plugin_id.clone(),
            status: "queued".to_string(),
            percent: 0,
            phase: plan.reason.clone(),
        });
    }

    for plan in &plans {
        if !plan.has_cargo {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "skipped".to_string(),
                percent: 100,
                phase: "Skipped: Cargo.toml missing".to_string(),
            });
            fingerprints.remove(&plan.plugin_id);
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Cargo.toml missing".to_string(),
                skipped: true,
            });
            continue;
        }

        if !plan.needs_rebuild {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "skipped".to_string(),
                percent: 100,
                phase: "Up to date".to_string(),
            });
            results.push(BuildResult {
                plugin_id: plan.plugin_id.clone(),
                success: true,
                output: "Skipped: Up to date".to_string(),
                skipped: true,
            });
            continue;
        }

        on_progress(PluginBuildProgress {
            plugin_id: plan.plugin_id.clone(),
            status: "building".to_string(),
            percent: 3,
            phase: "Starting cargo build".to_string(),
        });

        let result =
            build_cargo_plugin_with_progress(&plan.plugin_id, &plan.path, |percent, phase| {
                on_progress(PluginBuildProgress {
                    plugin_id: plan.plugin_id.clone(),
                    status: "building".to_string(),
                    percent,
                    phase,
                });
            });

        if result.success {
            if let Some(current_fingerprint) = &plan.current_fingerprint {
                fingerprints.insert(plan.plugin_id.clone(), current_fingerprint.clone());
            }
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "success".to_string(),
                percent: 100,
                phase: "Build complete".to_string(),
            });
        } else {
            on_progress(PluginBuildProgress {
                plugin_id: plan.plugin_id.clone(),
                status: "failed".to_string(),
                percent: 100,
                phase: "Build failed".to_string(),
            });
        }

        results.push(result);
    }

    BuildRun {
        plans,
        results,
        fingerprints,
    }
}

pub fn build_linked_plugins(dev_links: &HashMap<String, PathBuf>) -> Vec<BuildResult> {
    build_linked_plugins_with_progress(dev_links, &HashMap::new(), |_| {}).results
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
            skipped: false,
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
                skipped: false,
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
                skipped: false,
            }
        }
        Err(e) => {
            let error = format!("Failed while waiting for cargo build: {}", e);
            log::error!("{}", error);
            BuildResult {
                plugin_id: "qol-tray".to_string(),
                success: false,
                output: error,
                skipped: false,
            }
        }
    }
}

fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(5, "Preparing build".to_string());

    let mut child = match Command::new("cargo")
        .args(["build", "--message-format", "json"])
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let error = format!("Failed to run cargo build: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);
            return BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
                skipped: false,
            };
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let (artifact_tx, artifact_rx) = std::sync::mpsc::channel::<(u32, String)>();
    let (stdout_text_tx, stdout_text_rx) = std::sync::mpsc::channel::<String>();
    let stdout_handle = std::thread::spawn(move || {
        let mut done = 0u32;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                match msg["reason"].as_str() {
                    Some("compiler-artifact") => {
                        done += 1;
                        let name = msg["target"]["name"]
                            .as_str()
                            .unwrap_or("crate")
                            .to_string();
                        let _ = artifact_tx.send((done, name));
                    }
                    Some("compiler-message") => {
                        if let Some(rendered) = msg["message"]["rendered"].as_str() {
                            let _ = stdout_text_tx.send(rendered.trim_end().to_string());
                        }
                    }
                    _ => {}
                }
            } else if !line.trim().is_empty() {
                let _ = stdout_text_tx.send(line);
            }
        }
        done
    });

    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<String>();
    let stderr_handle = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = stderr_tx.send(line);
        }
    });

    let mut predicted = DEFAULT_PLUGIN_ARTIFACT_COUNT;
    let mut last_percent = 5u8;

    for (done, name) in artifact_rx {
        if done > predicted {
            predicted = done + done / 3 + 1;
        }
        let percent = ((done as f32 / predicted as f32) * 85.0) as u8 + 10;
        let percent = percent.min(95);
        if percent > last_percent {
            on_progress(percent, format!("Compiling {}", name));
            last_percent = percent;
        }
    }

    let _ = stdout_handle.join();
    let _ = stderr_handle.join();

    let mut lines: Vec<String> = stdout_text_rx.into_iter().collect();
    lines.extend(stderr_rx.into_iter());
    let combined = lines.join("\n");

    match child.wait() {
        Ok(status) => {
            let success = status.success();
            if success {
                on_progress(100, "Build complete".to_string());
                log::info!("Cargo build succeeded for {}", plugin_id);
            } else {
                log::error!("Cargo build failed for {}:\n{}", plugin_id, combined);
            }
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success,
                output: combined,
                skipped: false,
            }
        }
        Err(e) => {
            let error = format!("Failed while waiting for cargo build: {}", e);
            log::error!("Build error for {}: {}", plugin_id, error);
            BuildResult {
                plugin_id: plugin_id.to_string(),
                success: false,
                output: error,
                skipped: false,
            }
        }
    }
}

fn fingerprint_plugin(path: &Path) -> Result<String, String> {
    let mut hasher = Fnv1a64::default();
    let mut inputs = Vec::new();

    let walker = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !(entry.file_type().is_dir() && should_skip_dir(entry.file_name()))
        });

    for entry in walker {
        let entry = entry.map_err(|e| format!("Walk error: {}", e))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(path)
            .map_err(|e| format!("Failed to relativize path: {}", e))?;

        if !is_fingerprint_input(relative_path) {
            continue;
        }
        inputs.push((relative_path.to_path_buf(), entry.path().to_path_buf()));
    }

    if inputs.is_empty() {
        return Err("No Rust build inputs found".to_string());
    }

    inputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (relative_path, absolute_path) in inputs {
        hasher.write(relative_path.to_string_lossy().as_bytes());
        hasher.write_u8(0);

        let mut file = std::fs::File::open(&absolute_path)
            .map_err(|e| format!("Failed to open {}: {}", absolute_path.display(), e))?;
        let mut buf = [0u8; 8192];
        loop {
            let read = file
                .read(&mut buf)
                .map_err(|e| format!("Failed to read {}: {}", absolute_path.display(), e))?;
            if read == 0 {
                break;
            }
            hasher.write(&buf[..read]);
        }
        hasher.write_u8(0xff);
    }

    Ok(format!("{:016x}", hasher.finish()))
}

fn should_skip_dir(name: &OsStr) -> bool {
    matches!(name.to_str(), Some("target" | ".git" | ".hg" | ".svn"))
}

fn is_fingerprint_input(relative_path: &Path) -> bool {
    let Some(file_name) = relative_path.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    if matches!(
        file_name,
        "Cargo.toml" | "Cargo.lock" | "build.rs" | "rust-toolchain" | "rust-toolchain.toml"
    ) {
        return true;
    }

    if relative_path
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".cargo"))
    {
        return true;
    }

    relative_path
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("rs"))
        .unwrap_or(false)
}

#[derive(Default)]
struct Fnv1a64(u64);

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_basic_plugin(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[test]
    fn plan_marks_new_plugin_for_rebuild() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "No successful build recorded");
    }

    #[test]
    fn plan_marks_unchanged_plugin_as_up_to_date() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir.clone())]);
        let fingerprint = fingerprint_plugin(&plugin_dir).unwrap();
        let known = HashMap::from([("plugin-a".to_string(), fingerprint)]);
        let plans = plan_linked_plugin_builds(&links, &known);

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Up to date");
    }

    #[test]
    fn plan_skips_plugin_without_cargo_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("src.rs"), "fn main() {}\n").unwrap();

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].has_cargo);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Cargo.toml missing");
    }

    #[test]
    fn fingerprint_state_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let data = HashMap::from([("plugin-a".to_string(), "abc".to_string())]);

        save_build_fingerprints(tmp.path(), &data).unwrap();
        let loaded = load_build_fingerprints(tmp.path());

        assert_eq!(loaded, data);
    }
}
