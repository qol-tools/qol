use crate::lexer::Lang;
use crate::{LineChange, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructKind {
    Arc,
    Coil,
    Fork,
    Lattice,
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct {
    pub kind: ConstructKind,
    pub start_line: usize,
    pub end_line: usize,
    pub depth: usize,
    pub anchor: (usize, usize),
}

struct KeywordHit {
    pos: usize,
    word: &'static str,
    kind: ConstructKind,
    balance: isize,
    closes_before: isize,
}

struct LineScan {
    opens: isize,
    closes: isize,
    keywords: Vec<KeywordHit>,
    arrows: usize,
    indent: usize,
    has_code: bool,
    first_continuation: bool,
}

struct ForkSpan {
    end: usize,
}

#[derive(Default)]
struct CarryState {
    block_comment: bool,
    triple_quote: Option<u8>,
}

const RUST_CONSTRUCTS: &[(&str, ConstructKind)] = &[
    ("fn", ConstructKind::Arc),
    ("for", ConstructKind::Coil),
    ("while", ConstructKind::Coil),
    ("if", ConstructKind::Fork),
    ("else", ConstructKind::Fork),
    ("match", ConstructKind::Fork),
    ("struct", ConstructKind::Lattice),
    ("impl", ConstructKind::Lattice),
    ("enum", ConstructKind::Lattice),
    ("type", ConstructKind::Lattice),
    ("use", ConstructKind::Tick),
    ("const", ConstructKind::Tick),
];

const PYTHON_CONSTRUCTS: &[(&str, ConstructKind)] = &[
    ("def", ConstructKind::Arc),
    ("for", ConstructKind::Coil),
    ("while", ConstructKind::Coil),
    ("if", ConstructKind::Fork),
    ("elif", ConstructKind::Fork),
    ("else", ConstructKind::Fork),
    ("match", ConstructKind::Fork),
    ("case", ConstructKind::Fork),
    ("class", ConstructKind::Lattice),
    ("import", ConstructKind::Tick),
];

const JS_CONSTRUCTS: &[(&str, ConstructKind)] = &[
    ("function", ConstructKind::Arc),
    ("for", ConstructKind::Coil),
    ("while", ConstructKind::Coil),
    ("if", ConstructKind::Fork),
    ("else", ConstructKind::Fork),
    ("switch", ConstructKind::Fork),
    ("case", ConstructKind::Fork),
    ("class", ConstructKind::Lattice),
    ("enum", ConstructKind::Lattice),
    ("import", ConstructKind::Tick),
    ("const", ConstructKind::Tick),
];

const GENERIC_CONSTRUCTS: &[(&str, ConstructKind)] = &[
    ("fn", ConstructKind::Arc),
    ("def", ConstructKind::Arc),
    ("func", ConstructKind::Arc),
    ("function", ConstructKind::Arc),
    ("for", ConstructKind::Coil),
    ("while", ConstructKind::Coil),
    ("if", ConstructKind::Fork),
    ("else", ConstructKind::Fork),
    ("elif", ConstructKind::Fork),
    ("match", ConstructKind::Fork),
    ("switch", ConstructKind::Fork),
    ("case", ConstructKind::Fork),
    ("struct", ConstructKind::Lattice),
    ("impl", ConstructKind::Lattice),
    ("class", ConstructKind::Lattice),
    ("enum", ConstructKind::Lattice),
    ("type", ConstructKind::Lattice),
    ("import", ConstructKind::Tick),
    ("use", ConstructKind::Tick),
    ("const", ConstructKind::Tick),
];

fn construct_vocab(lang: Lang) -> &'static [(&'static str, ConstructKind)] {
    match lang {
        Lang::Rust => RUST_CONSTRUCTS,
        Lang::Python => PYTHON_CONSTRUCTS,
        Lang::Js => JS_CONSTRUCTS,
        Lang::Generic => GENERIC_CONSTRUCTS,
    }
}

