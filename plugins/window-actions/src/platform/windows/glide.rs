use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qol_windowing::platform::windows::Window;
use qol_windowing::{WindowId, WindowRect};

use crate::glide::{Direction, Phase};

const WATCHDOG: Duration = Duration::from_millis(1000);
const TICK: Duration = Duration::from_millis(16);
const MAX_STEP_SECONDS: f64 = 0.05;
const FIRST_STEP_SECONDS: f64 = 1.0 / 60.0;
const MIN_SPEED: f64 = 100.0;
const MAX_SPEED: f64 = 4000.0;
const DIAGONAL: f64 = std::f64::consts::FRAC_1_SQRT_2;
const DIRECTIONS: [Direction; 4] = [
    Direction::Left,
    Direction::Right,
    Direction::Up,
    Direction::Down,
];

trait Mover {
    fn frame(&self, id: &WindowId) -> Option<WindowRect>;
    fn place(&self, id: &WindowId, frame: WindowRect) -> bool;
}

struct Win32Mover;

impl Mover for Win32Mover {
    fn frame(&self, id: &WindowId) -> Option<WindowRect> {
        Window::from_id(id)
            .filter(|window| window.exists())
            .and_then(Window::frame)
    }

    fn place(&self, id: &WindowId, frame: WindowRect) -> bool {
        Window::from_id(id).is_some_and(|window| window.set_frame(frame).is_ok())
    }
}

#[derive(Default)]
struct Motion {
    directions: HashMap<Direction, u64>,
    sequence: u64,
}

impl Motion {
    fn press(&mut self, direction: Direction) {
        self.sequence += 1;
        self.directions.insert(direction, self.sequence);
    }

    fn release(&mut self, direction: Direction) {
        self.directions.remove(&direction);
    }

    fn is_empty(&self) -> bool {
        self.directions.is_empty()
    }

    fn vector(&self) -> (f64, f64) {
        let axis = |negative: Direction, positive: Direction| {
            let negative = self.directions.get(&negative).copied().unwrap_or(0);
            let positive = self.directions.get(&positive).copied().unwrap_or(0);
            match positive.cmp(&negative) {
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Less => -1.0,
            }
        };
        let dx = axis(Direction::Left, Direction::Right);
        let dy = axis(Direction::Up, Direction::Down);
        if dx != 0.0 && dy != 0.0 {
            (dx * DIAGONAL, dy * DIAGONAL)
        } else {
            (dx, dy)
        }
    }

    fn active(&self) -> String {
        let mut held: Vec<(u64, Direction)> = DIRECTIONS
            .into_iter()
            .filter_map(|direction| Some((*self.directions.get(&direction)?, direction)))
            .collect();
        held.sort_by_key(|(sequence, _)| *sequence);
        let names: Vec<String> = held
            .into_iter()
            .map(|(sequence, direction)| format!("{}:{sequence}", direction.as_str()))
            .collect();
        if names.is_empty() {
            "none".into()
        } else {
            names.join(",")
        }
    }
}

struct Target {
    id: WindowId,
    x: f64,
    y: f64,
    last_tick: Instant,
}

struct State {
    motion: Motion,
    target: Option<Target>,
    speed: f64,
    expires_at: Instant,
}

impl State {
    fn new() -> Self {
        Self {
            motion: Motion::default(),
            target: None,
            speed: MIN_SPEED,
            expires_at: Instant::now(),
        }
    }

    fn step(&mut self, now: Instant, elapsed: f64, mover: &impl Mover) -> bool {
        if self.motion.is_empty() || now > self.expires_at {
            return false;
        }
        let (dx, dy) = self.motion.vector();
        let speed = self.speed;
        let Some(target) = self.target.as_mut() else {
            return false;
        };
        target.last_tick = now;
        if dx == 0.0 && dy == 0.0 {
            return true;
        }
        let Some(frame) = mover.frame(&target.id) else {
            return false;
        };
        let next_x = target.x + dx * speed * elapsed;
        let next_y = target.y + dy * speed * elapsed;
        let goal = WindowRect {
            x: next_x.round(),
            y: next_y.round(),
            ..frame
        };
        if !mover.place(&target.id, goal) {
            return false;
        }
        let actual = mover.frame(&target.id).unwrap_or(goal);
        target.x = if actual.x == goal.x { next_x } else { actual.x };
        target.y = if actual.y == goal.y { next_y } else { actual.y };
        true
    }

