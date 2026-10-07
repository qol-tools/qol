use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};

use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    InOrder,
    Alone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<T> {
    pub task: T,
    pub state: State,
    pub progress: Option<u8>,
    pub reason: Option<String>,
    pub order: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

pub struct WorkQueue<T> {
    inner: Mutex<Inner<T>>,
    wake: Notify,
}

struct Inner<T> {
    slots: HashMap<String, Slot<T>>,
    next_order: u64,
}

struct Slot<T> {
    entry: Entry<T>,
    run: Run,
}

impl<T: Clone + Send + 'static> Default for WorkQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + 'static> WorkQueue<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slots: HashMap::new(),
                next_order: 0,
            }),
            wake: Notify::new(),
        }
    }

    pub fn push(&self, key: &str, task: T, run: Run) -> Result<(), Busy> {
        {
            let mut inner = self.lock();
            if inner.refuses(key) {
                return Err(Busy);
            }
            let order = inner.next_order;
            inner.next_order += 1;
            let entry = Entry {
                task,
                state: State::Queued,
                progress: None,
                reason: None,
                order,
            };
            inner.slots.insert(key.to_string(), Slot { entry, run });
        }
        self.wake.notify_one();
        Ok(())
    }

    pub fn cancel_queued(&self, matches: impl Fn(&str, &T) -> bool) -> usize {
        let mut inner = self.lock();
        let before = inner.slots.len();
        inner.slots.retain(|key, slot| {
            slot.entry.state != State::Queued || !matches(key, &slot.entry.task)
        });
        before - inner.slots.len()
    }

    pub fn set_progress(&self, key: &str, percent: u8) {
        if let Some(slot) = self.lock().slots.get_mut(key) {
            slot.entry.progress = Some(percent);
        }
    }

    pub fn get(&self, key: &str) -> Option<Entry<T>> {
        self.lock().slots.get(key).map(|slot| slot.entry.clone())
    }

    pub fn snapshot(&self) -> HashMap<String, Entry<T>> {
        self.lock()
            .slots
            .iter()
            .map(|(key, slot)| (key.clone(), slot.entry.clone()))
            .collect()
    }

    pub fn busy(&self) -> bool {
        self.lock().slots.values().any(Slot::active)
    }

    pub async fn run<F, Fut>(&self, mut work: F)
    where
        F: FnMut(String, T) -> Fut,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        loop {
            let Some((key, task)) = self.start_next() else {
                self.wake.notified().await;
                continue;
            };
            let outcome = tokio::spawn(work(key.clone(), task))
                .await
                .unwrap_or_else(|error| Err(format!("the task stopped: {error}")));
            self.settle(&key, outcome);
        }
    }

    fn start_next(&self) -> Option<(String, T)> {
        let mut inner = self.lock();
        let key = inner.next()?;
        let slot = inner.slots.get_mut(&key)?;
        slot.entry.state = State::Running;
        Some((key, slot.entry.task.clone()))
    }

    fn settle(&self, key: &str, outcome: Result<(), String>) {
        let mut inner = self.lock();
        match outcome {
            Ok(()) => {
                inner.slots.remove(key);
            }
            Err(reason) => {
                if let Some(slot) = inner.slots.get_mut(key) {
                    slot.entry.state = State::Failed;
                    slot.entry.progress = None;
                    slot.entry.reason = Some(reason);
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<T> Slot<T> {
    fn active(&self) -> bool {
        self.entry.state != State::Failed
    }
}

impl<T> Inner<T> {
    fn refuses(&self, key: &str) -> bool {
        self.slots.get(key).is_some_and(Slot::active)
            || self
                .slots
                .values()
                .any(|slot| slot.run == Run::Alone && slot.entry.state == State::Running)
    }

    fn next(&self) -> Option<String> {
        if self
            .slots
            .values()
            .any(|slot| slot.entry.state == State::Running)
        {
            return None;
        }
        let oldest = |run: Run| {
            self.slots
                .iter()
                .filter(|(_, slot)| slot.run == run && slot.entry.state == State::Queued)
                .min_by_key(|(_, slot)| slot.entry.order)
                .map(|(key, _)| key.clone())
        };
        oldest(Run::InOrder).or_else(|| oldest(Run::Alone))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn states(queue: &WorkQueue<&'static str>) -> Vec<(String, State)> {
        let mut entries: Vec<_> = queue.snapshot().into_iter().collect();
        entries.sort_by_key(|(_, entry)| entry.order);
        entries
            .into_iter()
            .map(|(key, entry)| (key, entry.state))
            .collect()
    }

    #[test]
    fn a_key_that_waits_or_runs_is_refused_and_a_failed_one_is_retried() {
        let queue = WorkQueue::new();
        queue.push("a", "install", Run::InOrder).unwrap();
        assert_eq!(queue.push("a", "update", Run::InOrder), Err(Busy));

        assert_eq!(queue.start_next(), Some(("a".to_string(), "install")));
        assert_eq!(queue.push("a", "update", Run::InOrder), Err(Busy));
        assert!(queue.busy());

        queue.settle("a", Err("offline".to_string()));
        let failed = queue.get("a").unwrap();
        assert_eq!(
            (failed.state, failed.reason.as_deref()),
            (State::Failed, Some("offline"))
        );
        assert!(!queue.busy());

        queue.push("a", "update", Run::InOrder).unwrap();
        assert_eq!(queue.get("a").unwrap().state, State::Queued);
        assert_eq!(queue.start_next(), Some(("a".to_string(), "update")));
        queue.settle("a", Ok(()));
        assert_eq!(queue.get("a"), None);
    }

    #[test]
    fn tasks_run_one_at_a_time_oldest_first_and_alone_ones_last() {
        let queue = WorkQueue::new();
        queue.push("host", "restart", Run::Alone).unwrap();
        queue.push("zed", "update", Run::InOrder).unwrap();
        queue.push("alpha", "update", Run::InOrder).unwrap();

        let mut ran = Vec::new();
        while let Some((key, _)) = queue.start_next() {
            assert_eq!(queue.start_next(), None, "{key} must run alone");
            ran.push(key.clone());
            if key == "zed" {
                queue.push("late", "install", Run::InOrder).unwrap();
            }
            queue.settle(&key, Ok(()));
        }
        assert_eq!(ran, ["zed", "alpha", "late", "host"]);
    }

    #[test]
    fn nothing_is_accepted_while_an_alone_task_runs() {
        let queue = WorkQueue::new();
        queue.push("host", "restart", Run::Alone).unwrap();
        queue.push("a", "update", Run::InOrder).unwrap();
        assert_eq!(queue.start_next(), Some(("a".to_string(), "update")));
        queue.settle("a", Ok(()));

        assert_eq!(queue.start_next(), Some(("host".to_string(), "restart")));
        assert_eq!(queue.push("b", "update", Run::InOrder), Err(Busy));

        queue.settle("host", Err("download failed".to_string()));
        assert_eq!(queue.push("b", "update", Run::InOrder), Ok(()));
    }

    #[test]
    fn only_waiting_tasks_are_cancelled() {
        let queue = WorkQueue::new();
        for key in ["a", "b", "c", "d"] {
            queue.push(key, "update", Run::InOrder).unwrap();
        }
        queue.push("e", "remove", Run::InOrder).unwrap();
        assert_eq!(queue.start_next(), Some(("a".to_string(), "update")));
        assert_eq!(queue.cancel_queued(|key, _| key == "b"), 1);
        assert_eq!(queue.cancel_queued(|key, _| key == "a"), 0);
        assert_eq!(queue.cancel_queued(|key, _| key == "missing"), 0);
        assert_eq!(queue.cancel_queued(|_, task| *task == "update"), 2);
        assert_eq!(
            states(&queue),
            [
                ("a".to_string(), State::Running),
                ("e".to_string(), State::Queued)
            ]
        );
    }

    #[test]
    fn progress_belongs_to_the_task_and_clears_on_failure() {
        let queue = WorkQueue::new();
        queue.push("host", "restart", Run::Alone).unwrap();
        queue.start_next();
        queue.set_progress("host", 40);
        assert_eq!(queue.get("host").unwrap().progress, Some(40));
        queue.settle("host", Err("verify failed".to_string()));
        assert_eq!(queue.get("host").unwrap().progress, None);
    }

    #[tokio::test]
    async fn the_worker_wakes_for_work_pushed_after_it_went_idle() {
        let queue = Arc::new(WorkQueue::new());
        let (sender, mut ran) = tokio::sync::mpsc::unbounded_channel();
        let worker = Arc::clone(&queue);
        tokio::spawn(async move {
            worker
                .run(|key, task: &'static str| {
                    let sender = sender.clone();
                    async move {
                        sender.send(key).unwrap();
                        match task {
                            "fail" => Err("no".to_string()),
                            "panic" => panic!("the task broke"),
                            _ => Ok(()),
                        }
                    }
                })
                .await
        });
        tokio::task::yield_now().await;

        queue.push("a", "fail", Run::InOrder).unwrap();
        queue.push("p", "panic", Run::InOrder).unwrap();
        queue.push("b", "update", Run::InOrder).unwrap();
        for expected in ["a", "p", "b"] {
            let key = tokio::time::timeout(Duration::from_secs(5), ran.recv())
                .await
                .unwrap();
            assert_eq!(key.as_deref(), Some(expected));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while queue.get("b").is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            states(&queue),
            [
                ("a".to_string(), State::Failed),
                ("p".to_string(), State::Failed)
            ]
        );
        assert!(queue.get("p").unwrap().reason.unwrap().contains("panic"));
    }
}
