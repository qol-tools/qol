use crate::{HeatLevel, TokenKind, TokenSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    Js,
    Generic,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Lang::Rust,
            "py" => Lang::Python,
            "js" | "ts" | "jsx" | "tsx" => Lang::Js,
            _ => Lang::Generic,
        }
    }
}

static RUST_KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "pub", "struct", "impl", "match", "if", "else", "for", "while", "return",
    "use", "mod", "trait", "enum", "self", "Self",
];

static PYTHON_KEYWORDS: &[&str] = &[
    "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as", "with",
    "lambda", "pass", "None", "True", "False", "in", "not", "and", "or",
];

static JS_KEYWORDS: &[&str] = &[
    "const", "let", "var", "function", "return", "if", "else", "for", "while", "class", "import",
    "export", "from", "new", "this", "async", "await", "try", "catch", "throw",
];

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => RUST_KEYWORDS,
        Lang::Python => PYTHON_KEYWORDS,
        Lang::Js => JS_KEYWORDS,
        Lang::Generic => &[],
    }
}

pub fn classify(line: &str, lang: Lang) -> Vec<TokenSpan> {
    let mut spans = Vec::new();
    if lang != Lang::Generic {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if is_line_comment(line, i, lang) {
                push_span(&mut spans, i, line.len() - i, TokenKind::Comment);
                break;
            }
            if lang != Lang::Python && bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                match block_comment_end(line, i) {
                    Some(end) => {
                        push_span(&mut spans, i, end - i, TokenKind::Comment);
                        i = end;
                    }
                    None => {
                        push_span(&mut spans, i, line.len() - i, TokenKind::Comment);
                        break;
                    }
                }
                continue;
            }
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                let end = scan_string(line, i);
                push_span(&mut spans, i, end - i, TokenKind::String);
                if end == line.len() {
                    break;
                }
                i = end;
                continue;
            }
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                if let Some(after) = keyword_at(line, i, keywords(lang)) {
                    push_span(&mut spans, i, after - i, TokenKind::Keyword);
                    i = after;
                    continue;
                }
            }
            i += 1;
        }
    }
    fill_plain(&mut spans, line.len());
    spans
}

pub fn merge_heat(spans: &mut Vec<TokenSpan>, heat_spans: &[TokenSpan]) -> Vec<TokenSpan> {
    let lexed = std::mem::take(spans);
    let mut merged = Vec::with_capacity(lexed.len() + heat_spans.len());
    let mut heat_i = 0;
    for lex in &lexed {
        let mut cursor = lex.start;
        let end = lex.start + lex.len;
        while heat_i < heat_spans.len()
            && heat_spans[heat_i].start + heat_spans[heat_i].len <= cursor
        {
            heat_i += 1;
        }
        while cursor < end {
            let Some(heat) = heat_spans.get(heat_i) else {
                push_seg(&mut merged, cursor, end - cursor, HeatLevel::Cool, lex.kind);
                break;
            };
            if heat.start >= end {
                push_seg(&mut merged, cursor, end - cursor, HeatLevel::Cool, lex.kind);
                break;
            }
            let seg_end = end.min(heat.start + heat.len);
            push_seg(&mut merged, cursor, seg_end - cursor, heat.heat, lex.kind);
            cursor = seg_end;
            if heat.start + heat.len <= cursor {
                heat_i += 1;
            }
        }
    }
    let result = merged.clone();
    *spans = merged;
    result
}

fn is_line_comment(line: &str, i: usize, lang: Lang) -> bool {
    let bytes = line.as_bytes();
    match lang {
        Lang::Rust | Lang::Js => bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/'),
        Lang::Python => bytes[i] == b'#',
        Lang::Generic => false,
    }
}

fn block_comment_end(line: &str, i: usize) -> Option<usize> {
    line[i + 2..].find("*/").map(|rel| i + 2 + rel + 2)
}

fn scan_string(line: &str, start: usize) -> usize {
    let bytes = line.as_bytes();
    let quote = bytes[start];
    let triple = start + 2 < bytes.len() && bytes[start + 1] == quote && bytes[start + 2] == quote;
    let mut j = start + if triple { 3 } else { 1 };
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        if triple {
            if j + 2 < bytes.len()
                && bytes[j] == quote
                && bytes[j + 1] == quote
                && bytes[j + 2] == quote
            {
                return j + 3;
            }
        } else if bytes[j] == quote {
            return j + 1;
        }
        j += 1;
    }
    line.len()
}

fn keyword_at(line: &str, i: usize, keywords: &[&str]) -> Option<usize> {
    if has_word_char_before(line, i) {
        return None;
    }
    for kw in keywords {
        if line[i..].starts_with(kw) && !word_char_at(line, i + kw.len()) {
            return Some(i + kw.len());
        }
    }
    None
}

fn has_word_char_before(line: &str, i: usize) -> bool {
    line[..i].chars().next_back().is_some_and(is_word_char)
}

