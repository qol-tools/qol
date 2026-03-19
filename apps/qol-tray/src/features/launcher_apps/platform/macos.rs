use super::super::LauncherEntry;
use crate::shortcuts::model::{AppRef, ShortcutAction};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(super) fn sync(entries: &[LauncherEntry], binary_path: &Path) -> Result<()> {
    let dir = apps_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create launcher apps dir {}", dir.display()))?;

    let app_names = app_dirnames(entries);
    let expected: HashSet<String> = app_names.values().cloned().collect();

    for entry in entries {
        let Some(app_name) = app_names.get(&entry.file_stem) else {
            continue;
        };
        let app_dir = dir.join(app_name);
        write_app_bundle(&app_dir, entry, binary_path)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

fn apps_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("Applications").join("QoL"))
}

fn app_dirnames(entries: &[LauncherEntry]) -> HashMap<String, String> {
    let sanitized: Vec<String> = entries.iter().map(sanitized_display_name).collect();

    let mut counts = HashMap::new();
    for name in &sanitized {
        *counts.entry(name.as_str()).or_insert(0usize) += 1;
    }

    let mut names = HashMap::new();
    for (entry, base) in entries.iter().zip(&sanitized) {
        let app_name = if counts.get(base.as_str()).copied().unwrap_or(0) <= 1 {
            format!("{}.app", base)
        } else {
            format!("{} ({}).app", base, entry.file_stem)
        };
        names.insert(entry.file_stem.clone(), app_name);
    }
    names
}

fn sanitized_display_name(entry: &LauncherEntry) -> String {
    let mut name = String::with_capacity(entry.display_name.len());
    for ch in entry.display_name.chars() {
        if ch == '/' || ch == ':' || ch.is_control() {
            name.push(' ');
            continue;
        }
        name.push(ch);
    }

    let collapsed = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_matches('.');
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    entry.file_stem.clone()
}

fn write_app_bundle(app_dir: &Path, entry: &LauncherEntry, binary_path: &Path) -> Result<()> {
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let run_path = macos_dir.join("run");
    let plist_path = contents_dir.join("Info.plist");

    let expected_script = build_script(binary_path, entry);
    let expected_plist = build_info_plist(entry);

    if file_matches(&run_path, &expected_script)
        && file_matches(&plist_path, &expected_plist)
        && is_executable(&run_path)
    {
        return Ok(());
    }

    std::fs::create_dir_all(&macos_dir)?;
    std::fs::write(&plist_path, &expected_plist)?;
    write_executable(&run_path, &expected_script)?;
    Ok(())
}

fn file_matches(path: &Path, expected: &str) -> bool {
    path.is_file()
        && std::fs::read_to_string(path)
            .ok()
            .is_some_and(|s| s == expected)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn build_info_plist(entry: &LauncherEntry) -> String {
    let name = xml_escape(&entry.display_name);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>CFBundleExecutable</key><string>run</string>\n\
         <key>CFBundleDisplayName</key><string>{}</string>\n\
         <key>CFBundleName</key><string>{}</string>\n\
         <key>CFBundleIdentifier</key><string>{}</string>\n\
         <key>CFBundlePackageType</key><string>APPL</string>\n\
         <key>CFBundleVersion</key><string>1</string>\n\
         <key>CFBundleShortVersionString</key><string>1</string>\n\
         <key>LSUIElement</key><true/>\n\
         </dict>\n\
         </plist>\n",
        name, name, entry.bundle_id
    )
}

