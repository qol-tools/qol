pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::plural;

    #[test]
    fn plural_picks_the_singular_only_for_one() {
        for (count, expected) in [(0, "0 items"), (1, "1 item"), (2, "2 items")] {
            assert_eq!(plural(count, "item", "items"), expected, "count: {count}");
        }
    }
}
