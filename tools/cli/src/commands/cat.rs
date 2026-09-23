use anyhow::{anyhow, bail, Context, Result};
use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PagerMode {
    Auto,
    Always,
    Never,
}

struct CatArgs<'a> {
    path: &'a str,
    color: ColorMode,
    pager: PagerMode,
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h")))
    {
        println!("{}", cat_help());
        return Ok(());
    }
    let args = parse_args(args)?;
    let content = if args.path == "-" {
        let mut content = String::new();
        io::stdin()
            .read_to_string(&mut content)
            .context("failed to read stdin")?;
        content
    } else {
        display_content(Path::new(args.path))?
    };
    let pager = match args.pager {
        PagerMode::Auto => io::stdout().is_terminal(),
        PagerMode::Always => true,
        PagerMode::Never => false,
    };
    let color = match args.color {
        ColorMode::Always => true,
        ColorMode::Auto => pager || io::stdout().is_terminal(),
        ColorMode::Never => false,
    };
    let output = numbered(&content, color);
    if pager {
        let shown = less(&output, args.pager == PagerMode::Auto)?;
        if !shown {
            print!("{output}");
        }
    } else {
        print!("{output}");
    }
    Ok(())
}

fn parse_args(args: &[OsString]) -> Result<CatArgs<'_>> {
    let mut path = None;
    let mut color = ColorMode::Auto;
    let mut pager = PagerMode::Auto;
    for arg in args {
        let arg = arg
            .to_str()
            .ok_or_else(|| anyhow!("argument is not valid UTF-8"))?;
        match arg {
            "--less" | "--pager" => pager = PagerMode::Always,
            "--no-less" | "--no-pager" | "--stdout" => pager = PagerMode::Never,
            "--plain" | "--color=never" => color = ColorMode::Never,
            "--color" | "--color=always" => color = ColorMode::Always,
            "--color=auto" => color = ColorMode::Auto,
            "--help" | "-h" => bail!("{}", cat_help()),
            _ if arg.starts_with("--color=") => {
                bail!("unsupported color mode `{arg}`\n\n{}", cat_help())
            }
            _ if arg.starts_with('-') && arg != "-" => {
                bail!("unknown option `{arg}`\n\n{}", cat_help())
            }
            _ => {
                if path.replace(arg).is_some() {
                    bail!("{}", cat_help());
                }
            }
        }
    }
    let Some(path) = path else {
        bail!("{}", cat_help());
    };
    Ok(CatArgs { path, color, pager })
}

fn cat_help() -> &'static str {
    "usage: qol cat [--less|--no-less] [--plain|--color=auto|--color=always|--color=never] <path|->"
}

fn less(output: &str, auto: bool) -> Result<bool> {
    let child = Command::new("less")
        .args(["-R", "-F", "-X"])
        .stdin(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) if auto && error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).context("failed to start `less`; install less or run with --no-less")
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(output.as_bytes()) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::BrokenPipe => {}
            Err(error) => return Err(error).context("failed to write to less"),
        }
    }
    let status = child.wait().context("failed to wait for less")?;
    if !status.success() {
        bail!("less exited with {status}");
    }
    Ok(true)
}

fn display_content(path: &Path) -> Result<String> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
        if let Some(formatted) = rustfmt_stdout(&raw) {
            return Ok(formatted);
        }
    }
    Ok(raw)
}

fn rustfmt_stdout(content: &str) -> Option<String> {
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(content.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn numbered(content: &str, color: bool) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let width = lines.len().to_string().len().max(2);
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if color {
            out.push_str(&format!(
                "\x1b[90m{:>width$} │\x1b[0m {}\n",
                index + 1,
                highlight_rust_line(line)
            ));
        } else {
            out.push_str(&format!("{:>width$} │ {line}\n", index + 1));
        }
    }
    out
}

