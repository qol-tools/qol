mod progress;
mod streams;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use super::super::types::BuildResult;
use super::codesign::codesign_debug_binaries;
use super::{parse_cargo_message, spawn_piped, CargoArtifact, CargoChild, CargoMessage};

pub(super) fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let CargoChild {
        mut child,
        stdout,
        stderr,
        process_tree,
    } = match start_build(plugin_id, path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = streams::spawn_output_readers(stdout, stderr);
    let mut progress_driver = progress::ProgressDriver::default();
    let wait_result = super::wait_with_timeout_and_poll_owned(
        plugin_id,
        &mut child,
        &process_tree,
        super::BUILD_TIMEOUT,
        || progress_driver.poll(readers.progress_rx(), &mut on_progress),
    );
    progress_driver.poll(readers.progress_rx(), &mut on_progress);
    let tree_closed = process_tree.tree_has_exited().unwrap_or(false);
    if let Err(message) = &wait_result {
        if !tree_closed {
            log::error!(
                "[dev-build] event=reader_join plugin_id={} joined=false process_tree_closed=false reason=cleanup_failed",
                plugin_id
            );
            drop(readers);
            return super::failed_build(plugin_id, message.clone());
        }
    }
    let join_started = std::time::Instant::now();
    let combined = readers.join_output();
    log::debug!(
        "[dev-build] event=reader_join plugin_id={} joined=true process_tree_closed={} elapsed_ms={}",
        plugin_id,
        tree_closed,
        join_started.elapsed().as_millis()
    );
    super::finish_build_after_wait(
        plugin_id,
        wait_result,
        combined,
        &mut on_progress,
        |output, progress| success_build(plugin_id, path, output, progress),
    )
}

fn start_build<F>(
    plugin_id: &str,
    path: &Path,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(0, "Preparing build".to_string());
    spawn_build(path).map_err(|error| failed_spawn(plugin_id, error))
}

fn spawn_build(path: &Path) -> Result<CargoChild, std::io::Error> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(path);
    crate::configure_dev_cargo(&mut command);
    spawn_piped(command)
}

pub(super) fn build_cargo_plugins_with_progress(
    plugins: &[(&str, &Path)],
    on_progress: &mut dyn FnMut(&str, u8, String),
) -> Vec<BuildResult> {
    let groups = group_plugins(plugins);
    let grouped_results = run_groups_parallel(&groups, on_progress, build_plugin_group);
    let results = grouped_results.into_iter().flatten().collect();
    restore_plugin_order(plugins, results)
}

struct PluginGroup<'a> {
    root: PathBuf,
    plugins: Vec<(&'a str, &'a Path)>,
}

enum GroupMessage {
    Progress {
        plugin_id: String,
        percent: u8,
        phase: String,
    },
    Done {
        group_index: usize,
        results: Vec<BuildResult>,
    },
}

fn group_plugins<'a>(plugins: &'a [(&'a str, &'a Path)]) -> Vec<PluginGroup<'a>> {
    let mut groups: Vec<PluginGroup<'a>> = Vec::new();
    for &(plugin_id, path) in plugins {
        let root = qol_workspace::workspace_root_from(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(group) = groups.iter_mut().find(|group| group.root == root) else {
            groups.push(PluginGroup {
                root,
                plugins: vec![(plugin_id, path)],
            });
            continue;
        };
        group.plugins.push((plugin_id, path));
    }
    groups
}

fn build_plugin_group(
    group: &PluginGroup<'_>,
    on_progress: &mut dyn FnMut(&str, u8, String),
) -> Vec<BuildResult> {
    if group.plugins.len() == 1 {
        let (plugin_id, path) = group.plugins[0];
        let mut emit = |percent, phase| on_progress(plugin_id, percent, phase);
        return vec![build_cargo_plugin_with_progress(plugin_id, path, &mut emit)];
    }
    build_cargo_plugin_batch_with_progress(&group.root, &group.plugins, on_progress)
}