    fn reset(&mut self) {
        self.motion = Motion::default();
        self.target = None;
    }

    fn observation(&self) -> String {
        let (dx, dy) = self.motion.vector();
        let position = self.target.as_ref().map_or_else(
            || "unknown".to_string(),
            |target| format!("{},{}", target.x.round(), target.y.round()),
        );
        let target = self
            .target
            .as_ref()
            .map_or("none", |target| target.id.as_str());
        format!(
            "active={} vector={dx},{dy} position={position} target={target}",
            self.motion.active()
        )
    }
}

pub(crate) struct GlideController {
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        qol_windowing::platform::windows::ensure_dpi_awareness();
        Ok(Self {
            state: Arc::new(Mutex::new(State::new())),
            worker: None,
        })
    }

    pub(crate) fn update(
        &mut self,
        direction: Direction,
        phase: Phase,
        speed: f64,
    ) -> Result<String, String> {
        match phase {
            Phase::Start => self.start(direction, speed),
            Phase::Heartbeat => Ok(self.heartbeat(speed)),
            Phase::Stop => Ok(self.stop(direction)),
        }
    }

    pub(crate) fn stop_all(&mut self) -> Result<(), String> {
        self.lock().reset();
        self.join_worker();
        Ok(())
    }

    pub(crate) fn maintain(&mut self) -> Option<Result<(), String>> {
        let expired = {
            let state = self.lock();
            !state.motion.is_empty() && Instant::now() > state.expires_at
        };
        expired.then(|| self.stop_all())
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.lock().motion.is_empty()
    }

    fn start(&mut self, direction: Direction, speed: f64) -> Result<String, String> {
        let mut state = self.lock();
        if state.target.is_none() {
            let window = Window::foreground().ok_or("No focused window")?;
            if !window.is_switchable() {
                return Err("Focused surface is not an app window".into());
            }
            let frame = window.frame().ok_or("Cannot read window geometry")?;
            state.target = Some(Target {
                id: window.id(),
                x: frame.x,
                y: frame.y,
                last_tick: Instant::now(),
            });
        }
        let now = Instant::now();
        state.motion.press(direction);
        state.speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        state.expires_at = now + WATCHDOG;
        if !state.step(now, FIRST_STEP_SECONDS, &Win32Mover) {
            state.reset();
            return Err("The focused window can no longer be moved".into());
        }
        let observation = state.observation();
        drop(state);
        self.ensure_worker();
        Ok(observation)
    }

    fn heartbeat(&mut self, speed: f64) -> String {
        let mut state = self.lock();
        if state.motion.is_empty() {
            return "active=none reason=no-active-glide".into();
        }
        state.speed = speed.clamp(MIN_SPEED, MAX_SPEED);
        state.expires_at = Instant::now() + WATCHDOG;
        state.observation()
    }

    fn stop(&mut self, direction: Direction) -> String {
        let mut state = self.lock();
        state.motion.release(direction);
        let observation = state.observation();
        let finished = state.motion.is_empty();
        if finished {
            state.reset();
        }
        drop(state);
        if finished {
            self.join_worker();
        }
        observation
    }

    fn ensure_worker(&mut self) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return;
        }
        self.join_worker();
        let state = Arc::clone(&self.state);
        self.worker = std::thread::Builder::new()
            .name("window-actions-glide".into())
            .spawn(move || run(&state))
            .ok();
    }

    fn join_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }
}

