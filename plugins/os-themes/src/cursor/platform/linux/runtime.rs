use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::{PlatformStateClient, Subscription};

use crate::config::Config;
use crate::cursor::{CursorEffect, RunControl};

use super::game_focus::{GameFocus, GameFocusDetector};
use super::motion::{MotionSample, ScaleEvent, ScaleUpdate, ShakeDetector};

const TICK_INTERVAL: Duration = Duration::from_millis(8);
const IDLE_WAKE_INTERVAL: Duration = Duration::from_millis(250);

pub fn create_effect() -> Box<dyn CursorEffect> {
    Box::new(LinuxCursorEffect)
}

struct LinuxCursorEffect;

impl CursorEffect for LinuxCursorEffect {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()> {
        if !config.enabled {
            return idle_until_stopped(control);
        }
        let mut session = open_session(config)?;
        let client = PlatformStateClient::from_env();
        let mut events = vec![RuntimeEventKind::CursorMoved];
        if config.pause_in_games {
            events.push(RuntimeEventKind::WindowListChanged);
        }
        let subscription = client
            .subscribe(events)
            .context("failed to subscribe to cursor events")?;
        let rx = spawn_reader(subscription);
        let game_focus = open_game_focus_detector(config.pause_in_games);
        let initial_focus = game_focus
            .as_ref()
            .map_or(GameFocus::inactive(), GameFocusDetector::probe);
        let mut state = EffectState::new(config, initial_focus.active);
        if initial_focus.active {
            log_game_focus(initial_focus);
        }
        loop {
            if control.should_stop() {
                break;
            }
            let timeout = if state.scaled {
                TICK_INTERVAL
            } else {
                IDLE_WAKE_INTERVAL
            };
            match rx.recv_timeout(timeout) {
                Ok(InputEvent::CursorMoved { at, x, y }) => {
                    state.record_cursor(&mut session, at, x, y);
                }
                Ok(InputEvent::WindowListChanged) => {
                    let focus = game_focus
                        .as_ref()
                        .map_or(GameFocus::inactive(), GameFocusDetector::probe);
                    if state.set_game_focus(&mut session, focus.active) {
                        log_game_focus(focus);
                    }
                }
                Err(RecvTimeoutError::Timeout) => state.tick(&mut session, Instant::now()),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        session.restore();
        Ok(())
    }
}

fn idle_until_stopped(control: &dyn RunControl) -> Result<()> {
    eprintln!("[shake-to-grow] disabled, idling");
    while !control.should_stop() {
        std::thread::sleep(IDLE_WAKE_INTERVAL);
    }
    Ok(())
}

enum InputEvent {
    CursorMoved { at: Instant, x: f32, y: f32 },
    WindowListChanged,
}

fn spawn_reader(mut subscription: Subscription) -> mpsc::Receiver<InputEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Some(event) = subscription.next_event() {
            let input = match event {
                RuntimeEvent::CursorMoved { x, y } => InputEvent::CursorMoved {
                    at: Instant::now(),
                    x,
                    y,
                },
                RuntimeEvent::WindowListChanged => InputEvent::WindowListChanged,
                RuntimeEvent::ActiveMonitorChanged { .. }
                | RuntimeEvent::FocusChanged { .. }
                | RuntimeEvent::LauncherAppsSynced { .. }
                | RuntimeEvent::MonitorsChanged { .. } => continue,
            };
            if tx.send(input).is_err() {
                break;
            }
        }
    });
    rx
}

struct EffectState {
    detector: ShakeDetector,
    last_pos: Option<(f32, f32)>,
    scaled: bool,
    pause_in_games: bool,
    game_focused: bool,
}

impl EffectState {
    fn new(config: &Config, game_focused: bool) -> Self {
        Self {
            detector: ShakeDetector::new(config),
            last_pos: None,
            scaled: false,
            pause_in_games: config.pause_in_games,
            game_focused: config.pause_in_games && game_focused,
        }
    }

    fn record_cursor(&mut self, session: &mut impl CursorSession, at: Instant, x: f32, y: f32) {
        if self.game_focused {
            self.last_pos = Some((x, y));
            return;
        }
        let (dx, dy) = delta(self.last_pos, x, y);
        self.last_pos = Some((x, y));
        let update = self.detector.record(MotionSample::new(at, dx, dy));
        self.apply(session, update);
    }

    fn tick(&mut self, session: &mut impl CursorSession, at: Instant) {
        if self.game_focused {
            return;
        }
        let update = self.detector.record(MotionSample::new(at, 0, 0));
        self.apply(session, update);
    }

    fn set_game_focus(&mut self, session: &mut impl CursorSession, game_focused: bool) -> bool {
        let game_focused = self.pause_in_games && game_focused;
        if game_focused == self.game_focused {
            return false;
        }
        self.game_focused = game_focused;
        self.last_pos = None;
        self.detector.reset();
        if self.scaled {
            session.restore();
            self.scaled = false;
        }
        true
    }

    fn apply(&mut self, session: &mut impl CursorSession, update: ScaleUpdate) {
        self.scaled = update
            .scale_changed
            .map_or(self.scaled, |scale| scale > 1.0 + f32::EPSILON);
        if self.scaled {
            session.refresh();
        }
        apply_update(session, update);
    }
}

fn delta(last: Option<(f32, f32)>, x: f32, y: f32) -> (i32, i32) {
    let Some((lx, ly)) = last else {
        return (0, 0);
    };
    ((x - lx) as i32, (y - ly) as i32)
}

