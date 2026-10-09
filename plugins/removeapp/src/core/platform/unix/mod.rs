pub(super) fn sanitize_stderr(raw: &[u8], cap: usize) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if c == '\n' || c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    let end = (0..=cap.min(out.len()))
        .rev()
        .find(|&i| out.is_char_boundary(i))
        .unwrap_or(0);
    out.truncate(end);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_stderr_strips_control_and_caps() {
        let raw = b"\x1b[31merror\x1b[0m\x07 happened";
        let out = sanitize_stderr(raw, 64);
        assert_eq!(out, "error happened");
        assert_eq!(sanitize_stderr(b"abcdef", 3), "abc");
    }
}
