//! Build-script helper that makes `plugin.toml` the sole source of a plugin's
//! id. A plugin's `build.rs` calls [`emit_plugin_id`]; the plugin then reads
//! the id with `env!("QOL_PLUGIN_ID")`, so the id can never be hand-typed (and
//! thus never drift) in Rust.

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
    use super::plugin_id;

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
