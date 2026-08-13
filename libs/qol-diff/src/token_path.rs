#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenFate {
    Kept,
    Ignited,
    Evaporated,
    Morphed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEdit {
    pub text: String,
    pub fate: TokenFate,
    pub changed_range: Option<(usize, usize)>,
}

pub fn token_edit_path(old_text: &str, new_text: &str) -> Vec<TokenEdit> {
    let old = tokenize(old_text);
    let new = tokenize(new_text);
    let mut path = Vec::with_capacity(old.len() + new.len());
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix].2 == new[prefix].2 {
        prefix += 1;
    }
    let mut suffix = 0;
    while prefix + suffix < old.len()
        && prefix + suffix < new.len()
        && old[old.len() - 1 - suffix].2 == new[new.len() - 1 - suffix].2
    {
        suffix += 1;
    }
    for (_, _, text) in &old[..prefix] {
        path.push(kept(text));
    }
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];
    let pairs = old_mid.len().min(new_mid.len());
    for index in 0..pairs {
        path.push(pair_edit(old_mid[index].2, new_mid[index].2));
    }
    for (_, _, text) in &old_mid[pairs..] {
        path.push(evaporated(text));
    }
    for (_, _, text) in &new_mid[pairs..] {
        path.push(ignited(text));
    }
    for (_, _, text) in &old[old.len() - suffix..] {
        path.push(kept(text));
    }
    path
}

fn pair_edit(old: &str, new: &str) -> TokenEdit {
    if old == new {
        return kept(new);
    }
    let (prefix, suffix) = char_prefix_suffix(old, new);
    if prefix == 0 && suffix == 0 {
        return ignited(new);
    }
    let changed = new.len() - prefix - suffix;
    if changed == 0 {
        return kept(new);
    }
    morphed(new, prefix, changed)
}

fn char_prefix_suffix(old: &str, new: &str) -> (usize, usize) {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let shorter = old.len().min(new.len());
    let mut suffix = 0;
    for (a, b) in old.chars().rev().zip(new.chars().rev()) {
        if a != b || prefix + suffix + a.len_utf8() > shorter {
            break;
        }
        suffix += a.len_utf8();
    }
    (prefix, suffix)
}

fn kept(text: &str) -> TokenEdit {
    TokenEdit {
        text: text.to_string(),
        fate: TokenFate::Kept,
        changed_range: None,
    }
}

fn ignited(text: &str) -> TokenEdit {
    TokenEdit {
        text: text.to_string(),
        fate: TokenFate::Ignited,
        changed_range: None,
    }
}

fn evaporated(text: &str) -> TokenEdit {
    TokenEdit {
        text: text.to_string(),
        fate: TokenFate::Evaporated,
        changed_range: None,
    }
}

fn morphed(text: &str, start: usize, len: usize) -> TokenEdit {
    TokenEdit {
        text: text.to_string(),
        fate: TokenFate::Morphed,
        changed_range: Some((start, len)),
    }
}

