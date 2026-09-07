#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text { text: text.into() }],
            structured: None,
            is_error: false,
        }
    }

    pub fn structured(value: serde_json::Value) -> Self {
        let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
        Self {
            content: vec![Content::Text { text: pretty }],
            structured: Some(value),
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text {
                text: message.into(),
            }],
            structured: None,
            is_error: true,
        }
    }
}

pub fn input_schema(params: &indexmap::IndexMap<String, String>) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, description) in params {
        properties.insert(
            name.clone(),
            serde_json::json!({"type": "string", "description": description}),
        );
        required.push(serde_json::Value::String(name.clone()));
    }
    serde_json::json!({"type": "object", "properties": properties, "required": required})
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use serde_json::json;

    #[test]
    fn tool_spec_serializes_input_schema_key() {
        let spec = ToolSpec {
            name: "echo".to_string(),
            description: "echoes arguments".to_string(),
            input_schema: json!({"type": "object"}),
        };
        let value = serde_json::to_value(&spec).expect("spec serializes");
        assert_eq!(value["name"], "echo");
        assert_eq!(value["description"], "echoes arguments");
        assert_eq!(value["inputSchema"], json!({"type": "object"}));
        assert!(value.get("input_schema").is_none());
    }

    #[test]
    fn tool_result_omits_structured_content_when_none() {
        let value = serde_json::to_value(ToolResult::text("hello")).expect("result serializes");
        assert_eq!(value["content"], json!([{"type": "text", "text": "hello"}]));
        assert_eq!(value["isError"], false);
        assert!(value.get("structuredContent").is_none());
    }

    #[test]
    fn tool_result_includes_structured_content_and_always_has_is_error() {
        let value = serde_json::to_value(ToolResult::structured(json!({"a": 1})))
            .expect("result serializes");
        assert_eq!(value["structuredContent"], json!({"a": 1}));
        assert_eq!(
            value["content"],
            json!([{"type": "text", "text": "{\n  \"a\": 1\n}"}])
        );
        assert_eq!(value["isError"], false);
        let error_value =
            serde_json::to_value(ToolResult::error("boom")).expect("result serializes");
        assert_eq!(error_value["isError"], true);
        assert!(error_value.get("structuredContent").is_none());
    }

    #[test]
    fn text_and_error_helpers_match_expected_fields() {
        assert_eq!(
            ToolResult::text("hi"),
            ToolResult {
                content: vec![Content::Text {
                    text: "hi".to_string()
                }],
                structured: None,
                is_error: false,
            }
        );
        assert_eq!(
            ToolResult::error("bad"),
            ToolResult {
                content: vec![Content::Text {
                    text: "bad".to_string()
                }],
                structured: None,
                is_error: true,
            }
        );
    }

    #[test]
    fn input_schema_with_two_parameters_preserves_map_order() {
        let mut params = IndexMap::new();
        params.insert("name".to_string(), "person name".to_string());
        params.insert("greeting".to_string(), "greeting text".to_string());
        assert_eq!(
            input_schema(&params),
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "person name"},
                    "greeting": {"type": "string", "description": "greeting text"},
                },
                "required": ["name", "greeting"],
            })
        );
    }

    #[test]
    fn input_schema_with_zero_parameters_is_empty_object_schema() {
        assert_eq!(
            input_schema(&IndexMap::new()),
            json!({"type": "object", "properties": {}, "required": []})
        );
    }
}
