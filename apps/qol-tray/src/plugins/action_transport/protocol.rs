use super::DaemonActionDispatch;

pub(super) fn parse_response(bytes: &[u8]) -> DaemonActionDispatch {
    let raw = match std::str::from_utf8(bytes) {
        Ok(value) => value.trim(),
        Err(_) => return DaemonActionDispatch::Unavailable,
    };

    if raw.is_empty() {
        return DaemonActionDispatch::Unavailable;
    }

    let (status, payload) = match raw.split_once(' ') {
        Some((status, payload)) => (status.to_ascii_lowercase(), payload.trim()),
        None => (raw.to_ascii_lowercase(), ""),
    };

    match status.as_str() {
        "handled" => DaemonActionDispatch::Handled,
        "fallback" => DaemonActionDispatch::Fallback,
        "error" => DaemonActionDispatch::Error(payload.to_string()),
        _ => DaemonActionDispatch::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_cases() {
        let cases = [
            (b"handled".as_slice(), DaemonActionDispatch::Handled),
            (b"fallback\n".as_slice(), DaemonActionDispatch::Fallback),
            (
                b"error daemon busy\n".as_slice(),
                DaemonActionDispatch::Error("daemon busy".to_string()),
            ),
            (
                b"error\n".as_slice(),
                DaemonActionDispatch::Error(String::new()),
            ),
            (b"".as_slice(), DaemonActionDispatch::Unavailable),
            (b"weird".as_slice(), DaemonActionDispatch::Unavailable),
        ];

        for (input, expected) in cases {
            let got = parse_response(input);
            assert_eq!(
                std::mem::discriminant(&got),
                std::mem::discriminant(&expected)
            );
        }
    }
}
