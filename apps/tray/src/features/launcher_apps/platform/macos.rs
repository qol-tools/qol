use super::super::icon::{LauncherIcon, MarkFiles};
use super::super::LauncherEntry;
use crate::shortcuts::model::{AppRef, ShortcutAction};
use anyhow::{Context, Result};
use qol_theme::Mark;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const ICON_FILE: &str = "icon";
const ICON_PIXELS: u32 = 1024;

pub(super) fn sync(entries: &[LauncherEntry], target: &Path, marks: &MarkFiles) -> Result<()> {
    let dir = apps_dir().context("Could not determine home directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create launcher apps dir {}", dir.display()))?;

    let app_names = app_dirnames(entries);
    let expected: HashSet<String> = app_names.values().cloned().collect();

    for entry in entries {
        let Some(app_name) = app_names.get(&entry.file_stem) else {
            continue;
        };
        let app_dir = dir.join(app_name);
        write_app_bundle(&app_dir, entry, target, marks)?;
    }

    clean_stale(&dir, &expected)?;
    Ok(())
}

pub(super) fn apps_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Applications").join("QoL"))
}

pub(super) fn publish_synced() {
    use qol_runtime::protocol::RuntimeEvent;

    let Some(dir) = apps_dir() else {
        log::warn!("launcher_apps: no apps dir on this platform; skipping LauncherAppsSynced");
        return;
    };
    crate::runtime::publish(&[RuntimeEvent::LauncherAppsSynced { dir }]);
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

fn write_app_bundle(
    app_dir: &Path,
    entry: &LauncherEntry,
    target: &Path,
    marks: &MarkFiles,
) -> Result<()> {
    if build_shortcut_script(entry).is_none() {
        super::verify_target(entry, target)?;
    }
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");
    let run_path = macos_dir.join("run");
    let plist_path = contents_dir.join("Info.plist");
    let icon_path = resources_dir.join(format!("{ICON_FILE}.icns"));

    let expected_script = build_script(target, entry);
    let expected_plist = build_info_plist(entry, launcher_icon(&entry.icon, marks).as_deref());
    let expected_icon = icon_png(&entry.icon, marks).map(|png| icns(&png));

    if file_matches(&run_path, &expected_script)
        && file_matches(&plist_path, &expected_plist)
        && is_executable(&run_path)
        && expected_icon
            .as_ref()
            .is_none_or(|icon| std::fs::read(&icon_path).is_ok_and(|current| &current == icon))
    {
        return Ok(());
    }

    std::fs::create_dir_all(&macos_dir)?;
    std::fs::write(&plist_path, &expected_plist)?;
    write_executable(&run_path, &expected_script)?;
    if let Some(icon) = expected_icon {
        std::fs::create_dir_all(&resources_dir)?;
        std::fs::write(&icon_path, icon)?;
    }
    Ok(())
}

fn icon_png(icon: &LauncherIcon, marks: &MarkFiles) -> Option<Vec<u8>> {
    let mark = match icon {
        LauncherIcon::Mark(mark) => *mark,
        LauncherIcon::TargetApp(app) => match target_app_png(app) {
            Some(png) => return Some(png),
            None => Mark::App,
        },
    };
    mark_png(&marks.svg(mark))
}

fn launcher_icon(icon: &LauncherIcon, marks: &MarkFiles) -> Option<PathBuf> {
    match icon {
        LauncherIcon::Mark(mark) => Some(marks.path(*mark)),
        LauncherIcon::TargetApp(_) => None,
    }
}

fn target_app_png(app: &AppRef) -> Option<Vec<u8>> {
    let size = ICON_PIXELS as usize;
    match app {
        AppRef::Path { path } => qol_app_icon::icon_png_for_path(Path::new(path), size),
        AppRef::BundleId { id } => qol_app_icon::icon_png_for_bundle_id(id, size),
        AppRef::Name { .. } => None,
    }
}

fn mark_png(svg: &str) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};

    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(ICON_PIXELS, ICON_PIXELS)?;
    let scale = ICON_PIXELS as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

fn icns(png: &[u8]) -> Vec<u8> {
    let entry_len = 8 + png.len();
    let total_len = 8 + entry_len;
    let mut icns = Vec::with_capacity(total_len);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&(total_len as u32).to_be_bytes());
    icns.extend_from_slice(b"ic10");
    icns.extend_from_slice(&(entry_len as u32).to_be_bytes());
    icns.extend_from_slice(png);
    icns
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

