pub(crate) fn parse_uri_list(list: &str) -> Vec<String> {
    list.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|uri| percent_decode(uri.strip_prefix("file://").unwrap_or(uri)))
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes
            .get(index + 1..index + 3)
            .filter(|_| bytes[index] == b'%')
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{parse_uri_list, percent_decode};

    #[test]
    fn uri_lists_become_decoded_paths() {
        let list = "# copied\r\nfile:///home/me/My%20File.txt\r\nfile:///tmp/a\r\n\r\n";
        assert_eq!(
            parse_uri_list(list),
            vec!["/home/me/My File.txt".to_string(), "/tmp/a".to_string()]
        );
        assert_eq!(percent_decode("100%"), "100%");
    }
}
