use std::cell::RefCell;
use std::fmt;
use std::ops::Deref;
use std::panic::Location;
use std::sync::{Arc, Once};
use std::thread;
use std::time::Duration;

use qol_sync::{Condvar, Mutex, MutexGuard, RwLock, WouldBlock};

type Probe = Box<dyn Fn() -> String>;

thread_local! {
    static RECORDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

struct Recorder;

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let line = format!("{} {}", record.level(), record.args());
        RECORDS.with(|records| records.borrow_mut().push(line));
        let probed = PROBE.with(|probe| probe.borrow().as_ref().map(|run| run()));
        if let Some(probed) = probed {
            RECORDS.with(|records| records.borrow_mut().push(probed));
        }
    }

    fn flush(&self) {}
}

static RECORDER: Recorder = Recorder;

fn record_this_thread() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&RECORDER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    RECORDS.with(|records| records.borrow_mut().clear());
    PROBE.with(|probe| *probe.borrow_mut() = None);
}

fn records() -> Vec<String> {
    RECORDS.with(|records| records.borrow().clone())
}

fn probe_while_logging(probe: impl Fn() -> String + 'static) {
    PROBE.with(|slot| *slot.borrow_mut() = Some(Box::new(probe)));
}

fn what_the_logger_finds<G: Deref>(taken: Result<G, WouldBlock>) -> String
where
    G::Target: fmt::Display,
{
    match taken {
        Ok(guard) => format!("the logger took the lock holding {}", *guard),
        Err(WouldBlock) => String::from("the logger found the lock held"),
    }
}

fn join_panicked(thread: thread::JoinHandle<()>) {
    assert!(thread.join().is_err());
}

#[track_caller]
fn lock_here<T>(mutex: &Mutex<T>) -> (MutexGuard<'_, T>, &'static Location<'static>) {
    (mutex.lock(), Location::caller())
}

#[test]
fn a_poisoned_mutex_hands_back_its_value_and_logs_once_where_it_was_locked() {
    record_this_thread();
    let mutex = Arc::new(Mutex::new(vec![7]));
    let held = Arc::clone(&mutex);
    join_panicked(thread::spawn(move || {
        let mut guard = held.lock();
        guard.push(8);
        panic!("poison the lock");
    }));

    let (mut guard, at) = lock_here(&mutex);
    assert_eq!(*guard, [7, 8]);
    guard.push(9);
    drop(guard);

    assert_eq!(*mutex.lock(), [7, 8, 9]);
    assert_eq!(
        records(),
        [format!("ERROR recovered a poisoned lock at {at}")]
    );
}

#[test]
fn a_poisoned_rwlock_hands_back_its_value_to_readers_and_writers() {
    record_this_thread();
    let lock = Arc::new(RwLock::new(String::from("kept")));
    let held = Arc::clone(&lock);
    join_panicked(thread::spawn(move || {
        let _guard = held.write();
        panic!("poison the lock");
    }));

    assert_eq!(*lock.read(), "kept");
    lock.write().push('!');
    assert_eq!(*lock.read(), "kept!");
    assert_eq!(records().len(), 1);
}

#[test]
fn a_waiter_wakes_with_the_value_when_its_notifier_panics_holding_the_lock() {
    record_this_thread();
    let shared = Arc::new((Mutex::new(0), Condvar::new()));
    let (count, wake) = &*shared;
    let waiting = count.lock();
    let notifier = Arc::clone(&shared);
    let poisoner = thread::spawn(move || {
        let (count, wake) = &*notifier;
        let mut guard = count.lock();
        *guard += 1;
        wake.notify_all();
        panic!("poison the lock");
    });

    let woken = wake.wait_while(waiting, |count| *count == 0);
    assert_eq!(*woken, 1);
    drop(woken);
    join_panicked(poisoner);

    assert_eq!(*count.lock(), 1);
    assert_eq!(records().len(), 1);
}

