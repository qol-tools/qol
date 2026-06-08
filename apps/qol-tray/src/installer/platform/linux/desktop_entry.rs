use std::path::Path;

pub(crate) enum DesktopExecArg<'a> {
    Literal(&'a str),
    Url,
}

pub(crate) fn format_desktop_exec_command(program: &Path, args: &[DesktopExecArg<'_>]) -> String {
    std::iter::once(format_desktop_exec_path(program))
        .chain(args.iter().map(format_desktop_exec_arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_desktop_exec_program(command: &str) -> Option<String> {
    parse_first_desktop_exec_token(command).map(unescape_desktop_field_codes)
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
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn formats_program_path_with_spaces() {
        let input = Path::new("/tmp/qol tray/qol-tray");
        let command = format_desktop_exec_command(input, &[]);
        assert_eq!(command, "\"/tmp/qol tray/qol-tray\"");
        assert_eq!(
            parse_desktop_exec_program(&command),
            Some("/tmp/qol tray/qol-tray".to_string())
        );
    }

    #[test]
    fn formats_url_field_code_outside_quotes() {
        let command = format_desktop_exec_command(
            Path::new("/tmp/qol tray/qol-tray"),
            &[DesktopExecArg::Url],
        );
        assert_eq!(command, "\"/tmp/qol tray/qol-tray\" %u");
    }

    #[test]
    fn formats_literal_args_as_quoted_tokens() {
        let command = format_desktop_exec_command(
            Path::new("/tmp/qol tray/qol-tray"),
            &[
                DesktopExecArg::Literal("exec"),
                DesktopExecArg::Literal("shortcut id"),
                DesktopExecArg::Literal("path%to%tool"),
            ],
        );
        assert_eq!(
            command,
            "\"/tmp/qol tray/qol-tray\" \"exec\" \"shortcut id\" \"path%%to%%tool\""
        );
    }

    #[test]
    fn parses_quoted_exec_token_with_args() {
        let input = "\"/tmp/qol tray/qol-tray\" %u";
        assert_eq!(
            parse_desktop_exec_program(input),
            Some("/tmp/qol tray/qol-tray".to_string())
        );
    }

    #[test]
    fn unescapes_literal_percent_pairs() {
        let command = format_desktop_exec_command(
            Path::new("path%to%tool"),
            &[DesktopExecArg::Literal("arg")],
        );
        assert_eq!(command, "\"path%%to%%tool\" \"arg\"");
        assert_eq!(
            parse_desktop_exec_program(&command),
            Some("path%to%tool".to_string())
        );
    }

    #[test]
    fn rejects_unclosed_quoted_program() {
        assert_eq!(parse_desktop_exec_program("\"/tmp/qol tray"), None);
    }
}
