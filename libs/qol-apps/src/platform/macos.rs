use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn bundle_info(app_path: &Path) -> (Option<String>, Option<String>) {
    let Ok(value) = plist::Value::from_file(app_path.join("Contents/Info.plist")) else {
        return (None, None);
    };
    let Some(dictionary) = value.as_dictionary() else {
        return (None, None);
    };
    let string = |key: &str| {
        dictionary
            .get(key)
            .and_then(|value| value.as_string())
            .map(str::to_string)
    };
    (string("CFBundleIdentifier"), string("CFBundleName"))
}

pub(crate) fn spotlight_app_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut command = Command::new("/usr/bin/mdfind");
    for root in roots {
        command.arg("-onlyin").arg(root);
    }
    command.arg("kMDItemContentType == 'com.apple.application-bundle'");
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_info_reads_identifier_and_name() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Foo.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo</string>
<key>CFBundleName</key><string>Foo</string>
</dict></plist>"#,
        )
        .unwrap();

        assert_eq!(
            bundle_info(&bundle),
            (Some("com.acme.foo".to_string()), Some("Foo".to_string()))
        );
    }
}
