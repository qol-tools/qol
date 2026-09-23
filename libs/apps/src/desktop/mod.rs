use std::fs;
use std::path::{Path, PathBuf};

const EXEC_FIELD_CODES: &[&str] = &[
    "%u", "%U", "%f", "%F", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m",
];

mod platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub exec: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRoot {
    pub path: PathBuf,
    pub max_depth: usize,
}

impl AppRoot {
    pub fn watch_recursive(&self) -> bool {
        self.max_depth <= 1
    }
}

pub enum DesktopExecArg<'a> {
    Literal(&'a str),
    Url,
}

pub fn xdg_cache_dir() -> Option<PathBuf> {
    platform::cache_dir()
}

pub fn linux_app_roots() -> Vec<AppRoot> {
    platform::app_roots()
}

pub fn scan_desktop_root(root: &AppRoot) -> Vec<AppEntry> {
    let mut entries = Vec::new();
    walk_for_desktop(&root.path, 0, root.max_depth, &mut entries);
    entries
}

pub fn parse_desktop_entry_file(path: &Path) -> Option<AppEntry> {
    let content = fs::read_to_string(path).ok()?;
    parse_desktop_entry_content(&content, path)
}

pub fn parse_desktop_entry_content(content: &str, path: &Path) -> Option<AppEntry> {
    if content
        .lines()
        .any(|line| line == "NoDisplay=true" || line == "Hidden=true")
    {
        return None;
    }

    let exec_raw = desktop_field(content, "Exec=")?;
    let exec = shell_words::split(&exec_raw)
        .ok()?
        .into_iter()
        .filter(|token| !EXEC_FIELD_CODES.contains(&token.as_str()))
        .collect();

    Some(AppEntry {
        name: desktop_field(content, "Name=")?,
        exec,
        path: path.to_path_buf(),
    })
}

pub fn format_desktop_exec_command(program: &Path, args: &[DesktopExecArg<'_>]) -> String {
    std::iter::once(format_desktop_exec_path(program))
        .chain(args.iter().map(format_desktop_exec_arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_desktop_exec_program(command: &str) -> Option<String> {
    parse_first_desktop_exec_token(command).map(unescape_desktop_field_codes)
}

pub fn escape_desktop_entry_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

fn walk_for_desktop(dir: &Path, depth: usize, max_depth: usize, entries: &mut Vec<AppEntry>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };

        if (file_type.is_file() || file_type.is_symlink())
            && path.extension().is_some_and(|ext| ext == "desktop")
        {
            if let Some(parsed) = parse_desktop_entry_file(&path) {
                entries.push(parsed);
            }
            continue;
        }

        if !file_type.is_dir() || depth >= max_depth {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with('.') || (depth == 0 && name == "share") {
            continue;
        }
        walk_for_desktop(&path, depth + 1, max_depth, entries);
    }
}

fn desktop_field(content: &str, prefix: &str) -> Option<String> {
    content
        .lines()
        .find(|line| line.starts_with(prefix))
        .map(|line| line[prefix.len()..].to_string())
}

fn format_desktop_exec_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    format_desktop_exec_literal(path.as_ref())
}

fn format_desktop_exec_arg(arg: &DesktopExecArg<'_>) -> String {
    match arg {
        DesktopExecArg::Literal(value) => format_desktop_exec_literal(value),
        DesktopExecArg::Url => "%u".to_string(),
    }
}

fn format_desktop_exec_literal(value: &str) -> String {
    format!("\"{}\"", escape_desktop_exec_token(value))
}

fn escape_desktop_exec_token(arg: &str) -> String {
    let mut escaped = String::with_capacity(arg.len());
    for ch in arg.chars() {
        match ch {
            '"' | '`' | '$' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            '%' => escaped.push_str("%%"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn unescape_desktop_field_codes(value: String) -> String {
    value.replace("%%", "%")
}

fn parse_first_desktop_exec_token(command: &str) -> Option<String> {
    let mut chars = command.trim_start().chars();
    match chars.next()? {
        '"' => parse_quoted_desktop_token(chars),
        first => parse_unquoted_desktop_token(std::iter::once(first).chain(chars)),
    }
}

fn parse_quoted_desktop_token(chars: impl Iterator<Item = char>) -> Option<String> {
    let mut output = String::new();
    let mut escaped = false;

    for ch in chars {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return non_empty(output),
            _ => output.push(ch),
        }
    }

    None
}

fn parse_unquoted_desktop_token(chars: impl Iterator<Item = char>) -> Option<String> {
    let mut output = String::new();
    let mut escaped = false;

    for ch in chars {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch.is_whitespace() {
            break;
        }
        output.push(ch);
    }

    if escaped {
        output.push('\\');
    }
    non_empty(output)
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests;
