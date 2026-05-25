use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::AutostartOps;

const BUNDLE_ID: &str = "com.qol-tools.qol-tray";

pub(crate) struct Platform;

impl AutostartOps for Platform {
    fn read_target(&self) -> Result<Option<PathBuf>> {
        let path = autostart_path_impl()?;
        read_plist_at(&path)
    }

    fn write_target(&self, binary: &Path) -> Result<()> {
        let path = autostart_path_impl()?;
        write_plist_to(&path, binary)
    }

    fn autostart_path(&self) -> Result<PathBuf> {
        autostart_path_impl()
    }
}

fn autostart_path_impl() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{BUNDLE_ID}.plist")))
}

fn write_plist_to(path: &Path, binary: &Path) -> Result<()> {
    let escaped = xml_escape(&binary.display().to_string());
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>Label</key>\n\
         <string>{BUNDLE_ID}</string>\n\
         <key>ProgramArguments</key>\n\
         <array>\n\
         <string>{escaped}</string>\n\
         </array>\n\
         <key>RunAtLoad</key>\n\
         <true/>\n\
         <key>KeepAlive</key>\n\
         <false/>\n\
         </dict>\n\
         </plist>\n"
    );
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Autostart path has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    std::fs::write(path, plist)
        .with_context(|| format!("Failed to write autostart file {}", path.display()))?;
    Ok(())
}

fn read_plist_at(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_program_argument(&content).map(PathBuf::from))
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

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_then_read(plist_path: &Path, binary: &Path) -> Option<PathBuf> {
        write_plist_to(plist_path, binary).unwrap();
        read_plist_at(plist_path).unwrap()
    }

    #[test]
    fn round_trip_plain_path() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp.path().join("autostart.plist");
        let binary = PathBuf::from("/Users/x/y");
        assert_eq!(write_then_read(&plist, &binary), Some(binary));
    }

    #[test]
    fn round_trip_path_with_spaces() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp.path().join("autostart.plist");
        let binary =
            PathBuf::from("/Users/x with space/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
        assert_eq!(write_then_read(&plist, &binary), Some(binary));
    }

    #[test]
    fn round_trip_path_with_xml_special_chars() {
        let tmp = TempDir::new().unwrap();
        let plist = tmp.path().join("autostart.plist");
        let binary = PathBuf::from("/Users/x/a&b<c>d/qol-tray");
        assert_eq!(write_then_read(&plist, &binary), Some(binary));
    }
}
