use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Text,
    Code,
    Image,
    Link,
    Command,
    Secret,
    Color,
    File,
}

const COMMANDS: [&str; 27] = [
    "apt",
    "brew",
    "cargo",
    "cat",
    "cd",
    "curl",
    "docker",
    "git",
    "grep",
    "journalctl",
    "kubectl",
    "ls",
    "make",
    "mkdir",
    "node",
    "npm",
    "npx",
    "pnpm",
    "python",
    "python3",
    "qol",
    "rg",
    "scp",
    "ssh",
    "sudo",
    "systemctl",
    "wget",
];

const CODE_WORDS: [&str; 10] = [
    "fn ",
    "def ",
    "function ",
    "class ",
    "import ",
    "#include",
    "const ",
    "let ",
    "return ",
    "pub ",
];

impl ClipKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Code => "Code",
            Self::Image => "Image",
            Self::Link => "Link",
            Self::Command => "Command",
            Self::Secret => "Secret",
            Self::Color => "Color",
            Self::File => "File",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Code => "code",
            Self::Image => "image",
            Self::Link => "link",
            Self::Command => "command",
            Self::Secret => "secret",
            Self::Color => "color",
            Self::File => "file",
        }
    }

    pub fn mono(self) -> bool {
        matches!(
            self,
            Self::Code | Self::Command | Self::Link | Self::Color | Self::File
        )
    }
}

pub fn classify_text(text: &str) -> ClipKind {
    let trimmed = text.trim();
    let single_line = !trimmed.contains('\n');
    if single_line && swatch(trimmed).is_some() {
        return ClipKind::Color;
    }
    if single_line && is_link(trimmed) {
        return ClipKind::Link;
    }
    if single_line && trimmed.starts_with("file://") {
        return ClipKind::File;
    }
    if single_line && is_command(trimmed) {
        return ClipKind::Command;
    }
    if is_code(trimmed) {
        return ClipKind::Code;
    }
    ClipKind::Text
}

pub fn swatch(text: &str) -> Option<String> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 | 4 => Some(format!(
            "#{}",
            hex.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_lowercase()
        )),
        6 | 8 => Some(format!("#{}", hex.to_lowercase())),
        _ => None,
    }
}

fn is_link(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && ["https://", "http://", "www.", "mailto:"]
            .iter()
            .any(|prefix| text.starts_with(prefix))
}

fn is_command(text: &str) -> bool {
    if text.starts_with("$ ") {
        return true;
    }
    let first = text.split_whitespace().next().unwrap_or_default();
    COMMANDS.contains(&first) && text.len() > first.len()
}

fn is_code(text: &str) -> bool {
    let json = (text.starts_with('{') && text.ends_with('}'))
        || (text.starts_with('[') && text.ends_with(']') && text.contains('\n'));
    if json {
        return true;
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return CODE_WORDS.iter().any(|word| text.starts_with(word)) && text.ends_with([';', '{']);
    }
    let indented = lines
        .iter()
        .filter(|line| line.starts_with("    ") || line.starts_with('\t'))
        .count();
    let punctuated = lines
        .iter()
        .filter(|line| line.trim_end().ends_with([';', '{', '}', ')', ',', ':']))
        .count();
    let keyword = lines.iter().any(|line| {
        CODE_WORDS
            .iter()
            .any(|word| line.trim_start().starts_with(word))
    });
    keyword || (indented > 0 && punctuated * 2 >= lines.len())
}

pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if png.get(..8)? != SIGNATURE || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

#[cfg(test)]
pub(crate) fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_classification_table() {
        let cases = [
            (
                "https://github.com/qol-tools/qol-monorepo/pull/147",
                ClipKind::Link,
            ),
            ("www.example.com", ClipKind::Link),
            ("#d9822b", ClipKind::Color),
            ("git push origin main", ClipKind::Command),
            ("$ ls -la", ClipKind::Command),
            ("file:///home/me/notes.md", ClipKind::File),
            ("{\n  \"a\": 1\n}", ClipKind::Code),
            ("fn main() {\n    run();\n}", ClipKind::Code),
            ("let x = 1;", ClipKind::Code),
            ("Meet at the station at nine.", ClipKind::Text),
            ("git", ClipKind::Text),
            (
                "Dear team,\nthe release moved to Friday.\nThanks",
                ClipKind::Text,
            ),
            ("#zzz", ClipKind::Text),
        ];
        for (text, kind) in cases {
            assert_eq!(classify_text(text), kind, "text: {text:?}");
        }
    }

    #[test]
    fn swatches_normalize_to_lowercase_hex() {
        let cases = [
            ("#D9822B", Some("#d9822b")),
            ("#abc", Some("#aabbcc")),
            ("#11223344", Some("#11223344")),
            ("#12345", None),
            ("d9822b", None),
        ];
        for (text, expected) in cases {
            assert_eq!(swatch(text).as_deref(), expected, "text: {text}");
        }
    }

    #[test]
    fn png_size_reads_the_ihdr_header() {
        assert_eq!(png_size(&png_header(856, 626)), Some((856, 626)));
        assert_eq!(png_size(b"not a png"), None);
    }
}
