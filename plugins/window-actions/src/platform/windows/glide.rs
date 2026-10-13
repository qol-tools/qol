use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qol_windowing::platform::windows::Window;
use qol_windowing::{WindowId, WindowRect};

use crate::glide::{Direction, Phase};

const WATCHDOG: Duration = Duration::from_millis(1000);
const TICK: Duration = Duration::from_millis(16);
const WORKER_STOP_WAIT: Duration = Duration::from_millis(500);
const JOIN_POLL: Duration = Duration::from_millis(5);
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
        let frame = keep_on_screen(
            frame,
            &qol_windowing::platform::windows::work_areas_left_to_right(),
        );
        Window::from_id(id).is_some_and(|window| window.set_frame_nonblocking(frame).is_ok())
    }
}

fn keep_on_screen(frame: WindowRect, work_areas: &[WindowRect]) -> WindowRect {
    let Some((left, top, right, bottom)) = work_areas
        .iter()
        .map(|area| (area.x, area.y, area.x + area.width, area.y + area.height))
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
    else {
        return frame;
    };
    WindowRect {
        x: frame.x.min(right - frame.width).max(left),
        y: frame.y.min(bottom - frame.height).max(top),
        ..frame
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
    generation: u64,
    x: f64,
    y: f64,
    goal: Option<(f64, f64)>,
    last_tick: Instant,
}

struct Step {
    id: WindowId,
    generation: u64,
    x: f64,
    y: f64,
    goal: Option<(f64, f64)>,
    dx: f64,
    dy: f64,
}

struct Placed {
    x: f64,
    y: f64,
    goal: (f64, f64),
}

enum Plan {
    Stop,
    Hold,
    Move(Step),
}

impl Step {
    fn apply(&self, mover: &impl Mover) -> Option<Placed> {
        let frame = mover.frame(&self.id)?;
        let (x, y) = match self.goal {
            Some((goal_x, goal_y)) => (
                if frame.x == goal_x { self.x } else { frame.x },
                if frame.y == goal_y { self.y } else { frame.y },
            ),
            None => (self.x, self.y),
        };
        let next_x = x + self.dx;
        let next_y = y + self.dy;
        let goal = WindowRect {
            x: next_x.round(),
            y: next_y.round(),
            ..frame
        };
        mover.place(&self.id, goal).then_some(Placed {
            x: next_x,
            y: next_y,
            goal: (goal.x, goal.y),
        })
    }
}

struct State {
    motion: Motion,
    target: Option<Target>,
    speed: f64,
    expires_at: Instant,
    generation: u64,
}

impl State {
    fn new() -> Self {
        Self {
            motion: Motion::default(),
            target: None,
            speed: MIN_SPEED,
            expires_at: Instant::now(),
            generation: 0,
        }
    }

    fn plan(&mut self, now: Instant, elapsed: f64) -> Plan {
        if self.motion.is_empty() || now > self.expires_at {
            return Plan::Stop;
        }
        let (dx, dy) = self.motion.vector();
        let speed = self.speed;
        let Some(target) = self.target.as_mut() else {
            return Plan::Stop;
        };
        target.last_tick = now;
        if dx == 0.0 && dy == 0.0 {
            return Plan::Hold;
        }
        Plan::Move(Step {
            id: target.id.clone(),
            generation: target.generation,
            x: target.x,
            y: target.y,
            goal: target.goal,
            dx: dx * speed * elapsed,
            dy: dy * speed * elapsed,
        })
    }

    fn settle(&mut self, generation: u64, placed: Option<Placed>) -> bool {
        if !self.is_current(generation) {
            return true;
        }
        let Some(placed) = placed else {
            self.reset();
            return false;
        };
        if let Some(target) = self.target.as_mut() {
            target.x = placed.x;
            target.y = placed.y;
            target.goal = Some(placed.goal);
        }
        true
    }

    fn is_current(&self, generation: u64) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| target.generation == generation)
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

fn advance(state: &Mutex<State>, plan: Plan, mover: &impl Mover) -> bool {
    match plan {
        Plan::Stop => false,
        Plan::Hold => true,
        Plan::Move(step) => {
            let placed = step.apply(mover);
            lock(state).settle(step.generation, placed)
        }
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
        let plan = {
            let mut state = self.lock();
            let fresh = state.target.is_none();
            if fresh {
                let window = Window::foreground().ok_or("No focused window")?;
                if !window.is_switchable() {
                    return Err("Focused surface is not an app window".into());
                }
                let frame = window.frame().ok_or("Cannot read window geometry")?;
                state.generation += 1;
                state.target = Some(Target {
                    id: window.id(),
                    generation: state.generation,
                    x: frame.x,
                    y: frame.y,
                    goal: None,
                    last_tick: Instant::now(),
                });
            }
            let now = Instant::now();
            state.motion.press(direction);
            state.speed = speed.clamp(MIN_SPEED, MAX_SPEED);
            state.expires_at = now + WATCHDOG;
            fresh.then(|| state.plan(now, FIRST_STEP_SECONDS))
        };
        if let Some(plan) = plan {
            if !advance(&self.state, plan, &Win32Mover) {
                self.lock().reset();
                return Err("The focused window can no longer be moved".into());
            }
        }
        let observation = self.lock().observation();
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
        let Some(generation) = self.lock().target.as_ref().map(|target| target.generation) else {
            return;
        };
        let state = Arc::clone(&self.state);
        self.worker = std::thread::Builder::new()
            .name("window-actions-glide".into())
            .spawn(move || run(&state, generation))
            .ok();
    }

    fn join_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            join_within(worker, WORKER_STOP_WAIT);
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

fn join_within(worker: JoinHandle<()>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(JOIN_POLL);
    }
    let _ = worker.join();
    true
}

fn run(state: &Mutex<State>, generation: u64) {
    loop {
        std::thread::sleep(TICK);
        let plan = {
            let mut state = lock(state);
            if !state.is_current(generation) {
                return;
            }
            let now = Instant::now();
            let elapsed = state.target.as_ref().map_or(0.0, |target| {
                now.duration_since(target.last_tick)
                    .as_secs_f64()
                    .min(MAX_STEP_SECONDS)
            });
            let plan = state.plan(now, elapsed);
            if matches!(plan, Plan::Stop) {
                state.reset();
                return;
            }
            plan
        };
        if !advance(state, plan, &Win32Mover) {
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
        pending: RefCell<Option<WindowRect>>,
        lagging: bool,
    }

    impl FakeMover {
        fn land(&self) {
            if let Some(frame) = self.pending.borrow_mut().take() {
                *self.frame.borrow_mut() = frame;
            }
        }
    }

    impl Mover for FakeMover {
        fn frame(&self, _id: &WindowId) -> Option<WindowRect> {
            Some(*self.frame.borrow())
        }

        fn place(&self, _id: &WindowId, frame: WindowRect) -> bool {
            let x = self.clamp_x.map_or(frame.x, |limit| frame.x.min(limit));
            let placed = WindowRect { x, ..frame };
            if self.lagging {
                *self.pending.borrow_mut() = Some(placed);
            } else {
                *self.frame.borrow_mut() = placed;
            }
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
            pending: RefCell::new(None),
            lagging: false,
        }
    }

    fn step(state: &mut State, now: Instant, elapsed: f64, mover: &FakeMover) -> bool {
        match state.plan(now, elapsed) {
            Plan::Stop => false,
            Plan::Hold => true,
            Plan::Move(step) => {
                let placed = step.apply(mover);
                state.settle(step.generation, placed)
            }
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
        state.generation = 1;
        state.target = Some(Target {
            id: WindowId::from_u32(1),
            generation: 1,
            x: 100.0,
            y: 200.0,
            goal: None,
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
        assert!(step(&mut state, Instant::now(), 0.05, &fake));
        let frame = *fake.frame.borrow();
        assert_eq!((frame.x, frame.y), (160.0, 200.0));
        assert_eq!((frame.width, frame.height), (800.0, 600.0));
    }

    #[test]
    fn a_constrained_move_resynchronizes_to_the_real_position() {
        let fake = mover(Some(120.0));
        let mut state = gliding(&[Direction::Right], 1200.0);
        assert!(step(&mut state, Instant::now(), 0.05, &fake));
        assert!(step(&mut state, Instant::now(), 0.05, &fake));
        assert_eq!(state.target.as_ref().map(|target| target.x), Some(180.0));
        assert_eq!(fake.frame.borrow().x, 120.0);
    }

    #[test]
    fn a_placement_that_lands_after_the_readback_keeps_full_speed() {
        let fake = FakeMover {
            lagging: true,
            ..mover(None)
        };
        let mut state = gliding(&[Direction::Right], 1200.0);
        for _ in 0..3 {
            assert!(step(&mut state, Instant::now(), 0.05, &fake));
            fake.land();
        }
        assert_eq!(fake.frame.borrow().x, 280.0);
        assert_eq!(state.target.as_ref().map(|target| target.x), Some(280.0));
    }

    #[test]
    fn a_failed_placement_ends_the_glide() {
        struct Gone;
        impl Mover for Gone {
            fn frame(&self, _id: &WindowId) -> Option<WindowRect> {
                None
            }

            fn place(&self, _id: &WindowId, _frame: WindowRect) -> bool {
                false
            }
        }
        let mut state = gliding(&[Direction::Left], 1200.0);
        let Plan::Move(step) = state.plan(Instant::now(), 0.016) else {
            panic!("a held direction plans a move");
        };
        assert!(!state.settle(step.generation, step.apply(&Gone)));
        assert!(state.target.is_none());
        assert!(state.motion.is_empty());
    }

    #[test]
    fn a_move_that_returns_after_its_glide_ended_changes_nothing() {
        let fake = mover(None);
        let mut state = gliding(&[Direction::Right], 1200.0);
        let Plan::Move(stale) = state.plan(Instant::now(), 0.05) else {
            panic!("a held direction plans a move");
        };
        state.reset();
        state.generation = 2;
        state.motion.press(Direction::Down);
        state.target = Some(Target {
            id: WindowId::from_u32(1),
            generation: 2,
            x: 500.0,
            y: 500.0,
            goal: None,
            last_tick: Instant::now(),
        });
        assert!(state.settle(stale.generation, stale.apply(&fake)));
        assert_eq!(
            state.target.as_ref().map(|target| (target.x, target.y)),
            Some((500.0, 500.0))
        );
        assert!(!state.is_current(stale.generation));
    }

    #[test]
    fn the_lock_is_free_while_a_move_is_applied() {
        struct Probe<'a>(&'a Mutex<State>);
        impl Mover for Probe<'_> {
            fn frame(&self, _id: &WindowId) -> Option<WindowRect> {
                Some(WindowRect {
                    x: 100.0,
                    y: 200.0,
                    width: 800.0,
                    height: 600.0,
                })
            }

            fn place(&self, _id: &WindowId, _frame: WindowRect) -> bool {
                self.0.try_lock().is_ok()
            }
        }
        let state = Mutex::new(gliding(&[Direction::Right], 1200.0));
        let plan = lock(&state).plan(Instant::now(), 0.016);
        assert!(advance(&state, plan, &Probe(&state)));
    }

    #[test]
    fn stopping_waits_a_bounded_time_for_a_stuck_worker() {
        let finished = std::thread::spawn(|| {});
        assert!(join_within(finished, Duration::from_secs(5)));

        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let stuck = std::thread::spawn(move || {
            let _ = blocked.recv();
        });
        let started = Instant::now();
        assert!(!join_within(stuck, Duration::from_millis(50)));
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(release);
    }

    #[test]
    fn placement_stops_at_the_edges_of_the_work_areas() {
        let rect = |x: f64, y: f64, width: f64, height: f64| WindowRect {
            x,
            y,
            width,
            height,
        };
        let single = [rect(0.0, 0.0, 1280.0, 752.0)];
        let pair = [
            rect(0.0, 0.0, 1280.0, 752.0),
            rect(1280.0, 0.0, 1920.0, 1040.0),
        ];
        let cases: [(&[WindowRect], WindowRect, (f64, f64)); 6] = [
            (&single, rect(100.0, 50.0, 400.0, 300.0), (100.0, 50.0)),
            (&single, rect(1100.0, 600.0, 400.0, 300.0), (880.0, 452.0)),
            (&single, rect(-50.0, -20.0, 400.0, 300.0), (0.0, 0.0)),
            (&single, rect(30.0, 30.0, 1600.0, 900.0), (0.0, 0.0)),
            (&pair, rect(1500.0, 600.0, 400.0, 300.0), (1500.0, 600.0)),
            (&[], rect(5000.0, 5000.0, 400.0, 300.0), (5000.0, 5000.0)),
        ];
        for (areas, frame, expected) in cases {
            let placed = keep_on_screen(frame, areas);
            assert_eq!((placed.x, placed.y), expected, "{frame:?} in {areas:?}");
            assert_eq!((placed.width, placed.height), (frame.width, frame.height));
        }
    }

    #[test]
    fn released_or_expired_glides_stop_stepping() {
        let fake = mover(None);
        let mut released = gliding(&[Direction::Left], 1200.0);
        released.motion.release(Direction::Left);
        assert!(!step(&mut released, Instant::now(), 0.016, &fake));

        let mut expired = gliding(&[Direction::Left], 1200.0);
        let later = Instant::now() + WATCHDOG + Duration::from_millis(1);
        assert!(!step(&mut expired, later, 0.016, &fake));
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
