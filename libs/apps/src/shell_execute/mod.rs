mod platform;

pub use platform::shell_execute;

pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_args_quote_only_what_the_command_line_would_split() {
        let cases: [(&[&str], &str); 6] = [
            (&["exec", "shortcut", "s-1"], "exec shortcut s-1"),
            (&["open url"], r#""open url""#),
            (&[""], r#""""#),
            (&[r#"say "hi""#], r#""say \"hi\"""#),
            (&[r"C:\Program Files\"], r#""C:\Program Files\\""#),
            (&[r"C:\bin\tool.exe"], r"C:\bin\tool.exe"),
        ];
        for (args, expected) in cases {
            let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            assert_eq!(join_args(&args), expected, "{args:?}");
        }
    }
}
