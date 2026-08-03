use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Result};

use crate::theme::ColorScheme;

use super::super::kconfig;
use super::{installed_themes, DesktopBackend};

const KDE_GLOBALS: &str = "kdeglobals";
const KCM_INPUT: &str = "kcminputrc";

pub(in super::super) struct Kde;

impl DesktopBackend for Kde {
    fn current_scheme(&self) -> Result<ColorScheme> {
        match kconfig::get(KDE_GLOBALS, "General", "ColorScheme")? {
            Some(name) => Ok(classify(&name)),
            None => match kconfig::get(KDE_GLOBALS, "Colors:Window", "BackgroundNormal")? {
                Some(value) => classify_window_background(&value).ok_or_else(|| {
                    anyhow!("cannot parse kdeglobals Colors:Window BackgroundNormal {value:?}")
                }),
                None => Ok(ColorScheme::Light),
            },
        }
    }

    fn apply(&self, target: ColorScheme) -> Result<()> {
        apply_color_scheme(target)?;
        apply_icon_theme(target);
        apply_cursor_theme(target);
        apply_gtk_override(target);
        Ok(())
    }
}

fn apply_color_scheme(target: ColorScheme) -> Result<()> {
    let current = kconfig::get(KDE_GLOBALS, "General", "ColorScheme")?
        .unwrap_or_else(|| "Breeze".to_string());
    let installed = installed_color_schemes();
    let resolved = resolve(&current, target, &installed).ok_or_else(|| {
        anyhow!("no installed {target:?} color scheme counterpart for {current:?}")
    })?;
    match color_scheme_applier(tool_available("plasma-apply-colorscheme")) {
        ColorSchemeApplier::PlasmaApplyColorScheme => {
            if run_live_apply("plasma-apply-colorscheme", &[colors_basename(&resolved)]) {
                return Ok(());
            }
        }
        ColorSchemeApplier::ConfigWrite => {}
    }
    kconfig::set(KDE_GLOBALS, "General", "ColorScheme", &resolved)?;
    kconfig::set(KDE_GLOBALS, "General", "Name", &resolved)?;
    Ok(())
}

fn apply_icon_theme(target: ColorScheme) {
    let Ok(Some(current)) = kconfig::get(KDE_GLOBALS, "Icons", "Theme") else {
        return;
    };
    let installed = installed_icon_themes();
    match resolve(&current, target, &installed) {
        Some(name) => {
            if let Err(error) = kconfig::set(KDE_GLOBALS, "Icons", "Theme", &name) {
                eprintln!("[os-themes] icon theme not updated: {error:#}");
            }
        }
        None => eprintln!(
            "[os-themes] icon theme not updated: no installed {target:?} counterpart for {current:?}"
        ),
    }
}

fn apply_cursor_theme(target: ColorScheme) {
    let Ok(Some(current)) = kconfig::get(KCM_INPUT, "Mouse", "cursorTheme") else {
        return;
    };
    let installed = installed_cursor_themes();
    match resolve(&current, target, &installed) {
        Some(name) => {
            if let Err(error) = kconfig::set(KCM_INPUT, "Mouse", "cursorTheme", &name) {
                eprintln!("[os-themes] cursor theme not updated: {error:#}");
                return;
            }
            apply_cursor_live(&name);
        }
        None => eprintln!(
            "[os-themes] cursor theme not updated: no installed {target:?} counterpart for {current:?}"
        ),
    }
}

/// kapplymousetheme applies the cursor live over X11; the config write above
/// already persisted the change, so a live failure only delays the new cursor
/// until the next Plasma reload.
fn apply_cursor_live(name: &str) {
    let size = kconfig::get(KCM_INPUT, "Mouse", "cursorSize")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u32>().ok());
    match cursor_applier(tool_available("kapplymousetheme"), is_x11_session(), size) {
        CursorApplier::KApplyMouseTheme { size } => {
            if !run_live_apply("kapplymousetheme", &[name, &size.to_string()]) {
                eprintln!(
                    "[os-themes] cursor theme is persisted and applies after a Plasma reload"
                );
            }
        }
        CursorApplier::ConfigWrite => {}
    }
}

fn apply_gtk_override(target: ColorScheme) {
    let Ok(Some(current)) =
        kconfig::read_direct("gtk-3.0/settings.ini", "Settings", "gtk-theme-name")
    else {
        return;
    };
    let installed = installed_themes();
    match resolve(&current, target, &installed) {
        Some(name) => {
            for file in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
                if let Err(error) = kconfig::write_direct(file, "Settings", "gtk-theme-name", &name)
                {
                    eprintln!("[os-themes] gtk theme not updated: {error:#}");
                }
            }
        }
        None => eprintln!(
            "[os-themes] gtk theme not updated: no installed {target:?} counterpart for {current:?}"
        ),
    }
}

/// Preferred route for a color scheme change on Plasma. The official tool both
/// persists the change and notifies running apps; the config write is the
/// fallback and applies after a Plasma reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorSchemeApplier {
    PlasmaApplyColorScheme,
    ConfigWrite,
}