pub fn detect_constructs(lines: &[LineChange], lang: Lang) -> Vec<Construct> {
    let mut state = CarryState::default();
    let scans: Vec<LineScan> = lines
        .iter()
        .map(|line| scan_line(line, &mut state, lang))
        .collect();
    let depths = brace_depths(&scans);
    let mut constructs = Vec::new();
    let mut forks = Vec::new();
    for (line_index, scan) in scans.iter().enumerate() {
        let before = depths[line_index];
        for hit in &scan.keywords {
            let base = before + hit.balance;
            if hit.kind == ConstructKind::Fork {
                if absorbed(&forks, line_index, hit, scan) {
                    continue;
                }
                let closing = closing_line(&scans, &depths, line_index, base, scan, hit, true);
                forks.push(ForkSpan { end: closing });
                constructs.push(Construct {
                    kind: ConstructKind::Fork,
                    start_line: line_index,
                    end_line: closing,
                    depth: depth_usize(base),
                    anchor: (line_index, closing),
                });
            } else {
                let closing = closing_line(&scans, &depths, line_index, base, scan, hit, false);
                constructs.push(Construct {
                    kind: hit.kind,
                    start_line: line_index,
                    end_line: closing,
                    depth: depth_usize(base),
                    anchor: (line_index, closing),
                });
            }
        }
    }
    constructs
}

pub fn branch_arms(lines: &[LineChange], construct: &Construct, lang: Lang) -> usize {
    if construct.kind != ConstructKind::Fork {
        return 0;
    }
    let mut state = CarryState::default();
    let scans: Vec<LineScan> = lines
        .iter()
        .map(|line| scan_line(line, &mut state, lang))
        .collect();
    let depths = brace_depths(&scans);
    let Some(start_scan) = scans.get(construct.start_line) else {
        return 1;
    };
    let base = construct.depth as isize;
    let opener = start_scan.keywords.iter().find(|hit| {
        hit.kind == ConstructKind::Fork && depths[construct.start_line] + hit.balance == base
    });
    let span_end = construct.end_line.saturating_add(1);
    match opener {
        Some(opener) if matches!(opener.word, "match" | "switch") => {
            let mut cases = 0;
            let mut arrows = 0;
            for scan in scans.iter().take(span_end).skip(construct.start_line) {
                cases += scan
                    .keywords
                    .iter()
                    .filter(|hit| hit.word == "case")
                    .count();
                arrows += scan.arrows;
            }
            cases.max(arrows).max(1)
        }
        _ => {
            let mut continuations = 0;
            for (index, scan) in scans
                .iter()
                .enumerate()
                .take(span_end)
                .skip(construct.start_line + 1)
            {
                let before = depths[index];
                if scan
                    .keywords
                    .iter()
                    .any(|hit| is_continuation(hit.word) && before + hit.balance == base)
                {
                    continuations += 1;
                }
            }
            1 + continuations
        }
    }
}

fn absorbed(forks: &[ForkSpan], line_index: usize, hit: &KeywordHit, scan: &LineScan) -> bool {
    if is_continuation(hit.word) {
        return forks.iter().any(|fork| fork.end >= line_index);
    }
    scan.keywords
        .iter()
        .take_while(|other| other.pos < hit.pos)
        .any(|other| is_continuation(other.word) && other.balance == hit.balance)
}

fn closing_line(
    scans: &[LineScan],
    depths: &[isize],
    line_index: usize,
    base: isize,
    scan: &LineScan,
    hit: &KeywordHit,
    fork: bool,
) -> usize {
    let opens_after = scan.opens - (hit.balance + hit.closes_before);
    if opens_after > 0 {
        let after = depths[line_index] + scan.opens - scan.closes;
        if after <= base {
            return line_index;
        }
        for (index, scan) in scans.iter().enumerate().skip(line_index + 1) {
            if depths[index] + scan.opens - scan.closes <= base {
                return index;
            }
        }
        return scans.len().saturating_sub(1);
    }
    let keyword_indent = scan.indent;
    let extended = scans
        .iter()
        .skip(line_index + 1)
        .find(|scan| scan.has_code)
        .is_some_and(|scan| scan.indent > keyword_indent);
    if !extended {
        return line_index;
    }
    for (index, scan) in scans.iter().enumerate().skip(line_index + 1) {
        if !scan.has_code || scan.indent > keyword_indent {
            continue;
        }
        if fork && scan.first_continuation {
            continue;
        }
        return index;
    }
    scans.len().saturating_sub(1)
}