impl Drop for GlideController {
    fn drop(&mut self) {
        let _ = self.stop_all();
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run(state: &Mutex<State>) {
    loop {
        std::thread::sleep(TICK);
        let mut state = lock(state);
        let now = Instant::now();
        let elapsed = state.target.as_ref().map_or(0.0, |target| {
            now.duration_since(target.last_tick)
                .as_secs_f64()
                .min(MAX_STEP_SECONDS)
        });
        if !state.step(now, elapsed, &Win32Mover) {
            state.reset();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct FakeMover {
        frame: RefCell<WindowRect>,
        clamp_x: Option<f64>,
    }

    impl Mover for FakeMover {
        fn frame(&self, _id: &WindowId) -> Option<WindowRect> {
            Some(*self.frame.borrow())
        }

        fn place(&self, _id: &WindowId, frame: WindowRect) -> bool {
            let x = self.clamp_x.map_or(frame.x, |limit| frame.x.min(limit));
            *self.frame.borrow_mut() = WindowRect { x, ..frame };
            true
        }
    }

    fn mover(clamp_x: Option<f64>) -> FakeMover {
        FakeMover {
            frame: RefCell::new(WindowRect {
                x: 100.0,
                y: 200.0,
                width: 800.0,
                height: 600.0,
            }),
            clamp_x,
        }
    }

    fn gliding(directions: &[Direction], speed: f64) -> State {
        let now = Instant::now();
        let mut state = State::new();
        for direction in directions {
            state.motion.press(*direction);
        }
        state.speed = speed;
        state.expires_at = now + WATCHDOG;
        state.target = Some(Target {
            id: WindowId::from_u32(1),
            x: 100.0,
            y: 200.0,
            last_tick: now,
        });
        state
    }

    #[test]
    fn the_most_recent_direction_wins_on_each_axis() {
        use Direction::{Down, Left, Right, Up};
        let cases: [(&[Direction], (f64, f64)); 6] = [
            (&[Left], (-1.0, 0.0)),
            (&[Left, Right], (1.0, 0.0)),
            (&[Right, Left], (-1.0, 0.0)),
            (&[Up], (0.0, -1.0)),
            (&[Down, Right], (DIAGONAL, DIAGONAL)),
            (&[], (0.0, 0.0)),
        ];
        for (pressed, expected) in cases {
            let mut motion = Motion::default();
            for direction in pressed {
                motion.press(*direction);
            }
            assert_eq!(motion.vector(), expected, "{pressed:?}");
        }
    }

    #[test]
    fn a_step_moves_by_speed_times_elapsed_and_keeps_the_size() {
        let fake = mover(None);
        let mut state = gliding(&[Direction::Right], 1200.0);
        assert!(state.step(Instant::now(), 0.05, &fake));
        let frame = *fake.frame.borrow();
        assert_eq!((frame.x, frame.y), (160.0, 200.0));
        assert_eq!((frame.width, frame.height), (800.0, 600.0));
    }

    #[test]
    fn a_constrained_move_resynchronizes_to_the_real_position() {
        let fake = mover(Some(120.0));
        let mut state = gliding(&[Direction::Right], 1200.0);
        assert!(state.step(Instant::now(), 0.05, &fake));
        assert_eq!(state.target.as_ref().map(|target| target.x), Some(120.0));
    }

    #[test]
    fn released_or_expired_glides_stop_stepping() {
        let fake = mover(None);
        let mut released = gliding(&[Direction::Left], 1200.0);
        released.motion.release(Direction::Left);
        assert!(!released.step(Instant::now(), 0.016, &fake));

        let mut expired = gliding(&[Direction::Left], 1200.0);
        let later = Instant::now() + WATCHDOG + Duration::from_millis(1);
        assert!(!expired.step(later, 0.016, &fake));
    }

    #[test]
    fn observations_list_held_directions_in_press_order() {
        let mut motion = Motion::default();
        motion.press(Direction::Up);
        motion.press(Direction::Left);
        assert_eq!(motion.active(), "up:1,left:2");
        motion.release(Direction::Up);
        assert_eq!(motion.active(), "left:2");
        motion.release(Direction::Left);
        assert_eq!(motion.active(), "none");
    }
}
