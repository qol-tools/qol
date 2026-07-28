use plugin_cli_sessions::signal::screen::{
    claude_awaiting_choice, claude_done, claude_working, has_numbered_choice_prompt,
};

const WORKING_WITH_TASKLIST: &str = include_str!("fixtures/claude_real/working_win1.txt");

#[test]
fn live_spinner_is_detected_even_below_a_rendered_tasklist() {
    assert!(
        claude_working(WORKING_WITH_TASKLIST),
        "the live spinner line (Verb… (Ns)) must read as working even when a multi-line \
         tool result renders below it and pushes it past the bottom-line window"
    );
    assert!(
        !claude_awaiting_choice(WORKING_WITH_TASKLIST),
        "a working frame must not read as an awaiting-choice prompt"
    );
    assert!(
        !has_numbered_choice_prompt(WORKING_WITH_TASKLIST),
        "a working frame must not read as a numbered choice prompt"
    );
    assert!(
        !claude_done(WORKING_WITH_TASKLIST),
        "a working frame is not done"
    );
}