fn word_char_at(line: &str, i: usize) -> bool {
    line.as_bytes()
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn push_span(spans: &mut Vec<TokenSpan>, start: usize, len: usize, kind: TokenKind) {
    if len == 0 {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.kind == kind && last.start + last.len == start {
            last.len += len;
            return;
        }
    }
    spans.push(TokenSpan {
        start,
        len,
        heat: HeatLevel::Cool,
        kind,
    });
}

fn push_seg(
    spans: &mut Vec<TokenSpan>,
    start: usize,
    len: usize,
    heat: HeatLevel,
    kind: TokenKind,
) {
    if len > 0 {
        spans.push(TokenSpan {
            start,
            len,
            heat,
            kind,
        });
    }
}

fn fill_plain(spans: &mut Vec<TokenSpan>, line_len: usize) {
    let mut cursor = 0;
    let mut filled = Vec::with_capacity(spans.len() + 1);
    for span in spans.drain(..) {
        if span.start > cursor {
            filled.push(TokenSpan {
                start: cursor,
                len: span.start - cursor,
                heat: HeatLevel::Cool,
                kind: TokenKind::Plain,
            });
        }
        filled.push(span);
        cursor = span.start + span.len;
    }
    if cursor < line_len {
        filled.push(TokenSpan {
            start: cursor,
            len: line_len - cursor,
            heat: HeatLevel::Cool,
            kind: TokenKind::Plain,
        });
    }
    *spans = filled;
}

#[cfg(test)]
mod tests {
    use super::{classify, merge_heat, Lang};
    use crate::{HeatLevel, TokenKind, TokenSpan};

    fn span(start: usize, len: usize, kind: TokenKind) -> TokenSpan {
        TokenSpan {
            start,
            len,
            heat: HeatLevel::Cool,
            kind,
        }
    }

    fn heat_span(start: usize, len: usize, heat: HeatLevel) -> TokenSpan {
        TokenSpan {
            start,
            len,
            heat,
            kind: TokenKind::Plain,
        }
    }

    fn coverage(spans: &[TokenSpan]) -> usize {
        spans.iter().map(|s| s.len).sum()
    }

    fn is_ordered_cover(spans: &[TokenSpan], line_len: usize) -> bool {
        coverage(spans) == line_len
            && spans
                .windows(2)
                .all(|w| w[0].start + w[0].len <= w[1].start)
            && spans.first().is_none_or(|s| s.start == 0)
    }

    #[test]
    fn lang_selects_from_file_extension() {
        assert_eq!(Lang::from_path("src/main.rs"), Lang::Rust);
        assert_eq!(Lang::from_path("A.RS"), Lang::Rust);
        assert_eq!(Lang::from_path("app.py"), Lang::Python);
        assert_eq!(Lang::from_path("app.js"), Lang::Js);
        assert_eq!(Lang::from_path("app.ts"), Lang::Js);
        assert_eq!(Lang::from_path("App.jsx"), Lang::Js);
        assert_eq!(Lang::from_path("app.tsx"), Lang::Js);
        assert_eq!(Lang::from_path("README"), Lang::Generic);
        assert_eq!(Lang::from_path("readme.txt"), Lang::Generic);
        assert_eq!(Lang::from_path("/dev/null"), Lang::Generic);
    }

    #[test]
    fn keywords_match_whole_words_only() {
        assert_eq!(
            classify("fn function", Lang::Rust),
            vec![span(0, 2, TokenKind::Keyword), span(2, 9, TokenKind::Plain)]
        );
        assert_eq!(
            classify("fnx", Lang::Rust),
            vec![span(0, 3, TokenKind::Plain)]
        );
        assert_eq!(
            classify("myfn", Lang::Rust),
            vec![span(0, 4, TokenKind::Plain)]
        );
        assert_eq!(
            classify("self Self", Lang::Rust),
            vec![
                span(0, 4, TokenKind::Keyword),
                span(4, 1, TokenKind::Plain),
                span(5, 4, TokenKind::Keyword),
            ]
        );
        assert_eq!(
            classify("not nothing", Lang::Python),
            vec![span(0, 3, TokenKind::Keyword), span(3, 8, TokenKind::Plain),]
        );
    }

    #[test]
    fn strings_handle_escapes() {
        assert_eq!(
            classify("\"a\\\"b\"", Lang::Rust),
            vec![span(0, 6, TokenKind::String)]
        );
        assert_eq!(
            classify("'it\\'s'", Lang::Js),
            vec![span(0, 7, TokenKind::String)]
        );
    }

    #[test]
    fn unterminated_string_runs_to_eol() {
        assert_eq!(
            classify("let s = \"abc", Lang::Rust),
            vec![
                span(0, 3, TokenKind::Keyword),
                span(3, 5, TokenKind::Plain),
                span(8, 4, TokenKind::String),
            ]
        );
    }

    #[test]
    fn python_hash_comment_runs_to_eol() {
        assert_eq!(
            classify("x = 1 # note", Lang::Python),
            vec![span(0, 6, TokenKind::Plain), span(6, 6, TokenKind::Comment),]
        );
    }

    #[test]
    fn python_triple_quoted_string() {
        assert_eq!(
            classify("x = \"\"\"doc\"\"\"", Lang::Python),
            vec![span(0, 4, TokenKind::Plain), span(4, 9, TokenKind::String),]
        );
    }

    #[test]
    fn rust_comment_styles() {
        assert_eq!(
            classify("let x = 1; // note", Lang::Rust),
            vec![
                span(0, 3, TokenKind::Keyword),
                span(3, 8, TokenKind::Plain),
                span(11, 7, TokenKind::Comment),
            ]
        );
        assert_eq!(
            classify("a /* mid */ b", Lang::Rust),
            vec![
                span(0, 2, TokenKind::Plain),
                span(2, 9, TokenKind::Comment),
                span(11, 2, TokenKind::Plain),
            ]
        );
        assert_eq!(
            classify("/* unterminated", Lang::Rust),
            vec![span(0, 15, TokenKind::Comment)]
        );
    }

    #[test]
    fn js_comment_styles() {
        assert_eq!(
            classify("const x = 1; // n", Lang::Js),
            vec![
                span(0, 5, TokenKind::Keyword),
                span(5, 8, TokenKind::Plain),
                span(13, 4, TokenKind::Comment),
            ]
        );
        assert_eq!(
            classify("/* c */ let", Lang::Js),
            vec![
                span(0, 7, TokenKind::Comment),
                span(7, 1, TokenKind::Plain),
                span(8, 3, TokenKind::Keyword),
            ]
        );
    }

    #[test]
    fn python_block_slashes_are_not_comments() {
        assert_eq!(
            classify("a /* b", Lang::Python),
            vec![span(0, 6, TokenKind::Plain)]
        );
    }

    #[test]
    fn keywords_inside_strings_and_comments_stay_plain() {
        assert_eq!(
            classify("let \"fn\" // fn", Lang::Rust),
            vec![
                span(0, 3, TokenKind::Keyword),
                span(3, 1, TokenKind::Plain),
                span(4, 4, TokenKind::String),
                span(8, 1, TokenKind::Plain),
                span(9, 5, TokenKind::Comment),
            ]
        );
    }

    #[test]
    fn generic_falls_back_to_plain() {
        assert_eq!(
            classify("fn \"x\" // c", Lang::Generic),
            vec![span(0, 11, TokenKind::Plain)]
        );
        assert_eq!(classify("", Lang::Generic), Vec::new());
    }

    #[test]
    fn classify_covers_the_line_exactly() {
        let lines = [
            ("fn main() {", Lang::Rust),
            ("    let x = \"hi\"; // note", Lang::Rust),
            ("/* block */ fn f() {}", Lang::Rust),
            ("let s = 'unterminated", Lang::Rust),
            ("def f(x):", Lang::Python),
            ("    return \"\"\"doc\"\"\"  # done", Lang::Python),
            ("x = 'a\\'b'", Lang::Python),
            ("const a = 'b'; // n", Lang::Js),
            ("async function f() { await g(); }", Lang::Js),
            ("nothing special", Lang::Generic),
        ];
        for (line, lang) in lines {
            assert!(
                is_ordered_cover(&classify(line, lang), line.len()),
                "{line:?}"
            );
        }
    }

    #[test]
    fn merge_heat_splits_hot_span_across_kind_boundaries() {
        let mut lexed = classify("fn \"ab\"", Lang::Rust);
        let heat = vec![
            heat_span(0, 1, HeatLevel::Cool),
            heat_span(1, 5, HeatLevel::Hot),
            heat_span(6, 1, HeatLevel::Cool),
        ];
        let merged = merge_heat(&mut lexed, &heat);
        assert_eq!(
            merged,
            vec![
                TokenSpan {
                    start: 0,
                    len: 1,
                    heat: HeatLevel::Cool,
                    kind: TokenKind::Keyword,
                },
                TokenSpan {
                    start: 1,
                    len: 1,
                    heat: HeatLevel::Hot,
                    kind: TokenKind::Keyword,
                },
                TokenSpan {
                    start: 2,
                    len: 1,
                    heat: HeatLevel::Hot,
                    kind: TokenKind::Plain,
                },
                TokenSpan {
                    start: 3,
                    len: 3,
                    heat: HeatLevel::Hot,
                    kind: TokenKind::String,
                },
                TokenSpan {
                    start: 6,
                    len: 1,
                    heat: HeatLevel::Cool,
                    kind: TokenKind::String,
                },
            ]
        );
        assert!(is_ordered_cover(&merged, "fn \"ab\"".len()));
    }

    #[test]
    fn merge_heat_preserves_cool_and_hot_heat() {
        let mut lexed = classify("x", Lang::Rust);
        let heat = vec![heat_span(0, 1, HeatLevel::Hot)];
        let merged = merge_heat(&mut lexed, &heat);
        assert_eq!(
            merged,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot,
                kind: TokenKind::Plain,
            }]
        );
        assert_eq!(lexed, merged);
    }
}