fn run_groups_parallel<'a, F>(
    groups: &'a [PluginGroup<'a>],
    on_progress: &mut dyn FnMut(&str, u8, String),
    build_group: F,
) -> Vec<Vec<BuildResult>>
where
    F: Fn(&PluginGroup<'a>, &mut dyn FnMut(&str, u8, String)) -> Vec<BuildResult> + Sync,
{
    if groups.is_empty() {
        return Vec::new();
    }
    let worker_count = groups.len().min(crate::MAX_CONCURRENT_PLUGIN_BUILDS);
    let (job_tx, job_rx) = mpsc::channel::<usize>();
    let (message_tx, message_rx) = mpsc::channel::<GroupMessage>();
    let job_rx = Arc::new(Mutex::new(job_rx));
    let mut grouped_results: Vec<Option<Vec<BuildResult>>> =
        (0..groups.len()).map(|_| None).collect();
    log::debug!(
        "[dev-build] event=group_queue group_count={} worker_limit={}",
        groups.len(),
        worker_count
    );

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let job_rx = Arc::clone(&job_rx);
            let tx = message_tx.clone();
            let build_group = &build_group;
            scope.spawn(move || loop {
                let group_index = match job_rx.lock() {
                    Ok(receiver) => receiver.recv().ok(),
                    Err(_) => None,
                };
                let Some(group_index) = group_index else {
                    return;
                };
                log::debug!(
                    "[dev-build] event=group_admit group_index={} active_limit={}",
                    group_index,
                    crate::MAX_CONCURRENT_PLUGIN_BUILDS
                );
                let mut emit = |plugin_id: &str, percent: u8, phase: String| {
                    let _ = tx.send(GroupMessage::Progress {
                        plugin_id: plugin_id.to_string(),
                        percent,
                        phase,
                    });
                };
                let results = build_group(&groups[group_index], &mut emit);
                let _ = tx.send(GroupMessage::Done {
                    group_index,
                    results,
                });
            });
        }
        for group_index in 0..groups.len() {
            if job_tx.send(group_index).is_err() {
                break;
            }
        }
        drop(job_tx);
        drop(message_tx);

        for message in message_rx {
            match message {
                GroupMessage::Progress {
                    plugin_id,
                    percent,
                    phase,
                } => on_progress(&plugin_id, percent, phase),
                GroupMessage::Done {
                    group_index,
                    results,
                } => grouped_results[group_index] = Some(results),
            }
        }
    });

    grouped_results
        .into_iter()
        .map(|results| results.unwrap_or_default())
        .collect()
}

