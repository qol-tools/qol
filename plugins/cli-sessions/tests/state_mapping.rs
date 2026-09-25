use qol_cli_sessions::registry::summary_for;
use qol_cli_sessions::status::Status;
use qol_gpui::kit::Kit;
use qol_gpui::theme::{ThemeMode, DARK_SYSTEM, LIGHT_SYSTEM};
use qol_terminal_sessions::cli::claude_tool;

#[test]
fn every_state_has_one_semantic_color_attention_policy_and_order_in_both_themes() {
    for (mode, system) in [
        (ThemeMode::Dark, DARK_SYSTEM),
        (ThemeMode::Light, LIGHT_SYSTEM),
    ] {
        let kit = Kit::new(mode, system);
        let expected = [
            (Status::NeedsYou, "needs you", system.danger, true, false),
            (Status::YourTurn, "your turn", system.warning, true, false),
            (
                Status::AwaitingReview,
                "awaiting agent review",
                system.info,
                false,
                false,
            ),
            (
                Status::Coordinating,
                "coordinating agents",
                system.info,
                false,
                false,
            ),
            (Status::Working, "working", system.success, false, false),
            (Status::Service, "live", system.info, false, false),
            (Status::Unknown, "idle", system.text_muted, false, true),
            (
                Status::Acknowledged,
                "acknowledged",
                system.text_muted,
                false,
                true,
            ),
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
            let (foreground, halo) = (definition.colors)(&kit);
            assert_eq!(foreground, color);
            if !idle {
                assert_eq!(halo >> 8, foreground);
            }
        }
    }
}

#[test]
fn only_active_states_animate() {
    for status in Status::ALL {
        assert_eq!(
            status.is_active(),
            matches!(
                status,
                Status::Working | Status::Coordinating | Status::Service
            )
        );
        assert!(!(status.is_active() && status.is_attention()));
    }
}
