pub const PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
}

impl ErrorCode {
    pub fn code(self) -> i64 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
        }
    }
}

pub fn result_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub fn error_response(
    id: serde_json::Value,
    code: ErrorCode,
    message: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code.code(), "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_code_values_match_json_rpc() {
        assert_eq!(ErrorCode::ParseError.code(), -32700);
        assert_eq!(ErrorCode::InvalidRequest.code(), -32600);
        assert_eq!(ErrorCode::MethodNotFound.code(), -32601);
        assert_eq!(ErrorCode::InvalidParams.code(), -32602);
        assert_eq!(ErrorCode::InternalError.code(), -32603);
    }

    #[test]
    fn result_response_shape() {
        let response = result_response(json!(1), json!({"ok": true}));
        assert_eq!(
            response,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}})
        );
    }

    #[test]
    fn error_response_shape() {
        let response = error_response(serde_json::Value::Null, ErrorCode::ParseError, "bad json");
        assert_eq!(
            response,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "bad json"}})
        );
    }

    #[test]
    fn protocol_version_constants() {
        assert_eq!(LATEST_PROTOCOL_VERSION, "2025-06-18");
        assert_eq!(
            PROTOCOL_VERSIONS,
            ["2024-11-05", "2025-03-26", "2025-06-18"]
        );
        assert!(PROTOCOL_VERSIONS.contains(&LATEST_PROTOCOL_VERSION));
    }
}