fn build_cargo_plugin_batch_with_progress(
    root: &Path,
    plugins: &[(&str, &Path)],
    on_progress: &mut dyn FnMut(&str, u8, String),
) -> Vec<BuildResult> {
    let plugin_ids: Vec<&str> = plugins.iter().map(|(plugin_id, _)| *plugin_id).collect();
    for plugin_id in &plugin_ids {
        on_progress(plugin_id, 0, "Preparing build".to_string());
    }
    let package_names = plugins
        .iter()
        .map(|(_, path)| qol_workspace::cargo_package_name(path).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>();
    let package_names = match package_names {
        Ok(package_names) => package_names,
        Err(error) => return failed_batch(plugins, error),
    };
    let label = plugin_ids.join(",");
    let started = Instant::now();
    let wrapper_mode = match std::env::var_os("QOL_DEV_RUSTC_WRAPPER") {
        Some(_) => "custom",
        None => "disabled",
    };
    log::debug!(
        "[dev-build] event=batch_start plugin_ids={} rustc_wrapper={}",
        label,
        wrapper_mode
    );
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--message-format")
        .arg("json")
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(root);
    for package_name in &package_names {
        command.arg("-p").arg(package_name);
    }
    crate::configure_dev_cargo(&mut command);
    let CargoChild {
        mut child,
        stdout,
        stderr,
        process_tree,
    } = match spawn_piped(command) {
        Ok(child) => child,
        Err(error) => return failed_batch(plugins, error.to_string()),
    };
    let readers = streams::spawn_output_readers(stdout, stderr);
    let mut progress_driver = progress::ProgressDriver::default();
    let mut emit = |percent: u8, phase: String| {
        for plugin_id in &plugin_ids {
            on_progress(plugin_id, percent, phase.clone());
        }
    };
    let wait_result = super::wait_with_timeout_and_poll_owned(
        &format!("batch:{label}"),
        &mut child,
        &process_tree,
        super::BUILD_TIMEOUT,
        || progress_driver.poll(readers.progress_rx(), &mut emit),
    );
    progress_driver.poll(readers.progress_rx(), &mut emit);
    let tree_closed = process_tree.tree_has_exited().unwrap_or(false);
    if let Err(message) = &wait_result {
        if !tree_closed {
            log::error!(
                "[dev-build] event=reader_join plugin_ids={} joined=false process_tree_closed=false reason=cleanup_failed",
                label
            );
            drop(readers);
            return failed_batch(plugins, message.clone());
        }
    }
    let combined = readers.join_output();
    let success = matches!(&wait_result, Ok(true));
    log::debug!(
        "[dev-build] event=batch_complete plugin_ids={} success={} elapsed_ms={}",
        label,
        success,
        started.elapsed().as_millis()
    );
    match wait_result {
        Ok(true) => plugins
            .iter()
            .map(|(plugin_id, path)| {
                let mut emit = |percent, phase| on_progress(plugin_id, percent, phase);
                success_build(plugin_id, path, combined.clone(), &mut emit)
            })
            .collect(),
        Ok(false) => classify_batch_failure(plugins, &combined, on_progress),
        Err(error) => failed_batch(plugins, error),
    }
}

fn failed_batch(plugins: &[(&str, &Path)], error: String) -> Vec<BuildResult> {
    plugins
        .iter()
        .map(|(plugin_id, _)| super::failed_build(plugin_id, error.clone()))
        .collect()
}

fn classify_batch_failure(
    plugins: &[(&str, &Path)],
    output: &str,
    on_progress: &mut dyn FnMut(&str, u8, String),
) -> Vec<BuildResult> {
    let plugin_ids = plugins
        .iter()
        .map(|(plugin_id, _)| *plugin_id)
        .collect::<Vec<_>>()
        .join(",");
    log::warn!(
        "[dev-build] event=batch_partial_results plugin_ids={} action=artifact_classification",
        plugin_ids
    );
    let successful_plugins = successful_batch_plugins(plugins, output);
    plugins
        .iter()
        .map(|(plugin_id, path)| {
            let mut emit = |percent, phase| on_progress(plugin_id, percent, phase);
            if successful_plugins.contains(plugin_id) {
                return success_build(plugin_id, path, output.to_string(), &mut emit);
            }
            super::failed_status_build(plugin_id, output.to_string())
        })
        .collect()
}

fn successful_batch_plugins<'a>(
    plugins: &'a [(&'a str, &'a Path)],
    output: &str,
) -> HashSet<&'a str> {
    let manifests = plugins
        .iter()
        .map(|(plugin_id, path)| (*plugin_id, comparable_path(&path.join("Cargo.toml"))))
        .collect::<Vec<_>>();
    output
        .lines()
        .filter_map(|line| {
            let Ok(CargoMessage::Artifact(artifact)) = parse_cargo_message(line) else {
                return None;
            };
            if !is_plugin_artifact(&artifact) {
                return None;
            }
            let manifest = comparable_path(&artifact.manifest_path);
            manifests
                .iter()
                .find(|(_, expected)| *expected == manifest)
                .map(|(plugin_id, _)| *plugin_id)
        })
        .collect()
}

