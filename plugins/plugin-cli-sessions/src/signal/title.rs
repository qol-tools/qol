pub fn title_working(title: &str) -> bool {
    matches!(
        title.trim_start().chars().next(),
        Some(c) if is_braille(c) || is_busy_star(c)
    )
}

fn is_braille(c: char) -> bool {
    let cp = c as u32;
    (0x2800..=0x28FF).contains(&cp)
}

fn is_busy_star(c: char) -> bool {
    let cp = c as u32;
    (0x2734..=0x273F).contains(&cp)
}
