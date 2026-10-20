use std::ffi::{OsStr, OsString};

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    group: Group,
    parts: Box<[Part]>,
    raw: OsString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Folder,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Number { digits: usize, value: Box<str> },
    Text(Box<str>),
}

impl SortKey {
    pub fn new(name: &OsStr, folder: bool) -> Self {
        Self {
            group: if folder { Group::Folder } else { Group::Other },
            parts: parts(&name.to_string_lossy()),
            raw: name.to_os_string(),
        }
    }
}

fn parts(name: &str) -> Box<[Part]> {
    let mut parts = Vec::new();
    let mut rest = name;
    while let Some(first) = rest.chars().next() {
        let digit = first.is_ascii_digit();
        let end = rest
            .find(|c: char| c.is_ascii_digit() != digit)
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        parts.push(if digit { number(run) } else { text(run) });
        rest = tail;
    }
    parts.into()
}

fn number(run: &str) -> Part {
    let value = run.trim_start_matches('0');
    Part::Number {
        digits: value.len(),
        value: value.into(),
    }
}

fn text(run: &str) -> Part {
    let plain: String = run.nfd().filter(|c| !is_combining_mark(*c)).collect();
    Part::Text(plain.to_lowercase().into())
}
