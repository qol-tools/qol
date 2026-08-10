use crate::{Error, NumstatEntry};

pub fn parse_numstat_line(line: &str) -> Result<NumstatEntry, Error> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut fields = line.splitn(3, '\t');
    let added = fields
        .next()
        .ok_or_else(|| parse_error(line, "missing added count"))?;
    let deleted = fields
        .next()
        .ok_or_else(|| parse_error(line, "missing deleted count"))?;
    let path = fields
        .next()
        .ok_or_else(|| parse_error(line, "missing path"))?;
    let added = parse_count(added).map_err(|detail| parse_error(line, detail))?;
    let deleted = parse_count(deleted).map_err(|detail| parse_error(line, detail))?;
    let path = if path.starts_with('"') {
        unquote_path(path)
    } else {
        path.to_string()
    };
    Ok(NumstatEntry {
        added,
        deleted,
        path,
    })
}

fn parse_count(field: &str) -> Result<Option<u64>, &'static str> {
    if field == "-" {
        return Ok(None);
    }
    field
        .parse::<u64>()
        .map(Some)
        .map_err(|_| "count is not a number or '-'")
}

pub(crate) fn parse_error(line: &str, detail: &str) -> Error {
    Error::Parse {
        line: line.to_string(),
        detail: detail.to_string(),
    }
}

pub(crate) fn unquote_prefix(quoted: &str) -> (String, &str) {
    let bytes = quoted.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                break;
            }
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                match bytes[i] {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b't' => out.push(b'\t'),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'a' => out.push(0x07),
                    b'v' => out.push(0x0b),
                    b'0'..=b'7' => {
                        let mut value = 0u32;
                        let mut count = 0;
                        while i < bytes.len() && count < 3 && (b'0'..=b'7').contains(&bytes[i]) {
                            value = value * 8 + u32::from(bytes[i] - b'0');
                            i += 1;
                            count += 1;
                        }
                        out.push(value as u8);
                        continue;
                    }
                    other => out.push(other),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    (String::from_utf8_lossy(&out).into_owned(), &quoted[i..])
}

fn unquote_path(quoted: &str) -> String {
    unquote_prefix(quoted).0
}

#[cfg(test)]
mod tests {
    use super::parse_numstat_line;

    #[test]
    fn parses_plain_line() {
        let entry = parse_numstat_line("3\t2\tcrate/src/lib.rs").expect("parse");
        assert_eq!(entry.added, Some(3));
        assert_eq!(entry.deleted, Some(2));
        assert_eq!(entry.path, "crate/src/lib.rs");
    }

    #[test]
    fn parses_zero_counts() {
        let entry = parse_numstat_line("0\t0\tREADME.md").expect("parse");
        assert_eq!(entry.added, Some(0));
        assert_eq!(entry.deleted, Some(0));
        assert_eq!(entry.path, "README.md");
    }

    #[test]
    fn parses_binary_line() {
        let entry = parse_numstat_line("-\t-\tassets/logo.png").expect("parse");
        assert_eq!(entry.added, None);
        assert_eq!(entry.deleted, None);
        assert_eq!(entry.path, "assets/logo.png");
    }

    #[test]
    fn parses_octal_escaped_path() {
        let entry = parse_numstat_line("0\t1\t\"na\\303\\257ve.rs\"").expect("parse");
        assert_eq!(entry.path, "na\u{ef}ve.rs");
    }

    #[test]
    fn parses_escaped_quote_in_path() {
        let entry = parse_numstat_line("0\t1\t\"quo\\\"te.rs\"").expect("parse");
        assert_eq!(entry.path, "quo\"te.rs");
    }

    #[test]
    fn parses_embedded_tab_in_quoted_path() {
        let entry = parse_numstat_line("2\t0\t\"weird\tname.rs\"").expect("parse");
        assert_eq!(entry.path, "weird\tname.rs");
    }

    #[test]
    fn parses_backslash_escape() {
        let entry = parse_numstat_line("1\t1\t\"dir\\\\file.rs\"").expect("parse");
        assert_eq!(entry.path, "dir\\file.rs");
    }

    #[test]
    fn strips_trailing_newline() {
        let entry = parse_numstat_line("1\t0\tfile.rs\n").expect("parse");
        assert_eq!(entry.path, "file.rs");
    }

    #[test]
    fn rejects_missing_deleted_count() {
        assert!(parse_numstat_line("3\tfile.rs").is_err());
    }

    #[test]
    fn rejects_missing_path() {
        assert!(parse_numstat_line("3\t2").is_err());
    }

    #[test]
    fn rejects_non_numeric_count() {
        assert!(parse_numstat_line("x\t2\tfile.rs").is_err());
    }

    #[test]
    fn rejects_empty_line() {
        assert!(parse_numstat_line("").is_err());
    }
}
