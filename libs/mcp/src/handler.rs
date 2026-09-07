use crate::jsonrpc::{
    error_response, result_response, ErrorCode, LATEST_PROTOCOL_VERSION, PROTOCOL_VERSIONS,
};
use crate::tool::{ToolResult, ToolSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub agent_home: Option<String>,
}

pub trait ToolHost {
    fn server_info(&self) -> ServerInfo;
    fn list(&self) -> Vec<ToolSpec>;
    fn call(&self, name: &str, arguments: serde_json::Value, caller: &Caller) -> ToolResult;
}

pub fn handle(
    host: &dyn ToolHost,
    message: serde_json::Value,
    caller: &Caller,
) -> Option<serde_json::Value> {
    let object = match message {
        serde_json::Value::Object(object) => object,
        _ => {
            return Some(error_response(
                serde_json::Value::Null,
                ErrorCode::InvalidRequest,
                "expected a single JSON-RPC request object",
            ))
        }
    };

    let method = object
        .get("method")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let id = object.get("id").cloned();

    let Some(method) = method else {
        if object.contains_key("result") || object.contains_key("error") {
            return None;
        }
        return id.map(|id| error_response(id, ErrorCode::InvalidRequest, "missing method"));
    };

    let id = id?;

    let params = object
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match method.as_str() {
        "initialize" => Some(result_response(id, initialize_result(host, &params))),
        "ping" => Some(result_response(id, serde_json::json!({}))),
        "tools/list" => Some(result_response(
            id,
            serde_json::json!({"tools": host.list()}),
        )),
        "tools/call" => match call_tool(host, &params, caller) {
            Ok(result) => Some(result_response(id, result)),
            Err((code, message)) => Some(error_response(id, code, message)),
        },
        _ => Some(error_response(
            id,
            ErrorCode::MethodNotFound,
            format!("unknown method: {method}"),
        )),
    }
}

fn initialize_result(host: &dyn ToolHost, params: &serde_json::Value) -> serde_json::Value {
    let version = match params
        .get("protocolVersion")
        .and_then(serde_json::Value::as_str)
    {
        Some(requested) if PROTOCOL_VERSIONS.contains(&requested) => requested.to_string(),
        _ => LATEST_PROTOCOL_VERSION.to_string(),
    };
    let info = host.server_info();
    serde_json::json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": info.name, "version": info.version},
    })
}