fn build_info_plist(entry: &LauncherEntry, launcher_icon: Option<&Path>) -> String {
    let name = xml_escape(&entry.display_name);
    let launcher_icon = launcher_icon.map_or(String::new(), |path| {
        format!(
            "<key>{}</key><string>{}</string>\n",
            qol_apps::LAUNCHER_ICON_KEY,
            xml_escape(&path.display().to_string())
        )
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>CFBundleExecutable</key><string>run</string>\n\
         <key>CFBundleIconFile</key><string>{}</string>\n\
         <key>CFBundleDisplayName</key><string>{}</string>\n\
         <key>CFBundleName</key><string>{}</string>\n\
         <key>CFBundleIdentifier</key><string>{}</string>\n\
         <key>CFBundlePackageType</key><string>APPL</string>\n\
         <key>CFBundleVersion</key><string>1</string>\n\
         <key>CFBundleShortVersionString</key><string>1</string>\n\
         <key>LSUIElement</key><true/>\n\
         {}\
         </dict>\n\
         </plist>\n",
        ICON_FILE, name, name, entry.bundle_id, launcher_icon
    )
}

fn write_executable(path: &Path, script: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn build_script(target: &Path, entry: &LauncherEntry) -> String {
    if let Some(script) = build_shortcut_script(entry) {
        return script;
    }
    let bin = shell_escape_single_quote(&target.display().to_string());
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
        ShortcutAction::PluginAction { .. } => None,
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
            icon: LauncherIcon::Mark(Mark::Qol),
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
            icon: LauncherIcon::Mark(Mark::Qol),
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
            icon: LauncherIcon::Mark(Mark::Qol),
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

    #[test]
    fn build_script_falls_back_to_qol_tray_exec_for_plugin_action_shortcuts() {
        let script = build_script(
            Path::new("/Applications/qol-tray.app/Contents/MacOS/qol-tray"),
            &shortcut_entry(ShortcutAction::PluginAction {
                plugin_id: "qol-cli-sessions".to_string(),
                action: "open".to_string(),
            }),
        );

        assert_eq!(
            script,
            "#!/bin/sh\nexec '/Applications/qol-tray.app/Contents/MacOS/qol-tray' 'exec' 'shortcut' 'browser'\n"
        );
    }

    #[test]
    fn icns_wraps_one_png_entry_with_big_endian_lengths() {
        let icns = icns(b"PNG");

        assert_eq!(&icns[..4], b"icns");
        assert_eq!(u32::from_be_bytes(icns[4..8].try_into().unwrap()), 19);
        assert_eq!(&icns[8..12], b"ic10");
        assert_eq!(u32::from_be_bytes(icns[12..16].try_into().unwrap()), 11);
        assert_eq!(&icns[16..], b"PNG");
    }

    #[test]
    fn every_mark_rasterizes_to_a_png() {
        for mark in Mark::ALL {
            let png = mark_png(&mark.svg(0)).unwrap_or_else(|| panic!("{}", mark.name()));
            assert!(png.starts_with(b"\x89PNG"), "{}", mark.name());
        }
    }

    #[test]
    fn info_plist_names_the_bundle_icon() {
        assert!(build_info_plist(&entry("shortcut-a", "A"), None)
            .contains("<key>CFBundleIconFile</key><string>icon</string>"));
    }

    #[test]
    fn mark_entries_point_the_launcher_at_their_mark_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let marks = MarkFiles::in_dir(tmp.path().join("marks"), 0);
        let app = tmp.path().join("Open Browser.app");

        write_app_bundle(
            &app,
            &shortcut_entry(ShortcutAction::OpenUrl {
                url: "https://example.com".to_string(),
                browser_override: None,
            }),
            &tmp.path().join("qol-tray"),
            &marks,
        )
        .unwrap();

        assert_eq!(
            qol_apps::read_macos_bundle_facts(&app).launcher_icon,
            Some(marks.path(Mark::Qol).display().to_string())
        );
    }

    #[test]
    fn target_app_entries_keep_the_app_icon() {
        let marks = MarkFiles::in_dir(PathBuf::from("/marks"), 0);
        let icon = LauncherIcon::TargetApp(AppRef::Name {
            name: "Safari".to_string(),
        });

        assert_eq!(launcher_icon(&icon, &marks), None);
    }

    #[test]
    fn build_script_keeps_qol_tray_for_command_entries() {
        let entry = LauncherEntry {
            file_stem: "command-shortcuts-add".to_string(),
            display_name: "QoL \u{203a} Add Shortcut".to_string(),
            description: String::new(),
            bundle_id: String::new(),
            exec_args: vec!["open".into(), "shortcuts/add".into()],
            shortcut_action: None,
            icon: LauncherIcon::Mark(Mark::Qol),
        };
        let script = build_script(
            Path::new("/Applications/qol-tray.app/Contents/MacOS/qol-tray"),
            &entry,
        );

        assert_eq!(
            script,
            "#!/bin/sh\nexec '/Applications/qol-tray.app/Contents/MacOS/qol-tray' 'open' 'shortcuts/add'\n"
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
                icon: LauncherIcon::Mark(Mark::Qol),
            });

            prop_assert!(!sanitized.is_empty());
            prop_assert!(!sanitized.contains('/'));
            prop_assert!(!sanitized.contains(':'));
            prop_assert!(!sanitized.chars().any(|ch| ch.is_control()));
            prop_assert!(!sanitized.starts_with('.'));
            prop_assert!(!sanitized.ends_with('.'));
        }
    }

    #[test]
    fn app_bundle_refuses_a_missing_referenced_binary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");

        let error = write_app_bundle(
            tmp.path(),
            &entry("command-shortcuts-add", "Add Shortcut"),
            &binary,
            &MarkFiles::in_dir(tmp.path().join("marks"), 0),
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing binary"), "got: {error}");
        assert!(
            !tmp.path().join("Contents").exists(),
            "a dead app bundle must not be written"
        );
    }

    #[test]
    fn app_bundle_without_a_binary_reference_skips_verification() {
        let tmp = tempfile::TempDir::new().unwrap();
        let binary = tmp.path().join("qol-tray");

        write_app_bundle(
            tmp.path(),
            &shortcut_entry(ShortcutAction::OpenUrl {
                url: "https://example.com".to_string(),
                browser_override: None,
            }),
            &binary,
            &MarkFiles::in_dir(tmp.path().join("marks"), 0),
        )
        .unwrap();

        let script = std::fs::read_to_string(tmp.path().join("Contents/MacOS/run")).unwrap();
        assert!(script.starts_with("#!/bin/sh\nexec /usr/bin/open"));
    }
}
