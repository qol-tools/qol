use super::*;
use std::fs;

struct Workspace(tempfile::TempDir);

impl Workspace {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fs::write(
            fixture.root().join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        fixture
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn package(&self, directory: &str, name: &str, configuration: &str) {
        let directory = self.root().join("packages").join(directory);
        fs::create_dir_all(directory.join("src")).unwrap();
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2021\"\n{configuration}\n"
            ),
        )
        .unwrap();
        fs::write(directory.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    fn lock(&self) {
        let output = Command::new("cargo")
            .current_dir(self.root())
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn args(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn names_follow_cargo_and_plugin_manifests_after_renaming() {
    let fixture = Workspace::new();
    for (package, binary, plugin) in [
        ("qol-original", "original-bin", "plugin-original-id"),
        ("qol-renamed", "renamed-bin", "plugin-renamed-id"),
    ] {
        fixture.package(
            "source-folder",
            package,
            &format!("[[bin]]\nname = {binary:?}\npath = \"src/main.rs\""),
        );
        fs::write(
            fixture.root().join("packages/source-folder/plugin.toml"),
            format!("[plugin]\nid = {plugin:?}\nname = \"Example\"\n"),
        )
        .unwrap();
        fixture.lock();
        for name in [
            package,
            binary,
            plugin,
            "source-folder",
            package.strip_prefix("qol-").unwrap(),
            plugin.strip_prefix("plugin-").unwrap(),
        ] {
            let selection = selection::resolve(fixture.root(), name).unwrap();
            assert_eq!(selection.package, package, "{name}");
            assert_eq!(selection.binary.as_deref(), Some(binary), "{name}");
        }
    }
    assert!(selection::resolve(fixture.root(), "qol-original").is_err());
    assert!(selection::resolve(fixture.root(), "original-bin").is_err());
}

#[test]
fn explicit_binaries_override_default_run_and_library_packages_remain_buildable() {
    let fixture = Workspace::new();
    fixture.package("app", "application", "default-run = \"primary\"");
    let src = fixture.root().join("packages/app/src");
    fs::create_dir_all(src.join("bin")).unwrap();
    for binary in ["primary", "qol-secondary", "plugin-secondary"] {
        fs::write(src.join("bin").join(format!("{binary}.rs")), "fn main() {}").unwrap();
    }
    fixture.package("library", "library", "");
    let library = fixture.root().join("packages/library/src");
    fs::rename(library.join("main.rs"), library.join("lib.rs")).unwrap();
    fixture.lock();
    for (name, binary) in [
        ("app", Some("primary")),
        ("application", Some("application")),
        ("qol-secondary", Some("qol-secondary")),
        ("library", None),
    ] {
        let selection = selection::resolve(fixture.root(), name).unwrap();
        assert_eq!(selection.binary.as_deref(), binary, "{name}");
    }
    let error = selection::resolve(fixture.root(), "secondary")
        .err()
        .unwrap();
    assert!(error.to_string().contains("ambiguous binary"));
    fs::remove_file(src.join("bin/plugin-secondary.rs")).unwrap();
    let selection = selection::resolve(fixture.root(), "secondary").unwrap();
    assert_eq!(selection.binary.as_deref(), Some("qol-secondary"));
}

#[test]
fn unknown_ambiguous_and_unsupported_targets_fail_before_building() {
    let fixture = Workspace::new();
    for package in ["first", "second"] {
        fixture.package(
            package,
            package,
            "[[bin]]\nname = \"shared\"\npath = \"src/main.rs\"",
        );
    }
    fixture.lock();
    for (name, excluded, expected) in [
        ("missing", vec![], "no workspace package or binary"),
        ("shared", vec![], "ambiguous build target"),
        (
            "first",
            vec!["first".to_string()],
            "does not support this host platform",
        ),
    ] {
        let error = build_command(fixture.root(), Some(name), &excluded, &[]).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn workspace_build_keeps_exclusions_and_declared_features() {
    let fixture = Workspace::new();
    let command = build_command(
        fixture.root(),
        None,
        &["unsupported".into()],
        &["first/dev".into(), "second/dev".into()],
    )
    .unwrap();
    assert_eq!(
        args(&command),
        [
            "build",
            "--locked",
            "--workspace",
            "--exclude",
            "unsupported",
            "--features",
            "first/dev",
            "--features",
            "second/dev",
        ]
    );
}

#[test]
fn named_build_skips_broken_package_with_same_binary_and_preserves_own_features() {
    let fixture = Workspace::new();
    for package in ["chosen", "broken"] {
        fixture.package(
            package,
            package,
            "[[bin]]\nname = \"shared\"\npath = \"src/main.rs\"\n[features]\ndev = []",
        );
    }
    fs::write(
        fixture.root().join("packages/chosen/src/main.rs"),
        "#[cfg(not(feature = \"dev\"))]\ncompile_error!(\"dev feature required\");\nfn main() { println!(\"chosen\"); }\n",
    )
    .unwrap();
    fs::write(
        fixture.root().join("packages/broken/src/main.rs"),
        "compile_error!(\"unrelated package must not compile\");\nfn main() {}\n",
    )
    .unwrap();
    fixture.lock();
    let mut command = build_command(
        fixture.root(),
        Some("chosen"),
        &[],
        &["chosen/dev".into(), "broken/dev".into()],
    )
    .unwrap();
    let output = command
        .args(["--offline", "--message-format=json", "--target-dir"])
        .arg(fixture.root().join("target"))
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifacts = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["reason"] == "compiler-artifact")
        .collect::<Vec<_>>();
    assert_eq!(artifacts.len(), 1);
    let executable = artifacts[0]["executable"].as_str().unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "chosen");
}
