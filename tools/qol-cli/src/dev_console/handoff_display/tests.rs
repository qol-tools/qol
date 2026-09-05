use super::*;
use crate::dev_console::dash::ReloadProgress;
use crate::dev_console::testkit::render_text;
use std::time::Duration;

fn handoff_dash() -> Dash {
    let mut dash = Dash::new(Vec::new());
    let mut activity = ReloadProgress::new();
    activity.started = Instant::now() - Duration::from_secs(8);
    activity.phase = "handoff".to_string();
    activity.detail = "successor generation".to_string();
    dash.reload = Reload::Handoff { activity };
    dash
}

fn displayed_seconds(frame: &str) -> u64 {
    let timer = frame
        .split("successor generation · ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let (minutes, seconds) = timer.trim_end_matches('s').split_once('m').unwrap();
    minutes.parse::<u64>().unwrap() * 60 + seconds.parse::<u64>().unwrap()
}

#[test]
fn blocking_handoff_keeps_rendering_elapsed_time_on_the_original_clock() {
    let mut dash = handoff_dash();
    let owner = std::thread::current().id();
    let (frames, observed) = mpsc::channel();
    let result = run(
        &mut dash,
        |dash| {
            assert!(dash.is_reloading());
            frames.send(render_text(dash)).unwrap();
            Ok(())
        },
        |updates| {
            assert_eq!(std::thread::current().id(), owner);
            updates.phase("start", "successor generation");
            let first = observed.recv_timeout(Duration::from_secs(5)).unwrap();
            let first_seconds = displayed_seconds(&first);
            assert!(first_seconds >= 8, "{first}");
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut frame_count = 1;
            loop {
                let frame = observed
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                frame_count += 1;
                let seconds = displayed_seconds(&frame);
                if seconds >= first_seconds + 2 {
                    assert!(frame.contains("start · successor generation"));
                    println!(
                        "{}",
                        serde_json::json!({
                            "timer_start_secs": first_seconds,
                            "timer_end_secs": seconds,
                            "rendered_frames": frame_count,
                            "handoff_on_owner_thread": true,
                        })
                    );
                    break;
                }
            }
            updates.adopt_running_worktree(PathBuf::from("/qol/successor"));
            updates.push_log("handoff fixture completed");
            42
        },
    )
    .unwrap();

    assert_eq!(result, 42);
    assert!(!dash.is_reloading());
    assert_eq!(dash.running_worktree, PathBuf::from("/qol/successor"));
    assert!(dash.pokes.doctor && dash.pokes.links && dash.pokes.emu);
    assert!(dash
        .logs
        .ring
        .lines
        .iter()
        .any(|line| line == "handoff fixture completed"));
}

#[test]
fn failed_handoff_preserves_its_error_and_finishes_the_display() {
    let mut dash = handoff_dash();
    let result = run(
        &mut dash,
        |_| Ok(()),
        |updates| {
            updates.phase("promote", "successor generation");
            updates.push_log("promotion fixture failed");
            Err::<(), _>(anyhow!("promotion fixture failed"))
        },
    )
    .unwrap();

    assert_eq!(result.unwrap_err().to_string(), "promotion fixture failed");
    assert!(!dash.is_reloading());
    assert_eq!(dash.running_worktree, PathBuf::from("/qol/base"));
}

#[test]
fn display_failure_does_not_skip_handoff_cleanup() {
    let mut dash = handoff_dash();
    let mut completed = false;
    let result = run(
        &mut dash,
        |_| Err(anyhow!("terminal fixture failed")),
        |_| completed = true,
    );

    assert!(completed);
    assert_eq!(result.unwrap_err().to_string(), "terminal fixture failed");
    assert!(!dash.is_reloading());
}