/// Preferred route for a cursor theme change on Plasma. kapplymousetheme
/// applies the cursor live over X11 only and needs the cursor size; the config
/// write is the fallback and applies after a Plasma reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorApplier {
    KApplyMouseTheme { size: u32 },
    ConfigWrite,
}

fn color_scheme_applier(plasma_apply_available: bool) -> ColorSchemeApplier {
    if plasma_apply_available {
        ColorSchemeApplier::PlasmaApplyColorScheme
    } else {
        ColorSchemeApplier::ConfigWrite
    }
}

/// kapplymousetheme applies the cursor live over X11 only and needs the cursor
/// size, so it is preferred only when the tool is present, the session is X11,
/// and the size is known.
fn cursor_applier(
    kapplymousetheme_available: bool,
    session_is_x11: bool,
    size: Option<u32>,
) -> CursorApplier {
    match (kapplymousetheme_available, session_is_x11, size) {
        (true, true, Some(size)) => CursorApplier::KApplyMouseTheme { size },
        _ => CursorApplier::ConfigWrite,
    }
}

/// The scheme name plasma-apply-colorscheme expects: the basename of the
/// scheme's .colors file, without the extension.
fn colors_basename(scheme: &str) -> &str {
    scheme.strip_suffix(".colors").unwrap_or(scheme)
}

/// Runs a live apply tool and reports whether it succeeded.
fn run_live_apply(tool: &str, args: &[&str]) -> bool {
    match Command::new(tool).args(args).status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("[os-themes] {tool} failed with {status}");
            false
        }
        Err(error) => {
            eprintln!("[os-themes] {tool} failed to run: {error:#}");
            false
        }
    }
}

fn tool_available(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(name))
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

fn is_x11_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|session| session == "x11")
        .unwrap_or(false)
}

fn classify(name: &str) -> ColorScheme {
    if name.to_ascii_lowercase().contains("dark") {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

fn classify_window_background(value: &str) -> Option<ColorScheme> {
    let parts: Vec<u32> = value
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    let [red, green, blue] = parts.as_slice() else {
        return None;
    };
    let gray = (299 * red + 587 * green + 114 * blue) / 1000;
    Some(if gray < 192 {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    })
}

fn resolve(current: &str, target: ColorScheme, installed: &[String]) -> Option<String> {
    if classify(current) == target {
        return Some(current.to_string());
    }
    let candidates = match target {
        ColorScheme::Light => light_candidates(current),
        ColorScheme::Dark => dark_candidates(current),
    };
    candidates
        .into_iter()
        .find(|candidate| installed.iter().any(|name| name == candidate))
}

fn dark_candidates(current: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for suffix in ["Light", "light"] {
        if let Some(stem) = current.strip_suffix(suffix) {
            candidates.push(format!("{stem}Dark"));
            candidates.push(format!("{stem}dark"));
        }
    }
    for suffix in ["Dark", "dark", "-Dark", "-dark", "_Dark", "_dark"] {
        candidates.push(format!("{current}{suffix}"));
    }
    for token in ["-", "_"] {
        let parts: Vec<&str> = current.split(token).collect();
        for position in 1..=parts.len() {
            let mut with_dark: Vec<&str> = parts.clone();
            with_dark.insert(position, "Dark");
            candidates.push(with_dark.join(token));
        }
    }
    candidates
}

fn light_candidates(current: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for suffix in ["-Dark", "-dark", "_Dark", "_dark", "Dark", "dark"] {
        if let Some(stem) = current.strip_suffix(suffix) {
            let separator = if suffix.starts_with('_') {
                "_"
            } else if suffix.starts_with('-') {
                "-"
            } else {
                ""
            };
            candidates.push(stem.to_string());
            candidates.push(format!("{stem}{separator}Light"));
            candidates.push(format!("{stem}{separator}light"));
            break;
        }
    }
    for token in ["-Dark", "-dark", "_Dark", "_dark"] {
        if let Some(stripped) = remove_token(current, token) {
            candidates.push(stripped);
        }
    }
    candidates
}

fn remove_token(value: &str, token: &str) -> Option<String> {
    let stripped = value.replacen(token, "", 1);
    (stripped != value).then_some(stripped)
}

fn installed_color_schemes() -> Vec<String> {
    let mut roots = vec![
        PathBuf::from("/usr/share/color-schemes"),
        PathBuf::from("/usr/local/share/color-schemes"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/color-schemes"));
    }
    file_stem_names(&roots, |path| path.extension() == Some("colors".as_ref()))
}

fn installed_icon_themes() -> Vec<String> {
    let roots = icon_roots();
    dir_names(&roots, |path| path.join("index.theme").is_file())
}

fn installed_cursor_themes() -> Vec<String> {
    let roots = icon_roots();
    dir_names(&roots, |path| path.join("cursors").is_dir())
}

fn icon_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".icons"));
        roots.push(PathBuf::from(&home).join(".local/share/icons"));
    }
    roots
}

fn file_stem_names(roots: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut names = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !keep(&path) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !names.iter().any(|name| name == stem) {
                names.push(stem.to_string());
            }
        }
    }
    names
}

