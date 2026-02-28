use super::DaemonActionDispatch;
use qol_runtime::protocol::DaemonResponse;

pub(super) fn parse_response(line: &str) -> DaemonActionDispatch {
    if let Ok(resp) = serde_json::from_str::<DaemonResponse>(line) {
        return match resp {
            DaemonResponse::Handled { .. } => DaemonActionDispatch::Handled,
            DaemonResponse::Fallback => DaemonActionDispatch::Fallback,
            DaemonResponse::Error { message } => DaemonActionDispatch::Error(message),
        };
    }

    let word = line.split_whitespace().next().unwrap_or("");
    match word {
        "handled" => DaemonActionDispatch::Handled,
        "fallback" => DaemonActionDispatch::Fallback,
        "error" => DaemonActionDispatch::Error(
            line.strip_prefix("error").unwrap_or("").trim().to_string(),
        ),
        _ => DaemonActionDispatch::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_cases() {
        let cases = [
            (r#"{"status":"handled"}"#, DaemonActionDispatch::Handled),
            (
                r#"{"status":"handled","data":{"key":"val"}}"#,
                DaemonActionDispatch::Handled,
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
            ("handled", DaemonActionDispatch::Handled),
            ("fallback", DaemonActionDispatch::Fallback),
            ("error something broke", DaemonActionDispatch::Error("something broke".to_string())),
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
}