fn call_tool(
    host: &dyn ToolHost,
    params: &serde_json::Value,
    caller: &Caller,
) -> Result<serde_json::Value, (ErrorCode, String)> {
    let Some(name) = params.get("name").and_then(serde_json::Value::as_str) else {
        return Err((ErrorCode::InvalidParams, "missing tool name".to_string()));
    };
    if !host.list().iter().any(|spec| spec.name == name) {
        return Err((ErrorCode::InvalidParams, format!("unknown tool: {name}")));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let result = host.call(name, arguments, caller);
    Ok(serde_json::to_value(&result).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeHost;

    impl ToolHost for FakeHost {
        fn server_info(&self) -> ServerInfo {
            ServerInfo {
                name: "test".to_string(),
                version: "1.2.3".to_string(),
            }
        }

        fn list(&self) -> Vec<ToolSpec> {
            vec![
                ToolSpec {
                    name: "echo".to_string(),
                    description: "echoes arguments".to_string(),
                    input_schema: json!({"type": "object"}),
                },
                ToolSpec {
                    name: "fail".to_string(),
                    description: "always fails".to_string(),
                    input_schema: json!({"type": "object"}),
                },
            ]
        }

        fn call(&self, name: &str, arguments: serde_json::Value, _caller: &Caller) -> ToolResult {
            match name {
                "echo" => ToolResult::structured(arguments),
                "fail" => ToolResult::error("boom"),
                _ => ToolResult::error("unknown tool"),
            }
        }
    }

    fn handle(host: &dyn ToolHost, message: serde_json::Value) -> Option<serde_json::Value> {
        super::handle(host, message, &Caller::default())
    }

    struct CallerHost {
        seen: std::sync::Mutex<Option<Caller>>,
    }

    impl ToolHost for CallerHost {
        fn server_info(&self) -> ServerInfo {
            ServerInfo {
                name: "caller-test".to_string(),
                version: "0.0.0".to_string(),
            }
        }

        fn list(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "echo".to_string(),
                description: "echoes arguments".to_string(),
                input_schema: json!({"type": "object"}),
            }]
        }

        fn call(&self, _name: &str, arguments: serde_json::Value, caller: &Caller) -> ToolResult {
            *self.seen.lock().unwrap() = Some(caller.clone());
            ToolResult::structured(arguments)
        }
    }

    fn request(
        id: impl Into<serde_json::Value>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        json!({"jsonrpc": "2.0", "id": id.into(), "method": method, "params": params})
    }

    #[test]
    fn batch_and_non_object_messages_are_invalid_requests() {
        for message in [json!([]), json!("hi"), json!(1), json!(null), json!(true)] {
            let response = handle(&FakeHost, message).expect("response");
            assert_eq!(response["error"]["code"], -32600);
            assert_eq!(response["id"], serde_json::Value::Null);
        }
    }

    #[test]
    fn client_response_with_result_or_error_is_ignored() {
        assert_eq!(
            handle(&FakeHost, json!({"jsonrpc": "2.0", "id": 1, "result": {}})),
            None
        );
        assert_eq!(
            handle(
                &FakeHost,
                json!({"jsonrpc": "2.0", "id": 1, "error": {"code": 1, "message": "x"}})
            ),
            None
        );
    }

    #[test]
    fn object_without_method_and_id_is_ignored() {
        assert_eq!(handle(&FakeHost, json!({"jsonrpc": "2.0"})), None);
    }

    #[test]
    fn object_without_method_but_with_id_is_invalid_request() {
        let response = handle(&FakeHost, json!({"jsonrpc": "2.0", "id": 7})).expect("response");
        assert_eq!(response["id"], 7);
        assert_eq!(response["error"]["code"], -32600);
    }

    #[test]
    fn notifications_are_ignored_for_every_method() {
        assert_eq!(
            handle(
                &FakeHost,
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            ),
            None
        );
        assert_eq!(
            handle(
                &FakeHost,
                json!({"jsonrpc": "2.0", "method": "unknown/notification"})
            ),
            None
        );
    }

    #[test]
    fn initialize_echoes_a_supported_protocol_version() {
        let response = handle(
            &FakeHost,
            request(1, "initialize", json!({"protocolVersion": "2024-11-05"})),
        )
        .expect("response");
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn initialize_falls_back_to_latest_protocol_version() {
        for params in [
            json!({"protocolVersion": "1999-01-01"}),
            json!({}),
            serde_json::Value::Null,
        ] {
            let response = handle(&FakeHost, request(1, "initialize", params)).expect("response");
            assert_eq!(
                response["result"]["protocolVersion"],
                LATEST_PROTOCOL_VERSION
            );
        }
    }

    #[test]
    fn initialize_result_carries_capabilities_and_server_info() {
        let response = handle(
            &FakeHost,
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
        )
        .expect("response");
        assert_eq!(
            response["result"]["capabilities"],
            json!({"tools": {"listChanged": false}})
        );
        assert_eq!(
            response["result"]["serverInfo"],
            json!({"name": "test", "version": "1.2.3"})
        );
    }

    #[test]
    fn ping_returns_empty_result() {
        let response = handle(&FakeHost, request(2, "ping", json!({}))).expect("response");
        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn tools_list_returns_host_specs() {
        let response = handle(&FakeHost, request(3, "tools/list", json!({}))).expect("response");
        let tools = response["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "echo");
        assert_eq!(tools[0]["inputSchema"], json!({"type": "object"}));
        assert_eq!(tools[1]["name"], "fail");
    }

    #[test]
    fn tools_call_without_name_is_invalid_params() {
        let response = handle(&FakeHost, request(4, "tools/call", json!({}))).expect("response");
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_with_non_string_name_is_invalid_params() {
        let response =
            handle(&FakeHost, request(4, "tools/call", json!({"name": 5}))).expect("response");
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_with_unknown_name_is_invalid_params_with_message() {
        let response =
            handle(&FakeHost, request(4, "tools/call", json!({"name": "nope"}))).expect("response");
        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(response["error"]["message"], "unknown tool: nope");
    }

    #[test]
    fn tools_call_echo_returns_structured_result() {
        let response = handle(
            &FakeHost,
            request(
                4,
                "tools/call",
                json!({"name": "echo", "arguments": {"x": 1}}),
            ),
        )
        .expect("response");
        assert_eq!(response["result"]["structuredContent"], json!({"x": 1}));
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn tools_call_defaults_arguments_to_empty_object() {
        let response =
            handle(&FakeHost, request(4, "tools/call", json!({"name": "echo"}))).expect("response");
        assert_eq!(response["result"]["structuredContent"], json!({}));
    }

    #[test]
    fn tools_call_fail_returns_error_tool_result() {
        let response =
            handle(&FakeHost, request(4, "tools/call", json!({"name": "fail"}))).expect("response");
        assert_eq!(response["result"]["content"][0]["text"], "boom");
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn tools_call_passes_the_caller_to_the_host() {
        let host = CallerHost {
            seen: std::sync::Mutex::new(None),
        };
        super::handle(
            &host,
            request(4, "tools/call", json!({"name": "echo"})),
            &Caller {
                agent_home: Some("/home/k/.claude-work".to_owned()),
            },
        )
        .expect("response");
        assert_eq!(
            host.seen
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|caller| caller.agent_home.as_deref()),
            Some("/home/k/.claude-work")
        );
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let response =
            handle(&FakeHost, request(5, "resources/list", json!({}))).expect("response");
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn round_trip_initialize_initialized_list_and_call() {
        let initialize = handle(
            &FakeHost,
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
        )
        .expect("response");
        assert_eq!(
            initialize,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "test", "version": "1.2.3"},
                },
            })
        );

        let initialized = handle(
            &FakeHost,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        );
        assert_eq!(initialized, None);

        let list = handle(&FakeHost, request(2, "tools/list", json!({}))).expect("response");
        assert_eq!(
            list,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "tools": [
                        {"name": "echo", "description": "echoes arguments", "inputSchema": {"type": "object"}},
                        {"name": "fail", "description": "always fails", "inputSchema": {"type": "object"}},
                    ],
                },
            })
        );

        let call = handle(
            &FakeHost,
            request(
                3,
                "tools/call",
                json!({"name": "echo", "arguments": {"hello": "world"}}),
            ),
        )
        .expect("response");
        assert_eq!(
            call,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": {
                    "content": [{"type": "text", "text": "{\n  \"hello\": \"world\"\n}"}],
                    "structuredContent": {"hello": "world"},
                    "isError": false,
                },
            })
        );
    }
}