fn tokenize(text: &str) -> Vec<(usize, usize, &str)> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        let word = first.is_alphanumeric() || first == '_';
        let mut end = start + first.len_utf8();
        while let Some(&(next, ch)) = chars.peek() {
            if (ch.is_alphanumeric() || ch == '_') != word {
                break;
            }
            end = next + ch.len_utf8();
            chars.next();
        }
        tokens.push((start, end, &text[start..end]));
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::{token_edit_path, TokenEdit, TokenFate};

    fn fates(path: &[TokenEdit]) -> Vec<TokenFate> {
        path.iter().map(|edit| edit.fate).collect()
    }

    fn texts(path: &[TokenEdit]) -> Vec<&str> {
        path.iter().map(|edit| edit.text.as_str()).collect()
    }

    #[test]
    fn replacement_mid_line_ignites_the_changed_token() {
        let path = token_edit_path("left 1 right", "left 2 right");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Kept,
                TokenFate::Kept,
                TokenFate::Ignited,
                TokenFate::Kept,
                TokenFate::Kept,
            ]
        );
        assert_eq!(texts(&path), vec!["left", " ", "2", " ", "right"]);
        assert!(path.iter().all(|edit| edit.changed_range.is_none()));
    }

    #[test]
    fn insertion_ignites_only_the_new_tokens() {
        let path = token_edit_path("a b", "a X b");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Kept,
                TokenFate::Kept,
                TokenFate::Ignited,
                TokenFate::Ignited,
                TokenFate::Kept,
            ]
        );
        assert_eq!(texts(&path), vec!["a", " ", "X", " ", "b"]);
    }

    #[test]
    fn deletion_evaporates_the_vanished_tokens() {
        let path = token_edit_path("a X b", "a b");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Kept,
                TokenFate::Kept,
                TokenFate::Evaporated,
                TokenFate::Evaporated,
                TokenFate::Kept,
            ]
        );
        assert_eq!(texts(&path), vec!["a", " ", "X", " ", "b"]);
    }

    #[test]
    fn identical_edges_stay_kept() {
        let path = token_edit_path("start MID end", "start NEW end");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Kept,
                TokenFate::Kept,
                TokenFate::Ignited,
                TokenFate::Kept,
                TokenFate::Kept,
            ]
        );
        assert_eq!(texts(&path), vec!["start", " ", "NEW", " ", "end"]);
    }

    #[test]
    fn fallback_ignites_when_no_tokens_share_text() {
        let path = token_edit_path("xyz", "abc");
        assert_eq!(fates(&path), vec![TokenFate::Ignited]);
        assert_eq!(texts(&path), vec!["abc"]);
        assert_eq!(path[0].changed_range, None);
    }

    #[test]
    fn suffix_growth_morphs_with_appended_range() {
        let path = token_edit_path("counter", "county");
        assert_eq!(fates(&path), vec![TokenFate::Morphed]);
        assert_eq!(texts(&path), vec!["county"]);
        assert_eq!(path[0].changed_range, Some((5, 1)));
    }

    #[test]
    fn mid_word_edit_morphs_with_changed_range() {
        let path = token_edit_path("start ABC end", "start AXYC end");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Kept,
                TokenFate::Kept,
                TokenFate::Morphed,
                TokenFate::Kept,
                TokenFate::Kept,
            ]
        );
        assert_eq!(texts(&path), vec!["start", " ", "AXYC", " ", "end"]);
        assert_eq!(path[2].changed_range, Some((1, 2)));
    }

    #[test]
    fn identical_text_keeps_every_token() {
        let path = token_edit_path("same text", "same text");
        assert_eq!(
            fates(&path),
            vec![TokenFate::Kept, TokenFate::Kept, TokenFate::Kept]
        );
        assert_eq!(texts(&path), vec!["same", " ", "text"]);
    }

    #[test]
    fn empty_old_ignites_every_token() {
        let path = token_edit_path("", "brand new");
        assert_eq!(
            fates(&path),
            vec![TokenFate::Ignited, TokenFate::Ignited, TokenFate::Ignited]
        );
        assert_eq!(texts(&path), vec!["brand", " ", "new"]);
    }

    #[test]
    fn empty_new_evaporates_every_token() {
        let path = token_edit_path("gone forever", "");
        assert_eq!(
            fates(&path),
            vec![
                TokenFate::Evaporated,
                TokenFate::Evaporated,
                TokenFate::Evaporated
            ]
        );
    }

    #[test]
    fn utf8_word_edit_morphs_with_byte_range() {
        let path = token_edit_path("x\u{00E9}a", "x\u{00EA}a");
        assert_eq!(fates(&path), vec![TokenFate::Morphed]);
        assert_eq!(path[0].changed_range, Some((1, 2)));
    }

    #[test]
    fn path_preserves_the_new_text_across_fates() {
        for (old, new) in [
            ("left 1 right", "left 2 right"),
            ("a b", "a X b"),
            ("a X b", "a b"),
            ("start MID end", "start NEW end"),
            ("xyz", "abc"),
            ("counter", "county"),
            ("same text", "same text"),
            ("", "brand new"),
            ("gone forever", ""),
            ("x\u{00E9}a", "x\u{00EA}a"),
        ] {
            let path = token_edit_path(old, new);
            let joined: String = path
                .iter()
                .filter(|edit| edit.fate != TokenFate::Evaporated)
                .map(|edit| edit.text.as_str())
                .collect();
            assert_eq!(joined, new, "non-evaporated path must rebuild {new:?}");
            for edit in &path {
                if let Some((start, len)) = edit.changed_range {
                    assert!(len > 0);
                    assert!(start + len <= edit.text.len());
                }
            }
        }
    }
}
