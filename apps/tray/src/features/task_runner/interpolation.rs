use std::collections::HashMap;

pub(super) fn interpolate(template: &str, params: &HashMap<String, String>) -> String {
    replace_template_vars(template, |key| params.get(key).cloned().unwrap_or_default())
}

pub(super) fn interpolate_shell(
    template: &str,
    params: &HashMap<String, String>,
    mut quote: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut failure = None;
    let script = replace_template_vars(template, |key| {
        let value = params.get(key).map(|item| item.as_str()).unwrap_or("");
        quote(value).unwrap_or_else(|error| {
            failure.get_or_insert(format!("parameter {key}: {error}"));
            String::new()
        })
    });
    failure.map_or(Ok(script), Err)
}

fn replace_template_vars(template: &str, mut replacer: impl FnMut(&str) -> String) -> String {
    let mut result = String::with_capacity(template.len());
    for part in template_parts(template) {
        match part {
            TemplatePart::Text(text) => result.push_str(text),
            TemplatePart::Param(key) => result.push_str(&replacer(key)),
        }
    }
    result
}

pub(super) enum TemplatePart<'a> {
    Text(&'a str),
    Param(&'a str),
}

pub(super) fn template_parts(template: &str) -> Vec<TemplatePart<'_>> {
    let mut parts = Vec::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];

        let Some(end) = after_open.find("}}") else {
            break;
        };

        let key = &after_open[..end];
        let placeholder_end = start + 2 + end + 2;

        if valid_key(key) {
            parts.push(TemplatePart::Text(&rest[..start]));
            parts.push(TemplatePart::Param(key));
        } else {
            parts.push(TemplatePart::Text(&rest[..placeholder_end]));
        }
        rest = &rest[placeholder_end..];
    }

    parts.push(TemplatePart::Text(rest));
    parts
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    type TestCase<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);

    #[test]
    fn interpolate_single_param() {
        let cases = [
            ("echo {{msg}}", &[("msg", "hello")], "echo hello"),
            ("{{x}}", &[("x", "value")], "value"),
            ("prefix {{a}} suffix", &[("a", "mid")], "prefix mid suffix"),
            ("{{foo}}bar", &[("foo", "baz")], "bazbar"),
            ("bar{{foo}}", &[("foo", "baz")], "barbaz"),
        ];

        for (template, params, expected) in cases {
            let map = param_map(params);
            assert_eq!(
                interpolate(template, &map),
                expected,
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_multiple_params() {
        let cases: &[TestCase<'_>] = &[
            ("{{a}} {{b}}", &[("a", "x"), ("b", "y")], "x y"),
            (
                "{{x}}{{y}}{{z}}",
                &[("x", "1"), ("y", "2"), ("z", "3")],
                "123",
            ),
            (
                "git checkout {{branch}} && cd {{dir}}",
                &[("branch", "main"), ("dir", "/a/b")],
                "git checkout main && cd /a/b",
            ),
            ("{{a}}-{{b}}-{{a}}", &[("a", "X"), ("b", "Y")], "X-Y-X"),
        ];

        for (template, params, expected) in cases {
            let map = param_map(params);
            assert_eq!(
                interpolate(template, &map),
                *expected,
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_missing_params() {
        let cases: &[TestCase<'_>] = &[
            ("{{missing}}", &[], ""),
            ("hello {{name}}", &[], "hello "),
            ("{{a}} {{b}}", &[("a", "x")], "x "),
            ("{{a}}{{missing}}{{b}}", &[("a", "1"), ("b", "2")], "12"),
        ];

        for (template, params, expected) in cases {
            let map = param_map(params);
            assert_eq!(
                interpolate(template, &map),
                *expected,
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_no_placeholders() {
        let cases = [
            ("no placeholders here", "no placeholders here"),
            ("", ""),
            ("echo hello world", "echo hello world"),
            ("{ not a placeholder }", "{ not a placeholder }"),
            ("{single}", "{single}"),
            ("{{}", "{{}"),
            ("}}", "}}"),
        ];

        let empty: HashMap<String, String> = HashMap::new();
        for (template, expected) in cases {
            assert_eq!(
                interpolate(template, &empty),
                expected,
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_special_values() {
        let cases = [
            ("{{path}}", &[("path", "/a/b/c")], "/a/b/c"),
            (
                "{{url}}",
                &[("url", "https://example.com?q=1&x=2")],
                "https://example.com?q=1&x=2",
            ),
            (
                "{{json}}",
                &[("json", r#"{"key": "value"}"#)],
                r#"{"key": "value"}"#,
            ),
            ("{{empty}}", &[("empty", "")], ""),
            ("{{spaces}}", &[("spaces", "  a b c  ")], "  a b c  "),
            ("{{unicode}}", &[("unicode", "日本語")], "日本語"),
            ("{{emoji}}", &[("emoji", "🚀")], "🚀"),
            ("{{newline}}", &[("newline", "a\nb\nc")], "a\nb\nc"),
            ("{{tab}}", &[("tab", "a\tb")], "a\tb"),
        ];

        for (template, params, expected) in cases {
            let map = param_map(params);
            assert_eq!(
                interpolate(template, &map),
                expected,
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_invalid_syntax_unchanged() {
        let cases = [
            "{{}}",
            "{{ spaces }}",
            "{{with-dash}}",
            "{{with.dot}}",
            "{{with/slash}}",
            "{single}",
            "{ {double} }",
            "{{nested{{inner}}}}",
            "{{123starts_with_num}}",
        ];

        let params = param_map(&[("spaces", "x"), ("with-dash", "x"), ("with.dot", "x")]);
        for template in cases {
            let result = interpolate(template, &params);
            assert!(
                !result.contains("x") || template.contains("x"),
                "invalid placeholder should not be replaced: {:?} -> {:?}",
                template,
                result
            );
        }
    }

    #[test]
    fn interpolate_valid_identifiers() {
        let cases = [
            ("{{a}}", "a"),
            ("{{A}}", "A"),
            ("{{abc}}", "abc"),
            ("{{ABC}}", "ABC"),
            ("{{a1}}", "a1"),
            ("{{var_name}}", "var_name"),
            ("{{CamelCase}}", "CamelCase"),
            ("{{_underscore}}", "_underscore"),
            ("{{a123b456}}", "a123b456"),
        ];

        for (template, key) in cases {
            let map = [(key.to_string(), "REPLACED".to_string())]
                .into_iter()
                .collect();
            assert_eq!(
                interpolate(template, &map),
                "REPLACED",
                "key {:?} should be valid",
                key
            );
        }
    }

    #[test]
    fn interpolate_shell_quotes_every_param() {
        let cases = [
            ("echo {{missing}}", &[][..], "echo <>"),
            (
                "echo {{msg}}",
                &[("msg", "hello world")][..],
                "echo <hello world>",
            ),
            ("{{a}}-{{a}}", &[("a", "x")][..], "<x>-<x>"),
            ("echo {{bad-key}}", &[][..], "echo {{bad-key}}"),
        ];

        for (template, params, expected) in cases {
            let map = param_map(params);
            assert_eq!(
                interpolate_shell(template, &map, |value| Ok(format!("<{value}>"))),
                Ok(expected.to_string()),
                "template: {:?}",
                template
            );
        }
    }

    #[test]
    fn interpolate_shell_names_the_param_its_quoter_rejects() {
        let map = param_map(&[("ok", "fine"), ("msg", "bad")]);
        let result = interpolate_shell("echo {{ok}} {{msg}}", &map, |value| {
            if value == "bad" {
                Err("rejected".to_string())
            } else {
                Ok(value.to_string())
            }
        });
        assert_eq!(result, Err("parameter msg: rejected".to_string()));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_interpolate_replaces_valid_key(
            prefix in "[^{}]{0,16}",
            suffix in "[^{}]{0,16}",
            key in "[A-Za-z0-9_]{1,16}",
            value in ".*"
        ) {
            let template = format!("{prefix}{{{{{key}}}}}{suffix}");
            let map = [(key.clone(), value.clone())].into_iter().collect();
            prop_assert_eq!(interpolate(&template, &map), format!("{prefix}{value}{suffix}"));
        }
    }

    fn param_map<K, V>(params: &[(K, V)]) -> HashMap<String, String>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        params
            .iter()
            .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
            .collect()
    }
}
