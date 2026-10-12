use std::collections::HashMap;

use super::super::interpolation::{interpolate_shell, template_parts, TemplatePart};

const CMD_METACHARACTERS: &[char] = &['^', '&', '|', '<', '>', '(', ')', '"', '!'];
const REPARSED_METACHARACTERS: &[char] = &['&', '|', '<', '>', '^', '%', '!', '"', '(', ')'];
const REPARSED_REFUSAL: &str = "cmd parses a pipeline, call or for /f command twice, so this parameter cannot contain & | < > ^ % ! \" ( )";
const QUOTED_REFUSAL: &str = "a parameter inside double quotes cannot contain a double quote";

#[derive(Clone, Copy)]
struct Slot {
    quoted: bool,
    reparsed: bool,
}

pub(super) fn render(template: &str, params: &HashMap<String, String>) -> Result<String, String> {
    let mut renderings = render_params(template, params).into_iter();
    interpolate_shell(template, params, |_| {
        renderings.next().unwrap_or_else(|| Ok(String::new()))
    })
}

fn render_params(template: &str, params: &HashMap<String, String>) -> Vec<Result<String, String>> {
    let parts = template_parts(template);
    let mut slots = scan_slots(&parts);
    let mut renderings = Vec::with_capacity(slots.len());
    let mut following = None;
    for part in parts.iter().rev() {
        let rendering = match part {
            TemplatePart::Text(text) => Ok(text.to_string()),
            TemplatePart::Param(key) => {
                let value = params.get(*key).map(|item| item.as_str()).unwrap_or("");
                let slot = slots.pop().unwrap_or(Slot {
                    quoted: false,
                    reparsed: true,
                });
                let rendering = render_param(value, slot, following);
                renderings.push(rendering.clone());
                rendering
            }
        };
        if let Some(first) = rendering.ok().and_then(|text| text.chars().next()) {
            following = Some(first);
        }
    }
    renderings.reverse();
    renderings
}

fn render_param(value: &str, slot: Slot, following: Option<char>) -> Result<String, String> {
    if value.contains(['\r', '\n']) {
        return Err("cmd cannot pass a line break".to_string());
    }
    if slot.reparsed && value.contains(REPARSED_METACHARACTERS) {
        return Err(REPARSED_REFUSAL.to_string());
    }
    if slot.quoted {
        if value.contains('"') {
            return Err(QUOTED_REFUSAL.to_string());
        }
        return Ok(escape_in_quotes(value, following));
    }
    if slot.reparsed {
        return Ok(quote_for_argv(value));
    }
    Ok(escape_for_cmd(&quote_for_argv(value), following))
}