fn is_plugin_artifact(artifact: &CargoArtifact) -> bool {
    !artifact
        .target_kind
        .iter()
        .any(|kind| kind == "custom-build")
        && !artifact.filenames.is_empty()
}

fn comparable_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn restore_plugin_order(plugins: &[(&str, &Path)], results: Vec<BuildResult>) -> Vec<BuildResult> {
    let mut results_by_id: HashMap<String, BuildResult> = results
        .into_iter()
        .map(|result| (result.plugin_id.clone(), result))
        .collect();
    plugins
        .iter()
        .map(|(plugin_id, _)| {
            results_by_id.remove(*plugin_id).unwrap_or_else(|| {
                super::failed_build(plugin_id, "Cargo builder returned no result".to_string())
            })
        })
        .collect()
}

fn success_build<F>(
    plugin_id: &str,
    path: &Path,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    codesign_debug_binaries(plugin_id, path);
    on_progress(100, "Build complete".to_string());
    log::info!("Cargo build succeeded for {}", plugin_id);
    super::finished_build(plugin_id, combined)
}

fn failed_spawn(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed to run cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    super::failed_build(plugin_id, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn groups_plugins_by_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        std::fs::write(first_root.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(second_root.join("Cargo.toml"), "[workspace]\n").unwrap();

        let first_a = first_root.join("plugin-a");
        let first_b = first_root.join("plugin-b");
        let second_a = second_root.join("plugin-c");
        let external = temp.path().join("external-plugin");
        let plugins = [
            ("a", first_a.as_path()),
            ("b", first_b.as_path()),
            ("c", second_a.as_path()),
            ("external", external.as_path()),
        ];

        let groups = group_plugins(&plugins);

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].root, first_root);
        assert_eq!(groups[0].plugins.len(), 2);
        assert_eq!(groups[0].plugins[0].0, "a");
        assert_eq!(groups[0].plugins[1].0, "b");
        assert_eq!(groups[1].root, second_root);
        assert_eq!(groups[1].plugins.len(), 1);
        assert_eq!(groups[1].plugins[0].0, "c");
        assert_eq!(groups[2].root, external);
        assert_eq!(groups[2].plugins.len(), 1);
    }

    #[test]
    fn restores_results_to_input_plugin_order() {
        let paths = [Path::new("plugin-a"), Path::new("plugin-b")];
        let plugins = [("b", paths[1]), ("a", paths[0])];
        let results = vec![
            super::super::finished_build("a", String::new()),
            super::super::finished_build("b", String::new()),
        ];

        let ordered = restore_plugin_order(&plugins, results);

        assert_eq!(ordered[0].plugin_id, "b");
        assert_eq!(ordered[1].plugin_id, "a");
    }

    #[test]
    fn independent_workspace_groups_run_in_parallel() {
        let temp = tempfile::tempdir().unwrap();
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        std::fs::write(first_root.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(second_root.join("Cargo.toml"), "[workspace]\n").unwrap();
        let first_plugin = first_root.join("plugin-a");
        let second_plugin = second_root.join("plugin-b");
        let plugins = [
            ("a", first_plugin.as_path()),
            ("b", second_plugin.as_path()),
        ];
        let groups = group_plugins(&plugins);
        let active = Arc::new((Mutex::new(0usize), Condvar::new()));
        let overlapped = Arc::new(AtomicBool::new(false));
        let active_for_builder = Arc::clone(&active);
        let overlapped_for_builder = Arc::clone(&overlapped);
        let mut progress = |_: &str, _: u8, _: String| {};

        let results = run_groups_parallel(&groups, &mut progress, move |group, _| {
            let (lock, condition) = &*active_for_builder;
            let mut active_count = lock.lock().unwrap();
            *active_count += 1;
            condition.notify_all();
            let deadline = Instant::now() + Duration::from_millis(250);
            while *active_count < 2 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let (next, timeout) = condition.wait_timeout(active_count, remaining).unwrap();
                active_count = next;
                if timeout.timed_out() {
                    break;
                }
            }
            if *active_count >= 2 {
                overlapped_for_builder.store(true, Ordering::SeqCst);
            }
            *active_count -= 1;
            condition.notify_all();
            group
                .plugins
                .iter()
                .map(|(plugin_id, _)| super::super::finished_build(plugin_id, String::new()))
                .collect()
        });

        assert!(overlapped.load(Ordering::SeqCst));
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn recovers_successful_plugins_after_batch_failure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"good\", \"bad\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        let good = root.join("good");
        let bad = root.join("bad");
        for package in [&good, &bad] {
            std::fs::create_dir_all(package.join("src")).unwrap();
        }
        std::fs::write(
            good.join("Cargo.toml"),
            "[package]\nname = \"good\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(good.join("src/lib.rs"), "pub fn good() {}\n").unwrap();
        std::fs::write(
            bad.join("Cargo.toml"),
            "[package]\nname = \"bad\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(bad.join("build.rs"), "fn main() {}\n").unwrap();
        std::fs::write(bad.join("src/lib.rs"), "pub fn bad() -> {}\n").unwrap();
        let plugins = [("good", good.as_path()), ("bad", bad.as_path())];
        let mut progress = |_: &str, _: u8, _: String| {};

        let results = build_cargo_plugins_with_progress(&plugins, &mut progress);

        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(!results[1].success);
    }

    #[test]
    fn builds_workspace_batch_successfully() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"first\", \"second\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        let first = root.join("first");
        let second = root.join("second");
        for (package, name, source) in [
            (&first, "first", "pub fn first() {}\n"),
            (&second, "second", "pub fn second() {}\n"),
        ] {
            std::fs::create_dir_all(package.join("src")).unwrap();
            std::fs::write(
                package.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            )
            .unwrap();
            std::fs::write(package.join("src/lib.rs"), source).unwrap();
        }
        let plugins = [("first", first.as_path()), ("second", second.as_path())];
        let mut progress = |_: &str, _: u8, _: String| {};

        let results = build_cargo_plugins_with_progress(&plugins, &mut progress);

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.success));
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn windows_output_descendant_helper() {
        let Some(marker) = std::env::var_os("QOL_DEV_BUILD_WINDOWS_DESCENDANT") else {
            return;
        };
        let mut descendant = Command::new("cmd");
        descendant.args(["/C", "ping -n 31 127.0.0.1 >NUL"]);
        let mut descendant = descendant.spawn().unwrap();
        std::fs::write(marker, descendant.id().to_string()).unwrap();
        let _ = descendant.wait();
    }

    #[cfg(windows)]
    #[test]
    fn windows_timeout_closes_output_readers_after_tree_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("descendant");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cargo_build::plugin_build::tests::windows_output_descendant_helper",
                "--nocapture",
            ])
            .env("QOL_DEV_BUILD_WINDOWS_DESCENDANT", &marker);
        let CargoChild {
            mut child,
            stdout,
            stderr,
            process_tree,
        } = super::super::spawn_piped(command).unwrap();
        let readers = streams::spawn_output_readers(stdout, stderr);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.is_file(), "descendant pid was not recorded");
        let descendant_pid = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();

        let started = Instant::now();
        let wait_result = super::super::wait_with_timeout_and_poll_owned(
            "windows-reader-timeout-test",
            &mut child,
            &process_tree,
            Duration::from_millis(100),
            || {},
        );
        let cleanup_elapsed = started.elapsed();
        let join_started = Instant::now();
        let _output = readers.join_output();
        let join_elapsed = join_started.elapsed();

        assert!(wait_result.is_err(), "the hanging process must time out");
        assert!(cleanup_elapsed < Duration::from_secs(3));
        assert!(join_elapsed < Duration::from_secs(1));
        assert!(process_tree.tree_has_exited().unwrap());
        assert!(!qol_process::is_pid_alive(descendant_pid));
    }
}