fn dir_names(roots: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut names = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || !keep(&path) {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !names.iter().any(|existing| existing == &name) {
                names.push(name);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn classify_detects_dark_in_kde_names() {
        let cases = [
            ("BreezeLight", ColorScheme::Light),
            ("BreezeDark", ColorScheme::Dark),
            ("Breeze", ColorScheme::Light),
            ("Breeze_Light", ColorScheme::Light),
            ("Breeze_Dark", ColorScheme::Dark),
            ("breeze-dark", ColorScheme::Dark),
            ("Papirus-Dark", ColorScheme::Dark),
            ("Mint-Y", ColorScheme::Light),
        ];
        for (name, expected) in cases {
            assert_eq!(classify(name), expected, "name: {name}");
        }
    }

    #[test]
    fn classify_window_background_uses_the_kde_gray_heuristic() {
        let cases = [
            ("239,240,241", Some(ColorScheme::Light)),
            ("255,255,255", Some(ColorScheme::Light)),
            ("35,38,41", Some(ColorScheme::Dark)),
            ("0,0,0", Some(ColorScheme::Dark)),
            ("192,192,192", Some(ColorScheme::Light)),
            ("191,191,191", Some(ColorScheme::Dark)),
            ("not-a-color", None),
            ("12", None),
        ];
        for (value, expected) in cases {
            assert_eq!(
                classify_window_background(value),
                expected,
                "value: {value}"
            );
        }
    }

    #[test]
    fn resolve_derives_kde_counterparts_from_installed_names() {
        let schemes = installed(&["Breeze", "BreezeLight", "BreezeDark"]);
        let cursors = installed(&["Breeze_Light", "Breeze_Dark"]);
        let gtk = installed(&["Adwaita", "Adwaita-dark", "Mint-Y-Pink", "Mint-Y-Dark-Pink"]);
        let cases = [
            (
                "BreezeLight",
                ColorScheme::Dark,
                schemes.as_slice(),
                "BreezeDark",
            ),
            (
                "BreezeDark",
                ColorScheme::Light,
                schemes.as_slice(),
                "Breeze",
            ),
            (
                "Breeze",
                ColorScheme::Dark,
                schemes.as_slice(),
                "BreezeDark",
            ),
            (
                "Breeze_Light",
                ColorScheme::Dark,
                cursors.as_slice(),
                "Breeze_Dark",
            ),
            (
                "Breeze_Dark",
                ColorScheme::Light,
                cursors.as_slice(),
                "Breeze_Light",
            ),
            ("Adwaita", ColorScheme::Dark, gtk.as_slice(), "Adwaita-dark"),
            (
                "Mint-Y-Pink",
                ColorScheme::Dark,
                gtk.as_slice(),
                "Mint-Y-Dark-Pink",
            ),
            (
                "Mint-Y-Dark-Pink",
                ColorScheme::Light,
                gtk.as_slice(),
                "Mint-Y-Pink",
            ),
        ];
        for (current, target, names, expected) in cases {
            let resolved = resolve(current, target, names).unwrap();
            assert_eq!(resolved, expected, "current: {current}, target: {target:?}");
        }
    }

    #[test]
    fn resolve_keeps_current_when_already_matching() {
        let resolved = resolve("BreezeDark", ColorScheme::Dark, &installed(&[])).unwrap();
        assert_eq!(resolved, "BreezeDark");
    }

    #[test]
    fn resolve_returns_none_when_no_counterpart_installed() {
        let result = resolve("Custom", ColorScheme::Dark, &installed(&["Custom"]));
        assert!(result.is_none(), "expected no dark counterpart for Custom");
    }

    #[test]
    fn color_scheme_applier_prefers_the_official_tool_when_available() {
        assert_eq!(
            color_scheme_applier(true),
            ColorSchemeApplier::PlasmaApplyColorScheme
        );
        assert_eq!(color_scheme_applier(false), ColorSchemeApplier::ConfigWrite);
    }

    #[test]
    fn cursor_applier_requires_tool_x11_session_and_known_size() {
        let cases = [
            (
                true,
                true,
                Some(24),
                CursorApplier::KApplyMouseTheme { size: 24 },
            ),
            (true, false, Some(24), CursorApplier::ConfigWrite),
            (false, true, Some(24), CursorApplier::ConfigWrite),
            (true, true, None, CursorApplier::ConfigWrite),
            (false, false, None, CursorApplier::ConfigWrite),
        ];
        for (tool, x11, size, expected) in cases {
            assert_eq!(
                cursor_applier(tool, x11, size),
                expected,
                "tool: {tool}, x11: {x11}, size: {size:?}"
            );
        }
    }

    #[test]
    fn colors_basename_strips_the_extension_only_when_present() {
        assert_eq!(colors_basename("BreezeDark"), "BreezeDark");
        assert_eq!(colors_basename("BreezeDark.colors"), "BreezeDark");
        assert_eq!(colors_basename("My Scheme.colors"), "My Scheme");
    }
}