fn scan_slots(parts: &[TemplatePart<'_>]) -> Vec<Slot> {
    let mut line = Vec::new();
    let mut params = Vec::new();
    for part in parts {
        match part {
            TemplatePart::Text(text) => line.extend(text.chars()),
            TemplatePart::Param(_) => {
                params.push(line.len());
                line.push('x');
            }
        }
    }
    let (quoted, reparsed) = scan_line(&line);
    params
        .into_iter()
        .map(|index| Slot {
            quoted: quoted[index],
            reparsed,
        })
        .collect()
}

fn scan_line(line: &[char]) -> (Vec<bool>, bool) {
    let mut quoted = vec![false; line.len()];
    let mut command_starts = vec![0];
    let mut reparsed = false;
    let mut in_quotes = false;
    let mut index = 0;
    while index < line.len() {
        quoted[index] = in_quotes;
        let character = line[index];
        index += 1;
        if in_quotes {
            in_quotes = character != '"';
            continue;
        }
        match character {
            '"' => in_quotes = true,
            '^' => index += 1,
            '&' | '(' => command_starts.push(index),
            '|' if line.get(index) == Some(&'|') => {
                index += 1;
                command_starts.push(index);
            }
            '|' => {
                reparsed = true;
                command_starts.push(index);
            }
            _ => {}
        }
    }
    reparsed |= command_starts
        .into_iter()
        .any(|start| reparsing_command(&line[start.min(line.len())..]));
    (quoted, reparsed)
}

fn reparsing_command(command: &[char]) -> bool {
    let command: String = command.iter().collect();
    let command =
        command.trim_start_matches(|character: char| character.is_whitespace() || character == '@');
    let mut words = command.split(|character: char| {
        character.is_whitespace() || CMD_METACHARACTERS.contains(&character)
    });
    match words.next() {
        Some(word) if word.eq_ignore_ascii_case("call") => true,
        Some(word) if word.eq_ignore_ascii_case("for") => words
            .find(|word| !word.is_empty())
            .is_some_and(|word| word.eq_ignore_ascii_case("/f")),
        _ => false,
    }
}

fn quote_for_argv(value: &str) -> String {
    if !value.is_empty() && !value.contains([' ', '\t', '"']) {
        return value.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        let escapes = if character == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        quoted.push_str(&"\\".repeat(escapes));
        quoted.push(character);
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn escape_in_quotes(value: &str, following: Option<char>) -> String {
    let mut escaped = String::with_capacity(value.len() * 2);
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '%' {
            escaped.push_str(&"\\".repeat(backslashes * 2));
            escaped.push_str("\"^%\"");
        } else {
            escaped.push_str(&"\\".repeat(backslashes));
            escaped.push(character);
        }
        backslashes = 0;
    }
    let trailing = if following == Some('"') {
        backslashes * 2
    } else {
        backslashes
    };
    escaped.push_str(&"\\".repeat(trailing));
    escaped
}

fn escape_for_cmd(argument: &str, following: Option<char>) -> String {
    let mut escaped = String::with_capacity(argument.len() * 2);
    let mut characters = argument.chars().peekable();
    while let Some(character) = characters.next() {
        if CMD_METACHARACTERS.contains(&character) {
            escaped.push('^');
        }
        escaped.push(character);
        if character == '%'
            && characters
                .peek()
                .copied()
                .or(following)
                .is_some_and(|next| {
                    !CMD_METACHARACTERS.contains(&next) && !matches!(next, '\r' | '\n')
                })
        {
            escaped.push('^');
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    type Params<'a> = &'a [(&'a str, &'a str)];

    fn render_with(template: &str, params: &[(&str, &str)]) -> Result<String, String> {
        let params = params
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        render(template, &params)
    }

    #[test]
    fn render_escapes_special_values() {
        let cases = [
            ("", "^\"^\""),
            ("safe_value-123", "safe_value-123"),
            ("hello world", "^\"hello world^\""),
            (
                "https://example.com?q=1&x=2",
                "https://example.com?q=1^&x=2",
            ),
            ("x & echo INJECTED &", "^\"x ^& echo INJECTED ^&^\""),
            ("say \"hi\"", "^\"say \\^\"hi\\^\"^\""),
            (r"C:\my dir\", r#"^"C:\my dir\\^""#),
            (r"C:\dir\", r"C:\dir\"),
            ("100%", "100%"),
            ("%PATH%", "%^PATH%"),
            ("%PATH:~0,1%", "%^PATH:~0,1%"),
            ("%%", "%^%"),
            ("%&", "%^&"),
            ("a^b", "a^^b"),
            ("(a|b)<c>!", "^(a^|b^)^<c^>^!"),
        ];

        for (value, expected) in cases {
            assert_eq!(
                render_with("{{v}}", &[("v", value)]).as_deref(),
                Ok(expected),
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn render_rejects_line_breaks() {
        for value in ["a\nb", "a\r\nb", "trailing\r"] {
            assert!(
                render_with("echo {{v}}", &[("v", value)]).is_err(),
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn render_never_joins_a_trailing_percent_with_what_follows() {
        let cases: &[(&str, Params, &str)] = &[
            (
                "echo x{{a}}{{b}}",
                &[("a", "%"), ("b", "PATH%")],
                "echo x%^PATH%",
            ),
            ("echo {{a}}PATH%", &[("a", "%")], "echo %^PATH%"),
            ("echo {{a}}&echo y", &[("a", "50%")], "echo 50%&echo y"),
            ("echo {{a}}{{b}}", &[("a", "%"), ("b", "&")], "echo %^&"),
            ("echo {{a}}", &[("a", "100%")], "echo 100%"),
        ];

        for (template, params, expected) in cases {
            let rendered = render_with(template, params);
            assert_eq!(rendered.as_deref(), Ok(*expected), "template: {template:?}");
        }
    }

    #[test]
    fn render_refuses_metacharacters_where_cmd_parses_twice() {
        let templates = [
            "type f | findstr {{q}}",
            "{{q}} | more",
            "call echo {{q}}",
            "@CALL echo {{q}}",
            "echo a & call echo {{q}}",
            "for /f %i in ('echo {{q}}') do @echo %i",
            "FOR  /F %i in ('echo x') do @echo {{q}}",
            "type f | findstr \"{{q}}\"",
        ];

        for template in templates {
            for value in ["x&calc", "50%", "a^b", "say \"hi\"", "(x)", "a!b"] {
                let rendered = render_with(template, &[("q", value)]);
                assert!(
                    rendered
                        .as_ref()
                        .is_err_and(|error| error.starts_with("parameter q: ")),
                    "template: {template:?}, value: {value:?}, rendered: {rendered:?}"
                );
            }
            assert_eq!(
                render_with(template, &[("q", "plain")]),
                Ok(template.replace("{{q}}", "plain")),
                "template: {template:?}"
            );
        }
    }

    #[test]
    fn render_quotes_spaced_values_where_cmd_parses_twice() {
        assert_eq!(
            render_with("type f | findstr {{q}}", &[("q", r"a b\")]).as_deref(),
            Ok(r#"type f | findstr "a b\\""#)
        );
    }

    #[test]
    fn render_parses_once_without_a_pipeline_call_or_for_f() {
        let cases = [
            ("git pull || echo {{q}}", "git pull || echo x^&calc"),
            ("echo a^|b {{q}}", "echo a^|b x^&calc"),
            ("echo \"a|b\" {{q}}", "echo \"a|b\" x^&calc"),
            ("echo call {{q}}", "echo call x^&calc"),
            (
                "for %i in (a) do echo {{q}}",
                "for %i in (a) do echo x^&calc",
            ),
            ("recall {{q}}", "recall x^&calc"),
        ];

        for (template, expected) in cases {
            assert_eq!(
                render_with(template, &[("q", "x&calc")]).as_deref(),
                Ok(expected),
                "template: {template:?}"
            );
        }
    }

    #[test]
    fn render_inside_template_quotes_adds_no_carets() {
        let cases = [
            (
                "git commit -m \"{{msg}}\"",
                "50% R&D",
                "git commit -m \"50\"^%\" R&D\"",
            ),
            ("echo \"{{msg}}\"", "a&b|c^d!(e)", "echo \"a&b|c^d!(e)\""),
            ("echo \"{{msg}}\"", "%PATH%", "echo \"\"^%\"PATH\"^%\"\""),
            ("dir \"{{msg}}\"", r"C:\dir\", r#"dir "C:\dir\\""#),
            ("dir \"{{msg}}\\x\"", r"C:\dir\", r#"dir "C:\dir\\x""#),
            ("dir \"{{msg}}\"", r"a\%", r#"dir "a\\"^%"""#),
        ];

        for (template, value, expected) in cases {
            assert_eq!(
                render_with(template, &[("msg", value)]).as_deref(),
                Ok(expected),
                "template: {template:?}, value: {value:?}"
            );
        }
    }

    #[test]
    fn render_refuses_a_double_quote_inside_template_quotes() {
        assert_eq!(
            render_with("echo \"{{msg}}\"", &[("msg", "say \"hi\"")]),
            Err(format!("parameter msg: {QUOTED_REFUSAL}"))
        );
    }

    #[test]
    fn render_names_the_first_refused_parameter() {
        assert_eq!(
            render_with(
                "type f | findstr {{a}} {{b}}",
                &[("a", "x&y"), ("b", "y|z")]
            ),
            Err(format!("parameter a: {REPARSED_REFUSAL}"))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn render_keeps_split_variable_references_literal_in_cmd() {
        let script =
            render_with("echo x{{a}}{{b}}", &[("a", "%"), ("b", "PATH%")]).expect("renderable");
        let output = super::super::shell_command(&script)
            .expect("cmd command")
            .output()
            .expect("cmd must run");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "x%PATH%\r\n");
    }

    #[cfg(target_os = "windows")]
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

        #[test]
        fn prop_render_reaches_the_program_as_one_argv_quoted_word(
            value in "[\x20-\x7e]{0,40}",
        ) {
            let script = render_with("echo x{{v}}", &[("v", value.as_str())])
                .expect("printable ASCII is quotable");
            let output = super::super::shell_command(&script)
                .expect("cmd command")
                .output()
                .expect("cmd must run");
            let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
            proptest::prop_assert_eq!(
                stdout,
                format!("x{}\r\n", quote_for_argv(&value)),
                "script={:?}",
                script,
            );
        }
    }
}
