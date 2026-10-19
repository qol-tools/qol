use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn starts_with_ignoring_case(name: &str, prefix: &str) -> bool {
    let mut name = folded(name);
    folded(prefix).all(|wanted| name.next() == Some(wanted))
}

pub(crate) fn folded(text: &str) -> impl Iterator<Item = char> + '_ {
    text.nfd()
        .filter(|letter| !is_combining_mark(*letter))
        .flat_map(char::to_lowercase)
}
