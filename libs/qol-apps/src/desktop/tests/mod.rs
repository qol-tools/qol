use std::path::Path;

use super::*;

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
    let command =
        format_desktop_exec_command(Path::new("/tmp/qol tray/qol-tray"), &[DesktopExecArg::Url]);
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
    let command =
        format_desktop_exec_command(Path::new("path%to%tool"), &[DesktopExecArg::Literal("arg")]);
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

#[test]
fn parses_visible_desktop_entry() {
    let content = "\
[Desktop Entry]
Name=Calculator
Exec=gnome-calculator %U --new-window
";
    let path = Path::new("/usr/share/applications/calculator.desktop");
    let entry = parse_desktop_entry_content(content, path).unwrap();
    assert_eq!(entry.name, "Calculator");
    assert_eq!(entry.exec, vec!["gnome-calculator", "--new-window"]);
    assert_eq!(entry.path, path.to_path_buf());
}

#[test]
fn skips_hidden_desktop_entry() {
    let content = "\
[Desktop Entry]
Name=Hidden
Hidden=true
Exec=hidden
";
    assert_eq!(
        parse_desktop_entry_content(content, Path::new("/tmp/hidden.desktop")),
        None
    );
}

#[cfg(unix)]
#[test]
fn scan_desktop_root_follows_flatpak_style_launcher_symlinks() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let exports = tmp.path().join("exports/share/applications");
    let target = tmp
        .path()
        .join("app/com.acme.Widget/current/widget.desktop");
    fs::create_dir_all(&exports).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(
        &target,
        "[Desktop Entry]\nName=Widget\nExec=flatpak run com.acme.Widget\n",
    )
    .unwrap();
    symlink(&target, exports.join("com.acme.Widget.desktop")).unwrap();

    let entries = scan_desktop_root(&AppRoot {
        path: exports,
        max_depth: 1,
    });

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Widget");
    assert_eq!(
        entries[0].exec.last().map(String::as_str),
        Some("com.acme.Widget")
    );
}

#[test]
fn escapes_desktop_entry_values() {
    assert_eq!(
        escape_desktop_entry_value("one\\two\nthree\tfour\rfive"),
        "one\\\\two\\nthree\\tfour\\rfive"
    );
}
