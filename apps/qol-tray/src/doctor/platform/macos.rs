use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) fn read_autostart_target() -> Result<Option<PathBuf>> {
    let path = crate::installer::autostart_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_program_argument(&content).map(PathBuf::from))
}

pub(super) fn install_marker_required(current_exe: &Path) -> bool {
    !current_exe
        .to_string_lossy()
        .contains(".app/Contents/MacOS/")
}

fn parse_program_argument(content: &str) -> Option<String> {
    let key_pos = content.find("<key>ProgramArguments</key>")?;
    let after_key = &content[key_pos..];
    let array_pos = after_key.find("<array>")?;
    let after_array = &after_key[array_pos + "<array>".len()..];
    let string_pos = after_array.find("<string>")?;
    let after_string = &after_array[string_pos + "<string>".len()..];
    let end_pos = after_string.find("</string>")?;
    let raw = &after_string[..end_pos];
    Some(xml_unescape(raw))
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
