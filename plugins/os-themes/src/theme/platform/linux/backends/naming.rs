use anyhow::{bail, Result};

use crate::theme::ColorScheme;

pub(super) fn classify(theme: &str) -> ColorScheme {
    if theme
        .split(['-', '_'])
        .any(|part| part.eq_ignore_ascii_case("dark"))
    {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

pub(super) fn resolve(current: &str, target: ColorScheme, installed: &[String]) -> Result<String> {
    if classify(current) == target {
        return Ok(current.to_string());
    }
    let counterpart = match target {
        ColorScheme::Light => light_variant(current, installed),
        ColorScheme::Dark => dark_variant(current, installed),
    };
    match counterpart {
        Some(name) => Ok(name),
        None => bail!("no installed {target:?} counterpart found for theme {current:?}"),
    }
}

fn light_variant(theme: &str, installed: &[String]) -> Option<String> {
    let stripped: Vec<&str> = theme
        .split('-')
        .filter(|part| !part.eq_ignore_ascii_case("dark"))
        .collect();
    let candidate = stripped.join("-");
    installed.iter().find(|name| **name == candidate).cloned()
}

fn dark_variant(theme: &str, installed: &[String]) -> Option<String> {
    let parts: Vec<&str> = theme.split('-').collect();
    let mut candidates = Vec::new();
    for position in 1..=parts.len() {
        let mut with_dark: Vec<&str> = parts.clone();
        with_dark.insert(position, "Dark");
        candidates.push(with_dark.join("-"));
    }
    candidates.push(format!("{theme}-dark"));
    candidates
        .into_iter()
        .find(|candidate| installed.iter().any(|name| name == candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorScheme;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn classify_detects_dark_segment() {
        let cases = [
            ("Mint-Y", ColorScheme::Light),
            ("Mint-Y-Dark", ColorScheme::Dark),
            ("Mint-Y-Dark-Pink", ColorScheme::Dark),
            ("Adwaita-dark", ColorScheme::Dark),
            ("Darkroom", ColorScheme::Light),
            ("foo_dark", ColorScheme::Dark),
        ];
        for (theme, expected) in cases {
            assert_eq!(classify(theme), expected, "theme: {theme}");
        }
    }

    #[test]
    fn resolve_derives_counterpart_from_installed_names() {
        let themes = installed(&["Mint-Y-Pink", "Mint-Y-Dark-Pink", "Adwaita", "Adwaita-dark"]);
        let cases = [
            ("Mint-Y-Dark-Pink", ColorScheme::Light, "Mint-Y-Pink"),
            ("Mint-Y-Pink", ColorScheme::Dark, "Mint-Y-Dark-Pink"),
            ("Adwaita", ColorScheme::Dark, "Adwaita-dark"),
            ("Adwaita-dark", ColorScheme::Light, "Adwaita"),
        ];
        for (current, target, expected) in cases {
            let resolved = resolve(current, target, &themes).unwrap();
            assert_eq!(resolved, expected, "current: {current}, target: {target:?}");
        }
    }

    #[test]
    fn resolve_keeps_current_when_already_matching() {
        let resolved = resolve("Mint-Y-Dark", ColorScheme::Dark, &installed(&[])).unwrap();
        assert_eq!(resolved, "Mint-Y-Dark");
    }

    #[test]
    fn resolve_errors_when_no_counterpart_installed() {
        let result = resolve(
            "Custom-Theme",
            ColorScheme::Dark,
            &installed(&["Custom-Theme"]),
        );
        assert!(
            result.is_err(),
            "expected no dark counterpart for Custom-Theme"
        );
    }
}