#[test]
fn a_timed_waiter_wakes_before_its_deadline_when_its_notifier_panics() {
    record_this_thread();
    let shared = Arc::new((Mutex::new(false), Condvar::new()));
    let (ready, wake) = &*shared;
    let waiting = ready.lock();
    let notifier = Arc::clone(&shared);
    let poisoner = thread::spawn(move || {
        let (ready, wake) = &*notifier;
        let mut guard = ready.lock();
        *guard = true;
        wake.notify_all();
        panic!("poison the lock");
    });

    let (woken, waited) =
        wake.wait_timeout_while(waiting, Duration::from_secs(30), |ready| !*ready);
    assert!(*woken);
    assert!(!waited.timed_out());
    drop(woken);
    join_panicked(poisoner);

    assert!(*ready.lock());
    assert_eq!(records().len(), 1);
}

#[test]
fn try_lock_is_refused_while_held_and_recovers_once_poisoned() {
    record_this_thread();
    let mutex = Arc::new(Mutex::new(1));
    let held = mutex.lock();
    assert_eq!(mutex.try_lock().map(|guard| *guard), Err(WouldBlock));
    drop(held);

    let poisoned = Arc::clone(&mutex);
    join_panicked(thread::spawn(move || {
        let _guard = poisoned.lock();
        panic!("poison the lock");
    }));
    assert_eq!(mutex.try_lock().map(|guard| *guard), Ok(1));
    assert_eq!(mutex.try_lock().map(|guard| *guard), Ok(1));
    assert_eq!(records().len(), 1);
}

#[test]
fn a_poisoned_mutex_gives_up_its_value_when_taken_apart() {
    record_this_thread();
    let mutex = Arc::new(Mutex::new(String::from("whole")));
    let held = Arc::clone(&mutex);
    join_panicked(thread::spawn(move || {
        let _guard = held.lock();
        panic!("poison the lock");
    }));

    let mut mutex = Arc::into_inner(mutex).unwrap();
    mutex.get_mut().push('!');
    assert_eq!(mutex.into_inner(), "whole!");
    assert_eq!(records().len(), 1);
}

#[test]
fn a_logger_can_take_the_poisoned_mutex_while_its_recovery_reports() {
    record_this_thread();
    let mutex = Arc::new(Mutex::new(5));
    let held = Arc::clone(&mutex);
    join_panicked(thread::spawn(move || {
        let _guard = held.lock();
        panic!("poison the lock");
    }));
    let logger_side = Arc::clone(&mutex);
    probe_while_logging(move || what_the_logger_finds(logger_side.try_lock()));

    let (guard, at) = lock_here(&mutex);
    assert_eq!(*guard, 5);
    assert_eq!(
        records(),
        [
            format!("ERROR recovered a poisoned lock at {at}"),
            String::from("the logger took the lock holding 5"),
        ]
    );
}

#[test]
fn a_logger_can_take_the_poisoned_rwlock_while_its_recovery_reports() {
    record_this_thread();
    let lock = Arc::new(RwLock::new(String::from("kept")));
    let held = Arc::clone(&lock);
    join_panicked(thread::spawn(move || {
        let _guard = held.write();
        panic!("poison the lock");
    }));
    let logger_side = Arc::clone(&lock);
    probe_while_logging(move || what_the_logger_finds(logger_side.try_write()));

    assert_eq!(*lock.read(), "kept");
    assert_eq!(records()[1], "the logger took the lock holding kept");
}

#[test]
fn a_logger_can_take_the_mutex_a_woken_waiter_recovers_while_it_reports() {
    record_this_thread();
    let shared = Arc::new((Mutex::new(0), Condvar::new()));
    let (count, wake) = &*shared;
    let logger_side = Arc::clone(&shared);
    probe_while_logging(move || what_the_logger_finds(logger_side.0.try_lock()));
    let waiting = count.lock();
    let notifier = Arc::clone(&shared);
    let poisoner = thread::spawn(move || {
        let (count, wake) = &*notifier;
        let mut guard = count.lock();
        *guard += 1;
        wake.notify_all();
        panic!("poison the lock");
    });

    let woken = wake.wait_while(waiting, |count| *count == 0);
    assert_eq!(*woken, 1);
    drop(woken);
    join_panicked(poisoner);
    assert_eq!(records()[1], "the logger took the lock holding 1");
}
