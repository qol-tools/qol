use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use walkdir::WalkDir;

pub(super) type FingerprintInput = (PathBuf, PathBuf);

const EMBED_MACROS: [&str; 2] = ["include_str!", "include_bytes!"];
const EMBEDDED_CONTRACT_FILES: [&str; 3] = ["plugin.toml", "qol-config.toml", "qol-runtime.toml"];

pub(super) fn fingerprint_inputs(path: &Path) -> Result<Vec<FingerprintInput>, String> {
    let mut inputs = Vec::new();
    let mut embedded = BTreeSet::new();
    for entry in walker(path) {
        let entry = entry.map_err(|error| format!("Walk error: {}", error))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative_path = relative_path(path, entry.path())?;
        if !is_fingerprint_input(&relative_path) {
            continue;
        }
        if is_rust_source(&relative_path) {
            embedded.extend(embedded_files(path, &relative_path, entry.path()));
        }
        inputs.push((relative_path, entry.path().to_path_buf()));
    }
    for relative_path in embedded {
        if inputs.iter().any(|(known, _)| *known == relative_path) {
            continue;
        }
        let absolute_path = path.join(&relative_path);
        if absolute_path.is_file() {
            inputs.push((relative_path, absolute_path));
        }
    }
    Ok(inputs)
}

fn embedded_files(root: &Path, relative_source: &Path, source: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(source) else {
        return Vec::new();
    };
    let source_dir = relative_source.parent().unwrap_or(Path::new(""));
    let text = text.as_str();
    EMBED_MACROS
        .into_iter()
        .flat_map(move |name| {
            text.match_indices(name)
                .map(move |(at, matched)| &text[at + matched.len()..])
        })
        .filter_map(|call| embedded_path(call, source_dir))
        .filter(|path| !path.as_os_str().is_empty() && root.join(path).is_file())
        .collect()
}

fn embedded_path(call: &str, source_dir: &Path) -> Option<PathBuf> {
    let arguments = call.trim_start().strip_prefix('(')?;
    if let Some((literal, _)) = string_literal(arguments) {
        return within_crate(&source_dir.join(literal));
    }
    let mut rest = arguments
        .trim_start()
        .strip_prefix("concat!")?
        .trim_start()
        .strip_prefix('(')?
        .trim_start()
        .strip_prefix("env!")?
        .trim_start()
        .strip_prefix('(')?;
    let (variable, after) = string_literal(rest)?;
    if variable != "CARGO_MANIFEST_DIR" {
        return None;
    }
    rest = after.trim_start().strip_prefix(')')?;
    let mut joined = String::new();
    while let Some(next) = rest.trim_start().strip_prefix(',') {
        let Some((literal, after)) = string_literal(next) else {
            break;
        };
        joined.push_str(literal);
        rest = after;
    }
    within_crate(Path::new(joined.trim_start_matches(['/', '\\'])))
}

fn string_literal(text: &str) -> Option<(&str, &str)> {
    let body = text.trim_start().strip_prefix('"')?;
    let end = body.find('"')?;
    Some((&body[..end], &body[end + 1..]))
}

fn within_crate(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn is_rust_source(relative_path: &Path) -> bool {
    relative_path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

fn walker(path: &Path) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(keep_entry)
}

fn keep_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    !(entry.file_type().is_dir() && should_skip_dir(entry.file_name()))
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    path.strip_prefix(root)
        .map(|relative| relative.to_path_buf())
        .map_err(|error| format!("Failed to relativize path: {}", error))
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

    if relative_path.parent() == Some(Path::new("")) && EMBEDDED_CONTRACT_FILES.contains(&file_name)
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn relative_inputs(root: &Path) -> Vec<PathBuf> {
        let mut inputs: Vec<_> = fingerprint_inputs(root)
            .unwrap()
            .into_iter()
            .map(|(relative, _)| relative)
            .collect();
        inputs.sort();
        inputs
    }

    #[test]
    fn embedded_files_and_contract_files_are_inputs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::create_dir_all(root.join("assets/icons")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        for file in [
            "Cargo.toml",
            "plugin.toml",
            "qol-config.toml",
            "README.md",
            "assets/icons/tray.svg",
            "assets/font.ttf",
            "assets/unused.png",
            "docs/plugin.toml",
            "src/ui/shader.wgsl",
        ] {
            fs::write(root.join(file), file).unwrap();
        }
        fs::write(
            root.join("src/lib.rs"),
            "const ICON: &[u8] = include_bytes!(\"../assets/icons/tray.svg\");\nconst FONT: &[u8] = include_bytes!(\n    concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/\", \"font.ttf\"),\n);\nconst ESCAPE: &str = include_str!(\"../../outside.toml\");\nconst MISSING: &str = include_str!(\"missing.txt\");\n",
        )
        .unwrap();
        fs::write(
            root.join("src/ui/mod.rs"),
            "const SHADER: &str = include_str!(\"shader.wgsl\");\n",
        )
        .unwrap();

        assert_eq!(
            relative_inputs(root),
            [
                "Cargo.toml",
                "assets/font.ttf",
                "assets/icons/tray.svg",
                "plugin.toml",
                "qol-config.toml",
                "src/lib.rs",
                "src/ui/mod.rs",
                "src/ui/shader.wgsl",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn embedded_paths_resolve_like_the_compiler() {
        let cases = [
            ("(\"a.txt\")", "src", Some("src/a.txt")),
            ("( \"../x/b.bin\" )", "src/ui", Some("src/x/b.bin")),
            ("(\"../../../up.txt\")", "src", None),
            ("(\"/etc/passwd\")", "src", None),
            (
                "(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/plugin.toml\"))",
                "src",
                Some("plugin.toml"),
            ),
            ("(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"))", "src", None),
            ("(SOME_CONST)", "src", None),
        ];
        for (call, source_dir, expected) in cases {
            assert_eq!(
                embedded_path(call, Path::new(source_dir)),
                expected.map(PathBuf::from),
                "{call}"
            );
        }
    }
}
