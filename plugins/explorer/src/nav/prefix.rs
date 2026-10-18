pub(crate) fn starts_with_ignoring_case(name: &str, prefix: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    prefix
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| name.next() == Some(wanted))
}
