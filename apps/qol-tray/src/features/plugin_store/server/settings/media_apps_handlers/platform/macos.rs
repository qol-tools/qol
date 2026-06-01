use serde_json::Value;
use std::path::Path as FsPath;

pub(super) fn discover_installed_apps() -> Vec<Value> {
    let mut apps: Vec<Value> = find_app_paths()
        .iter()
        .filter_map(|path| app_entry(path))
        .collect();
    sort_and_dedup_apps(&mut apps);
    apps
}

fn find_app_paths() -> Vec<String> {
    let Some(output) = mdfind_output() else {
        return Vec::new();
    };
    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn mdfind_output() -> Option<String> {
    use std::process::Command;

    let mut command = Command::new("mdfind");
    command.args(["-onlyin", "/Applications"]);
    command.args(["-onlyin", "/System/Applications"]);
    command.arg("kMDItemContentType == 'com.apple.application-bundle'");
    let output = command.output().ok()?;
    output_string(output)
}

fn app_entry(path: &str) -> Option<Value> {
    let app_path = FsPath::new(path);
    if !app_path.is_dir() {
        return None;
    }
    if has_contents_component(app_path) {
        return None;
    }
    let name = app_path.file_stem()?.to_str()?.to_string();
    let bundle_id = read_bundle_id(path)?;
    Some(serde_json::json!({ "bundle_id": bundle_id, "name": name }))
}

fn has_contents_component(path: &FsPath) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "Contents")
}

fn read_bundle_id(path: &str) -> Option<String> {
    use std::process::Command;

    let mut command = Command::new("defaults");
    command.args([
        "read",
        &format!("{}/Contents/Info", path),
        "CFBundleIdentifier",
    ]);
    let output = command.output().ok()?;
    output_string(output).filter(|id| !id.trim().is_empty())
}

fn output_string(output: std::process::Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sort_and_dedup_apps(apps: &mut Vec<Value>) {
    apps.sort_by_key(app_name);
    apps.dedup_by(|left, right| left["bundle_id"] == right["bundle_id"]);
}

fn app_name(value: &Value) -> String {
    value["name"].as_str().unwrap_or("").to_lowercase()
}
