use qol_terminal_sessions::cli::{activity_signature, editor_draft, provider_error_line};

const PROMPT_ECHO: &str = include_str!("fixtures/pi_real/prompt_echo_with_token.txt");
const PROVIDER_ERROR: &str = include_str!("fixtures/pi_real/provider_error_terminated.txt");
const FROZEN_A: &str = include_str!("fixtures/pi_real/frozen_spinner_a.txt");
const FROZEN_B: &str = include_str!("fixtures/pi_real/frozen_spinner_b.txt");
const TOKEN_IN_EDITOR: &str = include_str!("fixtures/pi_real/token_in_editor.txt");
const COMPLETION_LINE: &str = include_str!("fixtures/pi_real/completion_line.txt");

#[test]
fn a_provider_error_is_read_off_the_lane_tail() {
    assert_eq!(
        provider_error_line(PROVIDER_ERROR),
        Some("Error: terminated")
    );
    assert_eq!(provider_error_line(FROZEN_A), Some("Error: terminated"));
}

#[test]
fn a_lane_that_kept_working_after_an_error_is_not_faulted() {
    for screen in [PROMPT_ECHO, COMPLETION_LINE] {
        assert_eq!(provider_error_line(screen), None);
    }
    assert_eq!(provider_error_line("Error: "), None);
    assert_eq!(provider_error_line("no error here"), None);
}

#[test]
fn a_rotating_spinner_does_not_move_the_activity_signature() {
    assert_ne!(FROZEN_A, FROZEN_B);
    assert_eq!(activity_signature(FROZEN_A), activity_signature(FROZEN_B));
}

#[test]
fn real_output_still_moves_the_activity_signature() {
    assert_ne!(
        activity_signature(FROZEN_A),
        activity_signature(COMPLETION_LINE)
    );
    assert_ne!(
        activity_signature(FROZEN_A),
        activity_signature(TOKEN_IN_EDITOR)
    );
}

#[test]
fn an_unsent_kickstart_is_visible_as_an_editor_draft() {
    let draft = editor_draft(TOKEN_IN_EDITOR).expect("the editor holds the unsent kickstart");
    assert!(draft.contains("QOL_BRIDGE_DONE_9a62a1a69874fa89a31a"));
    assert!(draft.contains("[qol session bridge]"));
}

#[test]
fn an_empty_editor_has_no_draft() {
    for screen in [PROVIDER_ERROR, FROZEN_A, COMPLETION_LINE, PROMPT_ECHO] {
        assert_eq!(editor_draft(screen), None);
    }
}