fn open_session(config: &Config) -> Result<Session> {
    super::display::ensure_cursor_support()?;
    eprintln!("[shake-to-grow] started mode=tree");
    Ok(Session::Tree(super::display::x11::CursorSession::open(
        config.scale_factor,
    )?))
}

fn open_game_focus_detector(enabled: bool) -> Option<GameFocusDetector> {
    if !enabled {
        return None;
    }
    match GameFocusDetector::open() {
        Ok(detector) => Some(detector),
        Err(error) => {
            eprintln!("[shake-to-grow] game-focus detection unavailable: {error:#}");
            None
        }
    }
}

fn apply_update(session: &mut impl CursorSession, update: ScaleUpdate) -> bool {
    if let Some(event) = update.event {
        log_event(event);
    }
    if let Some(scale) = update.scale_changed {
        return session.set_scale(scale);
    }
    false
}

enum Session {
    Tree(super::display::x11::CursorSession),
}

trait CursorSession {
    fn set_scale(&mut self, scale: f32) -> bool;
    fn refresh(&mut self) -> bool;
    fn restore(&mut self);
}

impl CursorSession for Session {
    fn set_scale(&mut self, scale: f32) -> bool {
        match self {
            Self::Tree(session) => session.set_scale(scale),
        }
    }

    fn refresh(&mut self) -> bool {
        match self {
            Self::Tree(session) => session.refresh(),
        }
    }

    fn restore(&mut self) {
        match self {
            Self::Tree(session) => session.restore(),
        }
    }
}

fn log_event(event: ScaleEvent) {
    match event {
        ScaleEvent::Grew { tortuosity } => {
            eprintln!("[shake-to-grow] grow tortuosity={tortuosity:.1}")
        }
        ScaleEvent::Restored => eprintln!("[shake-to-grow] restore"),
    }
}

fn log_game_focus(focus: GameFocus) {
    if focus.active {
        let window = focus
            .window_id
            .map(|window| format!("0x{window:x}"))
            .unwrap_or_else(|| "unknown".to_string());
        let pid = focus
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        eprintln!(
            "[shake-to-grow] paused for focused game window={window} pid={pid} evidence={}",
            focus.evidence.unwrap_or("unknown")
        );
    } else {
        eprintln!("[shake-to-grow] resumed after game focus");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeSession {
        scales: Vec<f32>,
        refreshes: usize,
        restores: usize,
    }

    impl CursorSession for FakeSession {
        fn set_scale(&mut self, scale: f32) -> bool {
            self.scales.push(scale);
            true
        }

        fn refresh(&mut self) -> bool {
            self.refreshes += 1;
            true
        }

        fn restore(&mut self) {
            self.restores += 1;
        }
    }

    fn config(pause_in_games: bool) -> Config {
        Config {
            enabled: true,
            pause_in_games,
            shake_strictness: 6.5,
            regrow_strictness: 2.5,
            shake_min_extent_px: 150,
            regrow_min_extent_px: 60,
            shake_window_ms: 1000,
            scale_factor: 4,
            calm_duration_ms: 100,
            grow_ms: 250,
            shrink_ms: 225,
        }
    }

    fn shake(
        state: &mut EffectState,
        session: &mut FakeSession,
        started: Instant,
        duration_ms: u64,
    ) {
        let mut x = 0.0;
        let mut sign = 1.0;
        let mut elapsed = 0;
        while elapsed < duration_ms {
            for _ in 0..4 {
                elapsed += 16;
                x += 60.0 * sign;
                state.record_cursor(session, started + Duration::from_millis(elapsed), x, 0.0);
            }
            sign = -sign;
        }
    }

    #[test]
    fn focused_game_blocks_growth_and_restores_an_existing_growth() {
        let mut state = EffectState::new(&config(true), false);
        let mut session = FakeSession::default();
        let started = Instant::now();

        shake(&mut state, &mut session, started, 1000);
        assert!(state.scaled, "ordinary desktop shake must grow");
        assert!(
            session.scales.iter().any(|scale| *scale > 1.0),
            "ordinary desktop shake must reach the cursor session"
        );

        assert!(state.set_game_focus(&mut session, true));
        assert!(!state.scaled, "entering a game must clear scaled state");
        assert_eq!(
            session.restores, 1,
            "entering a game must immediately restore the cursor"
        );

        let scale_count = session.scales.len();
        shake(
            &mut state,
            &mut session,
            started + Duration::from_secs(2),
            2000,
        );
        state.tick(&mut session, started + Duration::from_secs(5));
        assert_eq!(
            session.scales.len(),
            scale_count,
            "game input must never reach cursor scaling"
        );
        assert_eq!(
            session.restores, 1,
            "stable game focus must not repeatedly restore"
        );

        assert!(state.set_game_focus(&mut session, false));
        shake(
            &mut state,
            &mut session,
            started + Duration::from_secs(6),
            1000,
        );
        assert!(state.scaled, "shake must work again after leaving the game");
    }

    #[test]
    fn disabled_game_pause_never_suppresses_motion() {
        let mut state = EffectState::new(&config(false), true);
        let mut session = FakeSession::default();

        assert!(!state.game_focused);
        assert!(!state.set_game_focus(&mut session, true));
        shake(&mut state, &mut session, Instant::now(), 1000);
        assert!(state.scaled);
        assert_eq!(session.restores, 0);
    }
}