fn highlight_rust_line(line: &str) -> String {
    if line.trim_start().starts_with("//") {
        return paint(line, "90");
    }
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            let mut token = String::from(ch);
            let mut escaped = false;
            for next in chars.by_ref() {
                token.push(next);
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == '"' {
                    break;
                }
            }
            out.push_str(&paint(&token, "32"));
        } else if ch == '\'' {
            let mut lookahead = chars.clone();
            if lookahead.peek().is_some_and(|next| is_ident_start(*next)) {
                let mut token = String::from(ch);
                while chars.peek().is_some_and(|next| is_ident_continue(*next)) {
                    token.push(chars.next().unwrap());
                }
                out.push_str(&token);
            } else {
                let mut token = String::from(ch);
                let mut escaped = false;
                for next in chars.by_ref() {
                    token.push(next);
                    if escaped {
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == '\'' {
                        break;
                    }
                }
                out.push_str(&paint(&token, "32"));
            }
        } else if ch == '/' && chars.peek() == Some(&'/') {
            let mut rest = String::from(ch);
            rest.extend(chars);
            out.push_str(&paint(&rest, "90"));
            break;
        } else if is_ident_start(ch) {
            let mut token = String::from(ch);
            while chars.peek().is_some_and(|next| is_ident_continue(*next)) {
                token.push(chars.next().unwrap());
            }
            if is_rust_keyword(&token) {
                out.push_str(&paint(&token, "1;34"));
            } else if chars.peek() == Some(&'!') {
                out.push_str(&paint(&token, "35"));
            } else {
                out.push_str(&token);
            }
        } else if ch.is_ascii_digit() {
            let mut token = String::from(ch);
            while chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_' || *next == '.')
            {
                token.push(chars.next().unwrap());
            }
            out.push_str(&paint(&token, "36"));
        } else {
            out.push(ch);
        }
    }
    out
}

fn paint(text: &str, code: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_rust_keyword(token: &str) -> bool {
    matches!(
        token,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_with_stable_width() {
        assert_eq!(numbered("a\nb\n", false), " 1 │ a\n 2 │ b\n");
    }

    #[test]
    fn empty_input_prints_nothing() {
        assert_eq!(numbered("", false), "");
    }

    #[test]
    fn colorizes_keywords_and_strings() {
        let line = highlight_rust_line("let s = \"hi\"; // ok");
        assert!(line.contains("\x1b[1;34mlet\x1b[0m"));
        assert!(line.contains("\x1b[32m\"hi\"\x1b[0m"));
        assert!(line.contains("\x1b[90m// ok\x1b[0m"));
    }

    #[test]
    fn leaves_lifetimes_unpainted() {
        let line = highlight_rust_line("fn x() -> &'static str");
        assert!(line.contains("&'static str"));
        assert!(!line.contains("\x1b[32m'static str"));
    }

    #[test]
    fn pager_defaults_to_auto() {
        let input = [OsString::from("src/main.rs")];
        let args = parse_args(&input).unwrap();
        assert_eq!(args.pager, PagerMode::Auto);
        assert_eq!(args.color, ColorMode::Auto);
        assert_eq!(args.path, "src/main.rs");
    }

    #[test]
    fn parses_pager_aliases() {
        let input = [OsString::from("--less"), OsString::from("src/main.rs")];
        let args = parse_args(&input).unwrap();
        assert_eq!(args.pager, PagerMode::Always);
        assert_eq!(args.color, ColorMode::Auto);
        assert_eq!(args.path, "src/main.rs");

        let input = [
            OsString::from("--pager"),
            OsString::from("--plain"),
            OsString::from("-"),
        ];
        let args = parse_args(&input).unwrap();
        assert_eq!(args.pager, PagerMode::Always);
        assert_eq!(args.color, ColorMode::Never);
        assert_eq!(args.path, "-");
    }

    #[test]
    fn parses_no_pager_aliases() {
        let input = [OsString::from("--no-less"), OsString::from("src/main.rs")];
        let args = parse_args(&input).unwrap();
        assert_eq!(args.pager, PagerMode::Never);

        let input = [OsString::from("--stdout"), OsString::from("src/main.rs")];
        let args = parse_args(&input).unwrap();
        assert_eq!(args.pager, PagerMode::Never);
    }
}
