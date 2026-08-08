use serde_json::{Map, Value};

use super::RowActionSpec;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRowAction {
    pub action: String,
    pub label: String,
    pub input: Value,
}

pub fn resolve_row_actions(
    row_action: Option<&RowActionSpec>,
    row_actions: &[RowActionSpec],
    row: &Value,
) -> Vec<ResolvedRowAction> {
    row_action
        .into_iter()
        .chain(row_actions)
        .filter(|spec| row_action_is_visible(spec, row))
        .map(|spec| ResolvedRowAction {
            action: spec.action.clone(),
            label: spec.label.clone().unwrap_or_else(|| "Run".into()),
            input: resolve_input(spec, row),
        })
        .collect()
}

fn row_action_is_visible(action: &RowActionSpec, row: &Value) -> bool {
    let Some(key) = action.when.as_deref() else {
        return true;
    };
    row.get(key).is_some_and(value_is_truthy)
}

fn value_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn resolve_input(action: &RowActionSpec, row: &Value) -> Value {
    let input = action
        .input
        .iter()
        .flatten()
        .map(|(key, template)| (key.clone(), interpolate_row_value(template, row)))
        .collect::<Map<_, _>>();
    Value::Object(input)
}

fn interpolate_row_value(template: &str, row: &Value) -> Value {
    if let Some(key) = exact_placeholder(template) {
        return row.get(key).cloned().unwrap_or(Value::Null);
    }
    Value::String(interpolate_row_template(template, row))
}

fn exact_placeholder(template: &str) -> Option<&str> {
    let key = template.strip_prefix('{')?.strip_suffix('}')?;
    placeholder_key(key).then_some(key)
}

fn placeholder_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn interpolate_row_template(template: &str, row: &Value) -> String {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        rendered.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('}') else {
            rendered.push_str(&rest[open..]);
            return rendered;
        };
        let key = &tail[..close];
        if !placeholder_key(key) {
            rendered.push('{');
            rest = tail;
            continue;
        }
        if let Some(value) = row.get(key) {
            rendered.push_str(&row_value_text(value));
        }
        rest = &tail[close + 1..];
    }
    rendered.push_str(rest);
    rendered
}

fn row_value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| match value {
            Value::Null => String::new(),
            value => value.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::{interpolate_row_template, resolve_row_actions, RowActionSpec};

    fn action(name: &str, label: Option<&str>, when: Option<&str>) -> RowActionSpec {
        RowActionSpec {
            action: name.into(),
            input: Some(IndexMap::from([
                ("address".into(), "{address}".into()),
                ("message".into(), "Connect {name} ({missing})".into()),
            ])),
            label: label.map(str::to_string),
            key: None,
            when: when.map(str::to_string),
        }
    }

    #[test]
    fn resolves_visible_actions_in_contract_order_with_typed_input() {
        let primary = action("inspect", None, None);
        let actions = [
            action("connect", Some("Connect"), Some("can_connect")),
            action("disconnect", Some("Disconnect"), Some("can_disconnect")),
        ];
        let row = serde_json::json!({
            "address": 42,
            "name": "Keyboard",
            "can_connect": true,
            "can_disconnect": false
        });

        let resolved = resolve_row_actions(Some(&primary), &actions, &row);

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].action, "inspect");
        assert_eq!(resolved[0].label, "Run");
        assert_eq!(resolved[1].action, "connect");
        assert_eq!(resolved[1].label, "Connect");
        assert_eq!(resolved[1].input["address"], serde_json::json!(42));
        assert_eq!(
            resolved[1].input["message"],
            serde_json::json!("Connect Keyboard ()")
        );
    }

    #[test]
    fn interpolate_row_template_matches_the_web_regex() {
        let row = serde_json::json!({
            "name": "WH-1000XM4",
            "index": 5,
            "paired": true,
            "nulled": null,
            "a": "A",
            "b": "B",
        });
        let cases = [
            ("{name}", "WH-1000XM4", "key present as string renders bare"),
            (
                "{index}",
                "5",
                "key present as number renders its JSON form",
            ),
            (
                "{paired}",
                "true",
                "key present as bool renders its JSON form",
            ),
            ("{nulled}", "", "key present as null renders nothing"),
            ("{missing}", "", "key absent entirely renders nothing"),
            (
                "{name} on {missing}",
                "WH-1000XM4 on ",
                "only the resolvable placeholder is replaced",
            ),
            (
                "{not a key}",
                "{not a key}",
                "a non-word brace expression is left verbatim",
            ),
            ("{}", "{}", "an empty brace pair is left verbatim"),
            (
                "plain text",
                "plain text",
                "a template without placeholders is unchanged",
            ),
            ("{abc", "{abc", "an unclosed brace is left verbatim"),
            (
                "{a{b}",
                "{aB",
                "a nested brace resolves only the inner placeholder",
            ),
            ("{a}}", "A}", "a trailing brace survives the replacement"),
            (
                "{ name }",
                "{ name }",
                "a spaced brace expression is left verbatim",
            ),
        ];
        for (template, expected, label) in cases {
            assert_eq!(
                interpolate_row_template(template, &row),
                expected,
                "{label}: template {template:?}"
            );
        }
    }

    #[test]
    fn when_gate_matches_row_value_truthiness() {
        let action = action("run", None, Some("enabled"));
        let cases = [
            (serde_json::json!(null), false),
            (serde_json::json!(false), false),
            (serde_json::json!(0), false),
            (serde_json::json!(""), false),
            (serde_json::json!(true), true),
            (serde_json::json!(1), true),
            (serde_json::json!("yes"), true),
            (serde_json::json!([]), true),
        ];
        for (enabled, expected) in cases {
            let row = serde_json::json!({ "enabled": enabled });
            assert_eq!(
                !resolve_row_actions(Some(&action), &[], &row).is_empty(),
                expected,
                "enabled: {enabled}"
            );
        }
    }
}
