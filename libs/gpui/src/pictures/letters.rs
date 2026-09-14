pub fn letters_for(labels: &[&str]) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(labels.len());
    for label in labels {
        let base = base_letters(label);
        let letters = if used.contains(&base) {
            match alternative_letters(label) {
                Some(alternative) if !used.contains(&alternative) => alternative,
                _ => base,
            }
        } else {
            base
        };
        used.push(letters.clone());
        out.push(letters);
    }
    out
}

fn base_letters(label: &str) -> String {
    let mut words = label.split_whitespace();
    if let (Some(first), Some(second)) = (words.next(), words.next()) {
        return format!("{}{}", upper_first(first), upper_first(second));
    }
    let word = label.split_whitespace().next().unwrap_or("");
    let first_two: String = word
        .chars()
        .filter(|ch| ch.is_uppercase())
        .take(2)
        .collect();
    if first_two.chars().count() >= 2 {
        first_two
    } else {
        upper_first(word)
    }
}

fn alternative_letters(label: &str) -> Option<String> {
    let word = label.split_whitespace().next()?;
    let mut chars = word.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    Some(format!("{}{}", upper(first), lower(second)))
}

fn upper_first(word: &str) -> String {
    word.chars().next().map(upper).unwrap_or_default()
}

fn upper(ch: char) -> String {
    ch.to_uppercase().collect()
}

fn lower(ch: char) -> String {
    ch.to_lowercase().collect()
}