fn write_executable(path: &Path, script: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn build_script(binary_path: &Path, entry: &LauncherEntry) -> String {
    if let Some(script) = build_shortcut_script(entry) {
        return script;
    }
    let bin = shell_escape_single_quote(&binary_path.display().to_string());
    let args: String = entry
        .exec_args
        .iter()
        .map(|a| format!("'{}'", shell_escape_single_quote(a)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("#!/bin/sh\nexec '{}' {}\n", bin, args)
}

fn build_shortcut_script(entry: &LauncherEntry) -> Option<String> {
    let action = entry.shortcut_action.as_ref()?;
    match action {
        ShortcutAction::OpenUrl {
            url,
            browser_override,
        } => Some(build_open_script(open_url_args(
            url,
            browser_override.as_ref(),
        ))),
        ShortcutAction::LaunchApp { app } => Some(build_open_script(open_app_args(app))),
    }
}

fn build_open_script(args: Vec<String>) -> String {
    let args = args
        .iter()
        .map(|arg| format!("'{}'", shell_escape_single_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("#!/bin/sh\nexec /usr/bin/open {}\n", args)
}

fn open_url_args(url: &str, browser_override: Option<&AppRef>) -> Vec<String> {
    let mut args = open_target_args(browser_override);
    args.push(url.to_string());
    args
}

fn open_app_args(app: &AppRef) -> Vec<String> {
    open_target_args(Some(app))
}

fn open_target_args(app: Option<&AppRef>) -> Vec<String> {
    let Some(app) = app else {
        return Vec::new();
    };
    match app {
        AppRef::BundleId { id } => vec!["-b".into(), id.clone()],
        AppRef::Path { path } => vec!["-a".into(), path.clone()],
        AppRef::Name { name } => vec!["-a".into(), name.clone()],
    }
}

fn clean_stale(dir: &Path, expected: &HashSet<String>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !name_str.ends_with(".app") {
            continue;
        }
        if expected.contains(name_str) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    Ok(())
}

fn shell_escape_single_quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::model::{AppRef, ShortcutAction};
    use proptest::prelude::*;

    fn entry(file_stem: &str, display_name: &str) -> LauncherEntry {
        LauncherEntry {
            file_stem: file_stem.to_string(),
            display_name: display_name.to_string(),
            description: String::new(),
            bundle_id: String::new(),
            exec_args: Vec::new(),
            shortcut_action: None,
        }
    }

    fn shortcut_entry(action: ShortcutAction) -> LauncherEntry {
        LauncherEntry {
            file_stem: "shortcut-browser".to_string(),
            display_name: "Open Browser".to_string(),
            description: String::new(),
            bundle_id: "com.qol-tools.shortcut.browser".to_string(),
            exec_args: vec!["exec".into(), "shortcut".into(), "browser".into()],
            shortcut_action: Some(action),
        }
    }

    fn fallback_entry() -> LauncherEntry {
        LauncherEntry {
            file_stem: "shortcut-browser".to_string(),
            display_name: "Open Browser".to_string(),
            description: String::new(),
            bundle_id: "com.qol-tools.shortcut.browser".to_string(),
            exec_args: vec!["exec".into(), "shortcut".into(), "browser".into()],
            shortcut_action: None,
        }
    }

    #[test]
    fn app_dirnames_preserve_friendly_names() {
        let names = app_dirnames(&[entry("shortcut-browser", "Open Browser")]);

        assert_eq!(
            names.get("shortcut-browser"),
            Some(&"Open Browser.app".to_string())
        );
    }

    #[test]
    fn app_dirnames_sanitize_unsafe_display_names() {
        let names = app_dirnames(&[entry("shortcut-docs", "Docs:/Team\nPortal")]);

        assert_eq!(
            names.get("shortcut-docs"),
            Some(&"Docs Team Portal.app".to_string())
        );
    }

    #[test]
    fn app_dirnames_fallback_to_file_stem_when_name_is_empty_after_sanitize() {
        let names = app_dirnames(&[entry("shortcut-empty", "/:\n\r\t")]);

        assert_eq!(
            names.get("shortcut-empty"),
            Some(&"shortcut-empty.app".to_string())
        );
    }

    #[test]
    fn app_dirnames_disambiguate_duplicates() {
        let names = app_dirnames(&[
            entry("shortcut-a", "Open Browser"),
            entry("shortcut-b", "Open Browser"),
        ]);

        assert_eq!(
            names.get("shortcut-a"),
            Some(&"Open Browser (shortcut-a).app".to_string())
        );
        assert_eq!(
            names.get("shortcut-b"),
            Some(&"Open Browser (shortcut-b).app".to_string())
        );
    }

    #[test]
    fn build_script_generates_direct_open_commands_for_shortcuts() {
        let binary = Path::new("/Applications/qol-tray.app/Contents/MacOS/qol-tray");
        let cases = [
            (
                ShortcutAction::OpenUrl {
                    url: "https://example.com".to_string(),
                    browser_override: None,
                },
                "#!/bin/sh\nexec /usr/bin/open 'https://example.com'\n",
            ),
            (
                ShortcutAction::OpenUrl {
                    url: "https://example.com/docs?q=1".to_string(),
                    browser_override: Some(AppRef::BundleId {
                        id: "com.google.Chrome".to_string(),
                    }),
                },
                "#!/bin/sh\nexec /usr/bin/open '-b' 'com.google.Chrome' 'https://example.com/docs?q=1'\n",
            ),
            (
                ShortcutAction::OpenUrl {
                    url: "https://exa'mple.com/path".to_string(),
                    browser_override: Some(AppRef::Path {
                        path: "/Applications/Arc Browser.app".to_string(),
                    }),
                },
                "#!/bin/sh\nexec /usr/bin/open '-a' '/Applications/Arc Browser.app' 'https://exa'\\''mple.com/path'\n",
            ),
            (
                ShortcutAction::OpenUrl {
                    url: "https://example.com".to_string(),
                    browser_override: Some(AppRef::Name {
                        name: "Google Chrome".to_string(),
                    }),
                },
                "#!/bin/sh\nexec /usr/bin/open '-a' 'Google Chrome' 'https://example.com'\n",
            ),
            (
                ShortcutAction::LaunchApp {
                    app: AppRef::BundleId {
                        id: "com.apple.Safari".to_string(),
                    },
                },
                "#!/bin/sh\nexec /usr/bin/open '-b' 'com.apple.Safari'\n",
            ),
            (
                ShortcutAction::LaunchApp {
                    app: AppRef::Path {
                        path: "/Applications/Visual Studio Code.app".to_string(),
                    },
                },
                "#!/bin/sh\nexec /usr/bin/open '-a' '/Applications/Visual Studio Code.app'\n",
            ),
            (
                ShortcutAction::LaunchApp {
                    app: AppRef::Name {
                        name: "iTerm".to_string(),
                    },
                },
                "#!/bin/sh\nexec /usr/bin/open '-a' 'iTerm'\n",
            ),
        ];

        for (action, expected) in cases {
            let script = build_script(binary, &shortcut_entry(action));
            assert_eq!(script, expected);
        }
    }

    #[test]
    fn build_script_falls_back_to_qol_tray_exec_without_shortcut_action() {
        let script = build_script(
            Path::new("/Applications/qol-tray.app/Contents/MacOS/qol-tray"),
            &fallback_entry(),
        );

        assert_eq!(
            script,
            "#!/bin/sh\nexec '/Applications/qol-tray.app/Contents/MacOS/qol-tray' 'exec' 'shortcut' 'browser'\n"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_sanitized_display_name_always_returns_safe_non_empty_name(
            display_name in any::<String>(),
            file_stem in "[A-Za-z0-9_-]{1,32}"
        ) {
            let sanitized = sanitized_display_name(&LauncherEntry {
                file_stem: file_stem.clone(),
                display_name,
                description: String::new(),
                bundle_id: String::new(),
                exec_args: Vec::new(),
                shortcut_action: None,
            });

            prop_assert!(!sanitized.is_empty());
            prop_assert!(!sanitized.contains('/'));
            prop_assert!(!sanitized.contains(':'));
            prop_assert!(!sanitized.chars().any(|ch| ch.is_control()));
            prop_assert!(!sanitized.starts_with('.'));
            prop_assert!(!sanitized.ends_with('.'));
        }
    }
}
