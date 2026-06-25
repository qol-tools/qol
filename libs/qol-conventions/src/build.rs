//! Build-script helpers that make `plugin.toml` the sole source of a plugin's
//! id and daemon port. A plugin's `build.rs` calls [`emit_plugin_id`] and, when
//! it opens a TCP port, [`emit_daemon_port`]; the plugin then reads the values
//! with `env!("QOL_PLUGIN_ID")` / `env!("QOL_DAEMON_PORT")`, so neither can be
//! hand-typed (and thus drift) anywhere in Rust or Python.

use std::path::Path;

pub fn emit_plugin_id() {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set in build scripts");
    let toml_path = Path::new(&manifest_dir).join("plugin.toml");
    println!("cargo:rerun-if-changed={}", toml_path.display());

    let contents = std::fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", toml_path.display()));
    let id = plugin_id(&contents).unwrap_or_else(|| {
        panic!(
            "no `id` under the [plugin] table in {}",
            toml_path.display()
        )
    });

    println!("cargo:rustc-env=QOL_PLUGIN_ID={id}");
}

pub fn emit_daemon_port() {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set in build scripts");
    let toml_path = Path::new(&manifest_dir).join("plugin.toml");
    println!("cargo:rerun-if-changed={}", toml_path.display());

    let contents = std::fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", toml_path.display()));
    let port = daemon_port(&contents).unwrap_or_else(|| {
        panic!(
            "no `port` under the [daemon] table in {}",
            toml_path.display()
        )
    });

    println!("cargo:rustc-env=QOL_DAEMON_PORT={port}");
}

fn daemon_port(contents: &str) -> Option<u16> {
    let mut in_daemon_table = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(section) = section_name(trimmed) {
            in_daemon_table = section == "daemon";
            continue;
        }
        if in_daemon_table {
            if let Some(value) = port_value(trimmed) {
                return Some(value);
            }
        }
    }
    None
}

fn port_value(line: &str) -> Option<u16> {
    let rest = line.strip_prefix("port")?;
    let rest = rest.trim_start().strip_prefix('=')?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn plugin_id(contents: &str) -> Option<String> {
    let mut in_plugin_table = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(section) = section_name(trimmed) {
            in_plugin_table = section == "plugin";
            continue;
        }
        if in_plugin_table {
            if let Some(value) = id_value(trimmed) {
                return Some(value);
            }
        }
    }
    None
}

fn section_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let rest = rest.strip_prefix('[').unwrap_or(rest);
    let end = rest.find(']')?;
    Some(rest[..end].trim())
}

fn id_value(line: &str) -> Option<String> {
    let rest = line.strip_prefix("id")?;
    let rest = rest.trim_start().strip_prefix('=')?;
    let start = rest.find('"')?;
    let after = &rest[start + 1..];
    let end = after.find('"')?;
    let value = &after[..end];
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{daemon_port, plugin_id};

    #[test]
    fn reads_port_from_the_daemon_table() {
        let cases = [
            ("[daemon]\nport = 42710\n", Some(42710)),
            (
                "[plugin]\nport = 1\n[daemon]\nenabled = true\nport = 42720\n",
                Some(42720),
            ),
            ("[daemon]\nport = 42700 # trailing\n", Some(42700)),
            ("[daemon]\nenabled = true\n", None),
            ("[plugin]\nport = 9999\n", None),
            ("port = 8080\n", None),
        ];
        for (input, expected) in cases {
            assert_eq!(daemon_port(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn reads_id_from_the_plugin_table() {
        let cases = [
            ("[plugin]\nid = \"plugin-foo\"\n", Some("plugin-foo")),
            (
                "[plugin]\nid = \"bar\"\nuid = \"x\"\n[[action]]\nid = \"open\"\n",
                Some("bar"),
            ),
            ("[plugin]\nid = \"baz\" # trailing\n", Some("baz")),
            ("[plugin]\n# uid = \"mint-me\"\nid = \"qux\"\n", Some("qux")),
            ("[meta]\nid = \"not-plugin\"\n", None),
            ("[plugin]\nname = \"no-id\"\n", None),
            ("id = \"top-level-ignored\"\n", None),
        ];
        for (input, expected) in cases {
            assert_eq!(plugin_id(input).as_deref(), expected, "input: {input:?}");
        }
    }
}
