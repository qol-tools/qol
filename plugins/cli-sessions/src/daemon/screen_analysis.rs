use std::sync::Arc;

use qol_terminal_sessions::cli::{CliScreenEvidence, CliSessionInterpreter, CliTool};

use crate::host::Pane;
use crate::signal::screen::{screen_hash, stable_screen};

pub(super) struct ScreenAnalysis {
    pub text: String,
    pub hash: u64,
    pub evidence: CliScreenEvidence,
    pub pane: Pane,
    tool: CliTool,
}

impl ScreenAnalysis {
    pub fn refresh(
        previous: Option<&Arc<Self>>,
        text: String,
        pane: &Pane,
        tool: &CliTool,
        interpreter: &CliSessionInterpreter,
    ) -> Arc<Self> {
        if let Some(previous) = previous.filter(|previous| {
            previous.text == text && previous.pane == *pane && previous.tool == *tool
        }) {
            return previous.clone();
        }
        Arc::new(Self {
            hash: screen_hash(stable_screen(&text, tool).as_ref()),
            evidence: interpreter.classify_screen(pane, &text),
            text,
            pane: pane.clone(),
            tool: tool.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::cli::{claude_tool, codex_tool, pi_tool};

    fn pane() -> Pane {
        Pane {
            id: crate::host::kitty_session_id(1),
            root_pid: 1,
            cwd: "/project".into(),
            title: "agent".into(),
            at_prompt: false,
            reported_cmd: Some("pi".into()),
            foreground_basenames: vec!["pi".into()],
            foreground_pids: Vec::new(),
            capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    #[test]
    fn unchanged_screen_reuses_analysis_but_all_inputs_invalidate_it() {
        let interpreter = CliSessionInterpreter::system();
        let pane = pane();
        let tool = pi_tool();
        let text = "output\n──\n/tmp\n$0.00";
        let first = ScreenAnalysis::refresh(None, text.into(), &pane, &tool, &interpreter);
        let same = ScreenAnalysis::refresh(Some(&first), text.into(), &pane, &tool, &interpreter);
        assert!(Arc::ptr_eq(&first, &same));
        let footer = ScreenAnalysis::refresh(
            Some(&first),
            text.replace("$0.00", "$1.00"),
            &pane,
            &tool,
            &interpreter,
        );
        assert!(!Arc::ptr_eq(&first, &footer));
        assert_eq!(first.hash, footer.hash);
        for field in 0..4 {
            let mut changed = pane.clone();
            match field {
                0 => changed.root_pid += 1,
                1 => changed.at_prompt = true,
                2 => changed.foreground_basenames = vec!["codex".into()],
                _ => changed.title = "changed".into(),
            }
            let next =
                ScreenAnalysis::refresh(Some(&first), text.into(), &changed, &tool, &interpreter);
            assert!(!Arc::ptr_eq(&first, &next));
            assert_eq!(next.evidence, interpreter.classify_screen(&changed, text));
        }
        let changed = ScreenAnalysis::refresh(
            Some(&first),
            text.into(),
            &pane,
            &codex_tool(),
            &interpreter,
        );
        assert!(!Arc::ptr_eq(&first, &changed));
    }

    #[test]
    #[ignore = "manual screen-analysis performance measurement"]
    fn benchmark_unchanged_screen_analysis() {
        use std::hint::black_box;
        use std::time::Instant;

        let interpreter = CliSessionInterpreter::system();
        let cases = [
            (
                claude_tool(),
                include_str!("../../tests/fixtures/claude_real/working_win1.txt"),
            ),
            (codex_tool(), "✦ Working … (2s)\nesc to interrupt"),
            (
                pi_tool(),
                include_str!("../../tests/fixtures/corpus/pi_embedded_working.txt"),
            ),
        ];
        let mut measurements = Vec::new();
        for (tool, text) in cases {
            let mut pane = pane();
            pane.foreground_basenames = vec![tool.id.to_string()];
            pane.reported_cmd = Some(tool.id.to_string());
            let baseline = ScreenAnalysis::refresh(None, text.into(), &pane, &tool, &interpreter);
            let iterations = 2000;
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(ScreenAnalysis::refresh(
                    None,
                    black_box(text.to_owned()),
                    &pane,
                    &tool,
                    &interpreter,
                ));
            }
            let uncached_ns = start.elapsed().as_nanos();
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(ScreenAnalysis::refresh(
                    Some(&baseline),
                    black_box(text.to_owned()),
                    &pane,
                    &tool,
                    &interpreter,
                ));
            }
            let cached_ns = start.elapsed().as_nanos();
            measurements.push(serde_json::json!({
                "harness": tool.id.as_str(), "screen_bytes": text.len(), "iterations": iterations,
                "uncached_ns": uncached_ns, "cached_ns": cached_ns,
            }));
        }
        println!(
            "SCREEN_ANALYSIS_BENCHMARK={}",
            serde_json::json!({"measurements": measurements})
        );
    }
}
