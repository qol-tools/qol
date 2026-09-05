use plugin_cli_sessions::registry::summary_for;
use plugin_cli_sessions::status::Status;
use qol_gpui::theme::{CliSessionsPalette, DARK_SYSTEM, LIGHT_SYSTEM};
use qol_terminal_sessions::cli::claude_tool;

#[test]
fn every_state_has_one_semantic_color_attention_policy_and_order_in_both_themes() {
    for system in [DARK_SYSTEM, LIGHT_SYSTEM] {
        let palette = CliSessionsPalette::from_system(system);
        let expected = [
            (Status::NeedsYou, "needs you", system.danger, true, false),
            (Status::YourTurn, "your turn", system.warning, true, false),
            (Status::Working, "working", system.success, false, false),
            (
                Status::Coordinating,
                "coordinating agents",
                system.info,
                false,
                false,
            ),
            (
                Status::AwaitingReview,
                "awaiting agent review",
                system.info,
                false,
                false,
            ),
            (Status::Service, "live", system.info, false, false),
            (
                Status::Acknowledged,
                "acknowledged",
                system.text_faint,
                false,
                true,
            ),
            (Status::Unknown, "idle", system.text_faint, false, true),
        ];
        assert_eq!(Status::ALL, expected.map(|row| row.0));
        for (priority, (status, label, color, attention, idle)) in expected.into_iter().enumerate()
        {
            let definition = status.definition();
            assert_eq!(definition.priority as usize, priority);
            assert_eq!(definition.label, label);
            assert_eq!(summary_for(status, &claude_tool()), label);
            assert_eq!(status.is_attention(), attention);
            assert_eq!(definition.idle, idle);
            let (foreground, halo) = (definition.colors)(&palette);
            assert_eq!(foreground, color);
            if !idle {
                assert_eq!(halo >> 8, foreground);
            }
        }
    }
}
