use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const QOL_MIME: &str = "x-scheme-handler/qol";
const QOL_DESKTOP: &str = "qol-tray.desktop";

pub(super) fn contains_qol_association(path: &Path) -> Result<bool> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read MIME associations from {}", path.display()))?;
    Ok(content.lines().any(line_has_qol_association))
}

pub(super) fn remove_qol_association(path: &Path) -> Result<()> {
    let original = fs::read_to_string(path)
        .with_context(|| format!("Failed to read MIME associations from {}", path.display()))?;
    let updated = strip_qol_associations(&original);
    if updated == original {
        return Ok(());
    }
    crate::file_io::atomic_write(path, updated.as_bytes())
}

fn line_has_qol_association(line: &str) -> bool {
    let Some((key, values)) = line.split_once('=') else {
        return false;
    };
    key.trim() == QOL_MIME && association_values(values).any(|value| value == QOL_DESKTOP)
}

fn association_values(values: &str) -> impl Iterator<Item = &str> {
    values
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn strip_qol_associations(content: &str) -> String {
    let had_final_newline = content.ends_with('\n');
    let mut lines = Vec::new();
    for line in content.lines() {
        let Some(updated) = strip_qol_from_line(line) else {
            continue;
        };
        lines.push(updated);
    }
    let mut output = lines.join("\n");
    if had_final_newline {
        output.push('\n');
    }
    output
}

fn strip_qol_from_line(line: &str) -> Option<String> {
    let Some((key, values)) = line.split_once('=') else {
        return Some(line.to_string());
    };
    if key.trim() != QOL_MIME {
        return Some(line.to_string());
    }
    let remaining: Vec<_> = association_values(values)
        .filter(|value| *value != QOL_DESKTOP)
        .collect();
    if remaining.is_empty() {
        return None;
    }
    Some(format!("{key}={};", remaining.join(";")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_qol_associations_preserves_every_other_entry() {
        let cases = [
            (
                "[Default Applications]\nx-scheme-handler/qol=qol-tray.desktop\n",
                "[Default Applications]\n",
            ),
            (
                "x-scheme-handler/qol=qol-tray.desktop;other.desktop;\n",
                "x-scheme-handler/qol=other.desktop;\n",
            ),
            (
                "x-scheme-handler/qol=other.desktop;qol-tray.desktop;third.desktop;\n",
                "x-scheme-handler/qol=other.desktop;third.desktop;\n",
            ),
            (
                "x-scheme-handler/http=qol-tray.desktop;\nx-scheme-handler/qol=other.desktop;\n",
                "x-scheme-handler/http=qol-tray.desktop;\nx-scheme-handler/qol=other.desktop;\n",
            ),
            ("unrelated=true", "unrelated=true"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(strip_qol_associations(input), expected, "input={input:?}");
        }
    }

    #[test]
    fn contains_qol_association_requires_the_exact_mime_and_desktop_id() {
        let cases = [
            ("x-scheme-handler/qol=qol-tray.desktop", true),
            ("x-scheme-handler/qol=other.desktop;qol-tray.desktop;", true),
            ("x-scheme-handler/qol=qol-tray.desktop.backup", false),
            ("x-scheme-handler/http=qol-tray.desktop", false),
            ("x-scheme-handler/qol=other.desktop", false),
        ];
        for (line, expected) in cases {
            assert_eq!(line_has_qol_association(line), expected, "line={line:?}");
        }
    }
}
