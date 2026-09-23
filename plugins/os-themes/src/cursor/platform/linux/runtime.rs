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
        let game_focus =
            open_game_focus_detector(config.pause_in_games || config.pause_in_fullscreen);
        let initial_focus = game_focus
            .as_ref()
            .map_or(GameFocus::inactive(), GameFocusDetector::probe);
        let mut state = EffectState::new(config, initial_focus.active);
        if initial_focus.active {
            log_game_focus(initial_focus);
        }
        let fullscreen = || {
            game_focus
                .as_ref()
                .is_some_and(GameFocusDetector::active_window_is_fullscreen)
        };
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
                    state.record_cursor(&mut session, at, x, y, &fullscreen);
                }
                Ok(InputEvent::WindowListChanged) => {
                    let focus = game_focus
                        .as_ref()
                        .map_or(GameFocus::inactive(), GameFocusDetector::probe);
                    if state.set_game_focus(&mut session, focus.active) {
                        log_game_focus(focus);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    state.tick(&mut session, Instant::now(), &fullscreen)
                }
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
    pause_in_fullscreen: bool,
    game_focused: bool,
}

impl EffectState {
    fn new(config: &Config, game_focused: bool) -> Self {
        Self {
            detector: ShakeDetector::new(config),
            last_pos: None,
            scaled: false,
            pause_in_games: config.pause_in_games,
            pause_in_fullscreen: config.pause_in_fullscreen,
            game_focused: config.pause_in_games && game_focused,
        }
    }

    fn record_cursor(
        &mut self,
        session: &mut impl CursorSession,
        at: Instant,
        x: f32,
        y: f32,
        fullscreen: &dyn Fn() -> bool,
    ) {
        if self.game_focused {
            self.last_pos = Some((x, y));
            return;
        }
        let (dx, dy) = delta(self.last_pos, x, y);
        self.last_pos = Some((x, y));
        let update = self.detector.record(MotionSample::new(at, dx, dy));
        self.apply(session, update, fullscreen);
    }

