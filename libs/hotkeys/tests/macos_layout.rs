#[cfg(target_os = "macos")]
fn main() {
    use qol_hotkeys::layout::{KeyLayout, OPTION_STATE, SHIFT_STATE};

    let danish = KeyLayout::by_id("com.apple.keylayout.Danish").expect("Danish layout");
    let us = KeyLayout::by_id("com.apple.keylayout.US").expect("US layout");
    let typed = |layout: &KeyLayout, code: u16, state: u32| {
        let mut dead = 0;
        layout.translate(code, state, &mut dead)
    };

    assert_eq!(danish.id(), "com.apple.keylayout.Danish");
    assert_eq!(typed(&danish, 0x2A, OPTION_STATE).as_deref(), Some("@"));
    assert_eq!(typed(&danish, 0x15, SHIFT_STATE).as_deref(), Some("€"));
    assert_eq!(typed(&danish, 0x0A, 0).as_deref(), Some("$"));
    assert_eq!(typed(&us, 0x13, SHIFT_STATE).as_deref(), Some("@"));

    let mut dead = 0;
    assert_eq!(
        danish.translate(0x1E, OPTION_STATE, &mut dead).as_deref(),
        Some("")
    );
    assert_ne!(dead, 0, "option+¨ is a dead key on Danish");
    assert_eq!(danish.translate(0x31, 0, &mut dead).as_deref(), Some("~"));

    assert!(KeyLayout::by_id("com.example.no-such-layout").is_err());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
