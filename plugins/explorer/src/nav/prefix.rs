use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn starts_with_ignoring_case(name: &str, prefix: &str) -> bool {
    let mut name = letters(name);
    letters(prefix).all(|wanted| name.next().is_some_and(|letter| letter.answers(&wanted)))
}

pub(crate) fn letter_count(text: &str) -> usize {
    text.nfd()
        .filter(|letter| !is_combining_mark(*letter))
        .count()
}

struct Letter {
    base: char,
    marks: Vec<char>,
}

impl Letter {
    fn answers(&self, wanted: &Self) -> bool {
        self.base.to_lowercase().eq(wanted.base.to_lowercase())
            && (wanted.marks.is_empty() || self.marks == wanted.marks)
    }
}

fn letters(text: &str) -> impl Iterator<Item = Letter> + '_ {
    let mut chars = text.nfd().peekable();
    std::iter::from_fn(move || {
        let base = chars.next()?;
        let mut marks = Vec::new();
        while let Some(mark) = chars.next_if(|next| is_combining_mark(*next)) {
            marks.push(mark);
        }
        Some(Letter { base, marks })
    })
}