    fn tick(
        &mut self,
        session: &mut impl CursorSession,
        at: Instant,
        fullscreen: &dyn Fn() -> bool,
    ) {
        if self.game_focused {
            return;
        }
        let update = self.detector.record(MotionSample::new(at, 0, 0));
        self.apply(session, update, fullscreen);
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

    fn apply(
        &mut self,
        session: &mut impl CursorSession,
        update: ScaleUpdate,
        fullscreen: &dyn Fn() -> bool,
    ) {
        let was_scaled = self.scaled;
        self.scaled = update
            .scale_changed
            .map_or(self.scaled, |scale| scale > 1.0 + f32::EPSILON);
        if matches!(update.event, Some(ScaleEvent::Grew { .. })) && !was_scaled {
            let reason = if session.live_cursor_hidden() {
                Some("hidden_cursor")
            } else if self.pause_in_fullscreen && fullscreen() {
                Some("fullscreen")
            } else {
                None
            };
            if let Some(reason) = reason {
                self.detector.reset();
                self.scaled = false;
                eprintln!("[shake-to-grow] grow suppressed reason={reason}");
                return;
            }
        }
        session.refresh();
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
    fn live_cursor_hidden(&mut self) -> bool;
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

    fn live_cursor_hidden(&mut self) -> bool {
        match self {
            Self::Tree(session) => session.live_cursor_hidden(),
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
        hidden: bool,
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

        fn live_cursor_hidden(&mut self) -> bool {
            self.hidden
        }
    }

    fn config(pause_in_games: bool) -> Config {
        Config {
            enabled: true,
            pause_in_games,
            pause_in_fullscreen: true,
            tuning_revision: 1,
            shake_strictness: 5.0,
            regrow_strictness: 2.5,
            shake_min_extent_px: 150,
            regrow_min_extent_px: 60,
            shake_window_ms: 600,
            scale_factor: 4,
            calm_duration_ms: 100,
            grow_ms: 120,
            shrink_ms: 225,
        }
    }

    fn shake(
        state: &mut EffectState,
        session: &mut FakeSession,
        started: Instant,
        duration_ms: u64,
        fullscreen: &dyn Fn() -> bool,
    ) {
        let mut x = 0.0;
        let mut sign = 1.0;
        let mut elapsed = 0;
        while elapsed < duration_ms {
            for _ in 0..4 {
                elapsed += 16;
                x += 60.0 * sign;
                state.record_cursor(
                    session,
                    started + Duration::from_millis(elapsed),
                    x,
                    0.0,
                    fullscreen,
                );
            }
            sign = -sign;
        }
    }

    #[test]
    fn focused_game_blocks_growth_and_restores_an_existing_growth() {
        let mut state = EffectState::new(&config(true), false);
        let mut session = FakeSession::default();
        let started = Instant::now();

        shake(&mut state, &mut session, started, 1000, &|| false);
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
            &|| false,
        );
        state.tick(&mut session, started + Duration::from_secs(5), &|| false);
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
            &|| false,
        );
        assert!(state.scaled, "shake must work again after leaving the game");
    }

    #[test]
    fn disabled_game_pause_never_suppresses_motion() {
        let mut state = EffectState::new(&config(false), true);
        let mut session = FakeSession::default();

        assert!(!state.game_focused);
        assert!(!state.set_game_focus(&mut session, true));
        shake(&mut state, &mut session, Instant::now(), 1000, &|| false);
        assert!(state.scaled);
        assert_eq!(session.restores, 0);
    }

    #[test]
    fn hidden_cursor_never_grows() {
        let mut control_state = EffectState::new(&config(true), false);
        let mut control_session = FakeSession::default();
        shake(
            &mut control_state,
            &mut control_session,
            Instant::now(),
            1000,
            &|| false,
        );
        assert!(
            control_state.scaled,
            "control case without a hidden cursor must grow"
        );

        let mut state = EffectState::new(&config(true), false);
        let mut session = FakeSession {
            hidden: true,
            ..FakeSession::default()
        };
        let started = Instant::now();
        shake(&mut state, &mut session, started, 1000, &|| false);
        assert!(
            session.scales.is_empty(),
            "a hidden live cursor must never reach set_scale"
        );
        assert!(!state.scaled, "a hidden live cursor must not grow");

        shake(
            &mut state,
            &mut session,
            started + Duration::from_millis(1200),
            1000,
            &|| false,
        );
        assert!(
            session.scales.is_empty(),
            "a second identical trace must still not grow within the window"
        );
        assert!(!state.scaled);
    }

    #[test]
    fn fullscreen_focus_never_grows() {
        let mut state = EffectState::new(&config(true), false);
        let mut session = FakeSession::default();
        shake(&mut state, &mut session, Instant::now(), 1000, &|| true);
        assert!(session.scales.is_empty(), "fullscreen must never grow");
        assert!(!state.scaled);
    }

    #[test]
    fn fullscreen_probe_ignored_when_disabled() {
        let mut config = config(true);
        config.pause_in_fullscreen = false;
        let mut state = EffectState::new(&config, false);
        let mut session = FakeSession::default();
        shake(&mut state, &mut session, Instant::now(), 1000, &|| true);
        assert!(
            session.scales.iter().any(|scale| *scale > 1.0),
            "with pause_in_fullscreen disabled a fullscreen window must not block growth"
        );
        assert!(state.scaled);
    }

    #[test]
    fn guards_are_not_consulted_while_shrinking() {
        let calls = std::cell::RefCell::new(0usize);
        let probe = || {
            *calls.borrow_mut() += 1;
            false
        };
        let mut state = EffectState::new(&config(true), false);
        let mut session = FakeSession::default();
        let started = Instant::now();
        shake(&mut state, &mut session, started, 1000, &probe);
        assert!(state.scaled, "plain grow with an empty probe must proceed");

        let mut ms = 1000;
        while state.scaled {
            ms += 16;
            state.tick(&mut session, started + Duration::from_millis(ms), &probe);
        }
        assert_eq!(
            *calls.borrow(),
            1,
            "the grow guard must be consulted exactly once, at the grow transition"
        );
    }
}
