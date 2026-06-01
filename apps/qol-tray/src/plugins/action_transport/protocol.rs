use super::DaemonActionDispatch;
use qol_runtime::protocol::DaemonResponse;

pub(super) fn parse_response(line: &str) -> DaemonActionDispatch {
    if let Ok(resp) = serde_json::from_str::<DaemonResponse>(line) {
        return match resp {
            DaemonResponse::Handled { data } => DaemonActionDispatch::Handled { payload: data },
            DaemonResponse::Fallback => DaemonActionDispatch::Fallback,
            DaemonResponse::Error { message } => DaemonActionDispatch::Error(message),
        };
    }

    let word = line.split_whitespace().next().unwrap_or("");
    match word {
        "handled" => DaemonActionDispatch::Handled { payload: None },
        "fallback" => DaemonActionDispatch::Fallback,
        "error" => {
            DaemonActionDispatch::Error(line.strip_prefix("error").unwrap_or("").trim().to_string())
        }
        _ => DaemonActionDispatch::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_cases() {
        let cases = [
            (
                r#"{"status":"handled"}"#,
                DaemonActionDispatch::Handled { payload: None },
            ),
            (r#"{"status":"fallback"}"#, DaemonActionDispatch::Fallback),
            (
                r#"{"status":"error","message":"daemon busy"}"#,
                DaemonActionDispatch::Error("daemon busy".to_string()),
            ),
            (
                r#"{"status":"error","message":""}"#,
                DaemonActionDispatch::Error(String::new()),
            ),
            ("handled", DaemonActionDispatch::Handled { payload: None }),
            ("fallback", DaemonActionDispatch::Fallback),
            (
                "error something broke",
                DaemonActionDispatch::Error("something broke".to_string()),
            ),
            ("", DaemonActionDispatch::Unavailable),
            ("garbage", DaemonActionDispatch::Unavailable),
        ];

        for (input, expected) in cases {
            let got = parse_response(input);
            assert_eq!(
                std::mem::discriminant(&got),
                std::mem::discriminant(&expected),
                "input: {:?}",
                input
            );
        }
    }

    #[test]
    fn parse_response_extracts_payload() {
        let input = r#"{"status":"handled","data":{"devices":[{"ieee":"0x123","online":true}]}}"#;
        let got = parse_response(input);
        match got {
            DaemonActionDispatch::Handled {
                payload: Some(value),
            } => {
                assert_eq!(
                    value,
                    serde_json::json!({"devices":[{"ieee":"0x123","online":true}]}),
                    "payload should carry JSON data"
                );
            }
            other => panic!("expected Handled with payload, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_handles_no_payload() {
        let input = r#"{"status":"handled"}"#;
        let got = parse_response(input);
        assert_eq!(got, DaemonActionDispatch::Handled { payload: None });
    }
}
