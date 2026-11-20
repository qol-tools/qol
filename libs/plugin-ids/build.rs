use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

fn main() {
    let root = PathBuf::from(build_env("CARGO_MANIFEST_DIR")).join("../..");
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
    let modules: String = plugin_folders(&root.join("plugins"))
        .iter()
        .map(|folder| plugin_module(folder))
        .collect();
    let generated = PathBuf::from(build_env("OUT_DIR")).join("plugins.rs");
    fs::write(&generated, modules)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", generated.display()));
}

fn build_env(name: &str) -> OsString {
    std::env::var_os(name).unwrap_or_else(|| panic!("{name} is set in build scripts"))
}

fn plugin_folders(plugins: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(plugins)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", plugins.display()));
    let mut folders: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", plugins.display()))
                .path()
        })
        .filter(|folder| folder.join("plugin.toml").is_file())
        .collect();
    folders.sort();
    folders
}

fn plugin_module(folder: &Path) -> String {
    let manifest = folder.join("plugin.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let contents = fs::read_to_string(&manifest)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifest.display()));
    let table: Table = contents
        .parse()
        .unwrap_or_else(|error| panic!("cannot parse {}: {error}", manifest.display()));
    let plugin = table.get("plugin");
    let daemon = table.get("daemon");
    let id = text(plugin, "id")
        .unwrap_or_else(|| panic!("no `id` under the [plugin] table in {}", manifest.display()));
    let mut constants = vec![text_constant("ID", id)];
    if let Some(uid) = text(plugin, "uid") {
        constants.push(text_constant("UID", uid));
    }
    if let Some(socket) = text(daemon, "socket") {
        constants.push(text_constant("SOCKET_NAME", socket_name(socket, &manifest)));
    }
    if let Some(port) = daemon
        .and_then(|daemon| daemon.get("port"))
        .and_then(Value::as_integer)
    {
        constants.push(format!(
            "pub const PORT: u16 = {};",
            port_number(port, &manifest)
        ));
    }
    format!(
        "pub mod {} {{\n    {}\n}}\n",
        module_name(folder),
        constants.join("\n    ")
    )
}

fn text<'a>(table: Option<&'a Value>, key: &str) -> Option<&'a str> {
    table?.get(key)?.as_str()
}

fn text_constant(name: &str, value: &str) -> String {
    format!("pub const {name}: &str = {value:?};")
}

fn socket_name<'a>(socket: &'a str, manifest: &Path) -> &'a str {
    Path::new(socket)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_else(|| panic!("[daemon].socket in {} names no file", manifest.display()))
}

fn port_number(port: i64, manifest: &Path) -> u16 {
    u16::try_from(port)
        .unwrap_or_else(|_| panic!("[daemon].port in {} is not a port", manifest.display()))
}

fn module_name(folder: &Path) -> String {
    folder
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_else(|| panic!("{} has no UTF-8 folder name", folder.display()))
        .replace('-', "_")
}