fn scan_line(line: &LineChange, state: &mut CarryState, lang: Lang) -> LineScan {
    let text = line.text.as_str();
    let bytes = text.as_bytes();
    let mut regions = Vec::new();
    for span in &line.token_spans {
        if span.kind == TokenKind::String || span.kind == TokenKind::Comment {
            let start = span.start.min(bytes.len());
            let end = span.start.saturating_add(span.len).min(bytes.len());
            if start < end {
                regions.push((start, end));
            }
        }
    }
    regions.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(regions.len());
    for region in regions {
        match merged.last_mut() {
            Some(last) if region.0 <= last.1 => last.1 = last.1.max(region.1),
            _ => merged.push(region),
        }
    }
    let indent = leading_indent(bytes);
    let mut i = 0usize;
    if state.block_comment {
        match find_bytes(bytes, b"*/", i) {
            Some(end) => {
                state.block_comment = false;
                i = end + 2;
            }
            None => return empty_scan(indent),
        }
    }
    if let Some(quote) = state.triple_quote {
        match find_bytes(bytes, &[quote, quote, quote], i) {
            Some(end) => {
                state.triple_quote = None;
                i = end + 3;
            }
            None => return empty_scan(indent),
        }
    }
    let mut opens = 0isize;
    let mut closes = 0isize;
    let mut balance = 0isize;
    let mut running_closes = 0isize;
    let mut keywords = Vec::new();
    let mut arrows = 0usize;
    let mut has_code = false;
    let mut first_continuation = false;
    let mut first_word = true;
    let mut region_index = 0usize;
    while i < bytes.len() {
        while region_index < merged.len() && merged[region_index].1 <= i {
            region_index += 1;
        }
        if let Some((start, end)) = merged.get(region_index).copied() {
            if start <= i && i < end {
                let region_text = &text[start..end];
                if region_text.starts_with("/*") && !region_text.contains("*/") {
                    state.block_comment = true;
                    i = end;
                    continue;
                }
                if let Some(quote) = triple_quote_start(region_text) {
                    if !region_text.as_bytes().ends_with(&triple(quote)) {
                        state.triple_quote = Some(quote);
                        i = end;
                        continue;
                    }
                }
                i = end;
                continue;
            }
        }
        let byte = bytes[i];
        if is_word_byte(byte) {
            has_code = true;
            let word_start = i;
            while i < bytes.len() && is_word_byte(bytes[i]) {
                i += 1;
            }
            let word = &text[word_start..i];
            if first_word {
                first_word = false;
                first_continuation = is_continuation(word);
            }
            if let Some((word, kind)) = keyword_hit(word, lang) {
                keywords.push(KeywordHit {
                    pos: word_start,
                    word,
                    kind,
                    balance,
                    closes_before: running_closes,
                });
            }
            continue;
        }
        match byte {
            b'{' => {
                opens += 1;
                balance += 1;
            }
            b'}' => {
                closes += 1;
                running_closes += 1;
                balance -= 1;
            }
            b'=' if bytes.get(i + 1) == Some(&b'>') => {
                arrows += 1;
                i += 1;
            }
            _ => {}
        }
        if !byte.is_ascii_whitespace() {
            has_code = true;
        }
        i += 1;
    }
    LineScan {
        opens,
        closes,
        keywords,
        arrows,
        indent,
        has_code,
        first_continuation,
    }
}

fn brace_depths(scans: &[LineScan]) -> Vec<isize> {
    let mut depths = Vec::with_capacity(scans.len());
    let mut depth = 0isize;
    for scan in scans {
        depths.push(depth);
        depth += scan.opens - scan.closes;
    }
    depths
}

fn empty_scan(indent: usize) -> LineScan {
    LineScan {
        opens: 0,
        closes: 0,
        keywords: Vec::new(),
        arrows: 0,
        indent,
        has_code: false,
        first_continuation: false,
    }
}

fn keyword_hit(word: &str, lang: Lang) -> Option<(&'static str, ConstructKind)> {
    construct_vocab(lang)
        .iter()
        .find(|(candidate, _)| *candidate == word)
        .copied()
}

fn is_continuation(word: &str) -> bool {
    matches!(word, "else" | "elif" | "case")
}

fn triple_quote_start(text: &str) -> Option<u8> {
    if text.starts_with("\"\"\"") {
        Some(b'"')
    } else if text.starts_with("'''") {
        Some(b'\'')
    } else {
        None
    }
}

fn triple(quote: u8) -> [u8; 3] {
    [quote, quote, quote]
}

fn find_bytes(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| offset + from)
}

fn leading_indent(bytes: &[u8]) -> usize {
    let mut indent = 0;
    for byte in bytes {
        match byte {
            b' ' => indent += 1,
            b'\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn depth_usize(depth: isize) -> usize {
    depth.max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{classify, Lang};
    use crate::{LineKind, TokenSpan};

    fn line(text: &str, lang: Lang) -> LineChange {
        LineChange {
            kind: LineKind::Context,
            text: text.to_string(),
            token_spans: classify(text, lang),
            old_line_no: None,
            new_line_no: None,
        }
    }

    fn raw_line(text: &str, spans: Vec<TokenSpan>) -> LineChange {
        LineChange {
            kind: LineKind::Context,
            text: text.to_string(),
            token_spans: spans,
            old_line_no: None,
            new_line_no: None,
        }
    }

    #[test]
    fn nested_constructs_report_span_and_depth() {
        let lines = vec![
            line("fn outer() {", Lang::Rust),
            line("    for i in 0..n {", Lang::Rust),
            line("        if i > 0 {", Lang::Rust),
            line("            f(i);", Lang::Rust),
            line("        }", Lang::Rust),
            line("    }", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 3);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!(
            (
                constructs[0].start_line,
                constructs[0].end_line,
                constructs[0].depth
            ),
            (0, 6, 0)
        );
        assert_eq!(constructs[0].anchor, (0, 6));
        assert_eq!(constructs[1].kind, ConstructKind::Coil);
        assert_eq!(
            (
                constructs[1].start_line,
                constructs[1].end_line,
                constructs[1].depth
            ),
            (1, 5, 1)
        );
        assert_eq!(constructs[2].kind, ConstructKind::Fork);
        assert_eq!(
            (
                constructs[2].start_line,
                constructs[2].end_line,
                constructs[2].depth
            ),
            (2, 4, 2)
        );
        assert_eq!(constructs[2].anchor, (2, 4));
    }

    #[test]
    fn loop_inside_function_renders_coil_inside_arc() {
        let lines = vec![
            line("fn total(n: u32) -> u32 {", Lang::Rust),
            line("    let mut sum = 0;", Lang::Rust),
            line("    while n > 0 {", Lang::Rust),
            line("        sum += n;", Lang::Rust),
            line("        n -= 1;", Lang::Rust),
            line("    }", Lang::Rust),
            line("    sum", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 2);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 7));
        assert_eq!(constructs[1].kind, ConstructKind::Coil);
        assert_eq!(
            (
                constructs[1].start_line,
                constructs[1].end_line,
                constructs[1].depth
            ),
            (2, 5, 1)
        );
    }

    #[test]
    fn match_arms_fork_prongs() {
        let lines = vec![
            line("fn classify(x: u32) -> u32 {", Lang::Rust),
            line("    match x {", Lang::Rust),
            line("        1 => 10,", Lang::Rust),
            line("        2 => 20,", Lang::Rust),
            line("        _ => 0,", Lang::Rust),
            line("    }", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 2);
        let fork = &constructs[1];
        assert_eq!(fork.kind, ConstructKind::Fork);
        assert_eq!((fork.start_line, fork.end_line), (1, 5));
        assert_eq!(branch_arms(&lines, fork, Lang::Rust), 3);
    }

    #[test]
    fn imports_and_consts_render_ticks() {
        let lines = vec![
            line("use std::collections::HashMap;", Lang::Rust),
            line("const LIMIT: u32 = 8;", Lang::Rust),
            line("fn main() {}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 3);
        assert_eq!(constructs[0].kind, ConstructKind::Tick);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 0));
        assert_eq!(constructs[1].kind, ConstructKind::Tick);
        assert_eq!(constructs[2].kind, ConstructKind::Arc);
        assert_eq!((constructs[2].start_line, constructs[2].end_line), (2, 2));
    }

    #[test]
    fn strings_and_comments_are_invisible_to_the_pass() {
        let lines = vec![
            line("let s = \"fn fake() {\";", Lang::Rust),
            line("// if (x) { for (;;) { }", Lang::Rust),
            line("/* while (true) { */", Lang::Rust),
            line("let t = 1;", Lang::Rust),
        ];
        assert!(detect_constructs(&lines, Lang::Rust).is_empty());
    }

    #[test]
    fn braces_inside_strings_do_not_warp_the_envelope() {
        let lines = vec![
            line("let s = \"{\";", Lang::Rust),
            line("fn main() {", Lang::Rust),
            line("    let t = \"}\";", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        assert_eq!(
            (
                constructs[0].kind,
                constructs[0].start_line,
                constructs[0].end_line
            ),
            (ConstructKind::Arc, 1, 3)
        );
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let lines = vec![
            line("/* begin", Lang::Rust),
            line("fn hidden() {", Lang::Rust),
            line("    for i in 0..2 { }", Lang::Rust),
            line("*/", Lang::Rust),
            line("fn visible() {}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!(constructs[0].start_line, 4);
    }

    #[test]
    fn python_triple_strings_carry_across_lines() {
        let lines = vec![
            line("def f():", Lang::Python),
            line("    \"\"\"docstring with fn() {", Lang::Python),
            line("    more docs", Lang::Python),
            line("    \"\"\"", Lang::Python),
            line("    return 1", Lang::Python),
        ];
        let constructs = detect_constructs(&lines, Lang::Python);
        assert_eq!(constructs.len(), 1);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 4));
    }

    #[test]
    fn else_chain_merges_into_one_fork() {
        let lines = vec![
            line("if a {", Lang::Rust),
            line("    f();", Lang::Rust),
            line("} else {", Lang::Rust),
            line("    g();", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        let fork = &constructs[0];
        assert_eq!((fork.start_line, fork.end_line), (0, 4));
        assert_eq!(branch_arms(&lines, fork, Lang::Rust), 2);
    }

    #[test]
    fn else_if_extends_the_fork_chain() {
        let lines = vec![
            line("if a {", Lang::Rust),
            line("    f();", Lang::Rust),
            line("} else if b {", Lang::Rust),
            line("    g();", Lang::Rust),
            line("} else {", Lang::Rust),
            line("    h();", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        let fork = &constructs[0];
        assert_eq!((fork.start_line, fork.end_line), (0, 6));
        assert_eq!(branch_arms(&lines, fork, Lang::Rust), 3);
    }

    #[test]
    fn indented_bodies_extend_python_constructs() {
        let lines = vec![
            line("def f(x):", Lang::Python),
            line("    for i in range(x):", Lang::Python),
            line("        if i:", Lang::Python),
            line("            return", Lang::Python),
            line("    return 0", Lang::Python),
        ];
        let constructs = detect_constructs(&lines, Lang::Python);
        assert_eq!(constructs.len(), 3);
        assert_eq!(
            (
                constructs[0].kind,
                constructs[0].start_line,
                constructs[0].end_line
            ),
            (ConstructKind::Arc, 0, 4)
        );
        assert_eq!(
            (
                constructs[1].kind,
                constructs[1].start_line,
                constructs[1].end_line
            ),
            (ConstructKind::Coil, 1, 4)
        );
        assert_eq!(
            (
                constructs[2].kind,
                constructs[2].start_line,
                constructs[2].end_line
            ),
            (ConstructKind::Fork, 2, 4)
        );
    }

    #[test]
    fn python_match_cases_form_one_fork() {
        let lines = vec![
            line("match cmd:", Lang::Python),
            line("    case \"a\":", Lang::Python),
            line("        go()", Lang::Python),
            line("    case _:", Lang::Python),
            line("        stop()", Lang::Python),
        ];
        let constructs = detect_constructs(&lines, Lang::Python);
        assert_eq!(constructs.len(), 1);
        let fork = &constructs[0];
        assert_eq!(fork.kind, ConstructKind::Fork);
        assert_eq!((fork.start_line, fork.end_line), (0, 4));
        assert_eq!(branch_arms(&lines, fork, Lang::Python), 2);
    }

    #[test]
    fn structs_and_impls_render_lattices() {
        let lines = vec![
            line("struct Point {", Lang::Rust),
            line("    x: f64,", Lang::Rust),
            line("    y: f64,", Lang::Rust),
            line("}", Lang::Rust),
            line("impl Point {", Lang::Rust),
            line("    fn dist(&self) -> f64 {", Lang::Rust),
            line("        0.0", Lang::Rust),
            line("    }", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 3);
        assert_eq!(
            (
                constructs[0].kind,
                constructs[0].start_line,
                constructs[0].end_line
            ),
            (ConstructKind::Lattice, 0, 3)
        );
        assert_eq!(
            (
                constructs[1].kind,
                constructs[1].start_line,
                constructs[1].end_line
            ),
            (ConstructKind::Lattice, 4, 8)
        );
        assert_eq!(
            (
                constructs[2].kind,
                constructs[2].start_line,
                constructs[2].end_line
            ),
            (ConstructKind::Arc, 5, 7)
        );
    }

    #[test]
    fn js_functions_and_switches_detect() {
        let lines = vec![
            line("function run() {", Lang::Js),
            line("    switch (x) {", Lang::Js),
            line("        case 1:", Lang::Js),
            line("            break;", Lang::Js),
            line("        case 2:", Lang::Js),
            line("            break;", Lang::Js),
            line("    }", Lang::Js),
            line("}", Lang::Js),
        ];
        let constructs = detect_constructs(&lines, Lang::Js);
        assert_eq!(constructs.len(), 2);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 7));
        let fork = &constructs[1];
        assert_eq!((fork.start_line, fork.end_line), (1, 6));
        assert_eq!(branch_arms(&lines, fork, Lang::Js), 2);
    }

    #[test]
    fn stray_else_without_its_if_still_renders() {
        let lines = vec![
            line("} else {", Lang::Rust),
            line("    g();", Lang::Rust),
            line("}", Lang::Rust),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        assert_eq!(constructs[0].kind, ConstructKind::Fork);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 2));
    }

    #[test]
    fn empty_token_spans_treat_the_whole_line_as_code() {
        let lines = vec![
            raw_line("fn bare() {", Vec::new()),
            raw_line("    body();", Vec::new()),
            raw_line("}", Vec::new()),
        ];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(constructs.len(), 1);
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
        assert_eq!((constructs[0].start_line, constructs[0].end_line), (0, 2));
    }

    #[test]
    fn language_vocabularies_keep_foreign_words_silent() {
        let js = vec![
            line("const hit = str.match(/x/);", Lang::Js),
            line("const type = 1;", Lang::Js),
            line("function run() {", Lang::Js),
            line("    return hit;", Lang::Js),
            line("}", Lang::Js),
        ];
        let constructs = detect_constructs(&js, Lang::Js);
        assert_eq!(constructs.len(), 3, "match() and type add no shapes");
        assert_eq!(constructs[0].kind, ConstructKind::Tick);
        assert_eq!(constructs[1].kind, ConstructKind::Tick);
        assert_eq!(constructs[2].kind, ConstructKind::Arc);
        assert_eq!((constructs[2].start_line, constructs[2].end_line), (2, 4));
        let python = vec![
            line("def f(x):", Lang::Python),
            line("    return type(x)", Lang::Python),
        ];
        let constructs = detect_constructs(&python, Lang::Python);
        assert_eq!(constructs.len(), 1, "type() is not a Python construct");
        assert_eq!(constructs[0].kind, ConstructKind::Arc);
    }

    #[test]
    fn non_fork_constructs_report_zero_arms() {
        let lines = vec![line("fn f() {}", Lang::Rust)];
        let constructs = detect_constructs(&lines, Lang::Rust);
        assert_eq!(branch_arms(&lines, &constructs[0], Lang::Rust), 0);
    }
}
