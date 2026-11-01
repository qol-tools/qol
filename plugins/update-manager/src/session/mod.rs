use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDateTime;

use crate::updates::{
    visible, Auto, AutoKey, Clock, Failure, Finish, Holding, Host, Hours, Mint, Refusal, Update,
    Updates,
};

mod journal;

pub(crate) use journal::Journal;

pub(crate) const MINT_DEFAULT: Hours = Hours {
    from: Clock::hm(0, 0),
    until: Clock::hm(1, 0),
    startup: true,
};
const DEFAULT_TIME: Clock = Clock::hm(1, 30);
const LATER_CHOICES: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Op {
    Hours,
    Auto(AutoKey),
    Tonight,
    RestartNow,
    Skips,
}

#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub(crate) updates: Vec<Update>,
    pub(crate) done: usize,
    pub(crate) started: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Failed {
    pub(crate) updates: Vec<Update>,
    pub(crate) failure: Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Helper {
    Off,
    Starting,
    Ready,
    Failed(Refusal),
}

#[derive(Debug, Clone)]
pub(crate) struct Rules {
    pub(crate) guard: bool,
    pub(crate) hold: bool,
    pub(crate) helper: Helper,
    pub(crate) holding: Option<Holding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InUse {
    pub(crate) label: String,
    pub(crate) bluetooth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Proposal {
    Hours(Option<Hours>),
    Tonight,
    Auto(AutoKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Act {
    Install(Vec<String>),
    Guard,
    Hold(bool),
    Retry,
    Tonight(bool),
    Time(Clock),
    Hours(Option<Hours>),
    Auto(AutoKey, bool),
    Ask(Proposal),
    RestartNow,
    Open(String),
    Skip(String),
    Never(String),
    Unskip(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Task {
    Install(Vec<Update>),
    Hours {
        hours: Option<Hours>,
        startup_delay: u32,
        restart: Option<NaiveDateTime>,
        was: Option<Hours>,
    },
    Auto {
        key: AutoKey,
        on: bool,
        was: bool,
    },
    Restart(Option<NaiveDateTime>),
    RestartNow,
    Rules,
    Open(String),
    Skips {
        add: Vec<String>,
        remove: Vec<String>,
        system: bool,
    },
}

#[derive(Debug)]
pub(crate) struct Session {
    pub(crate) updates: Option<Updates>,
    read: Vec<Update>,
    pub(crate) host: Host,
    pub(crate) checked: Option<NaiveDateTime>,
    pub(crate) job: Option<Job>,
    pub(crate) failed: Option<Failed>,
    pub(crate) rules: Rules,
    pub(crate) in_use: Option<InUse>,
    pub(crate) time: Clock,
    pub(crate) ask: Option<Proposal>,
    pub(crate) busy: BTreeSet<Op>,
    pub(crate) errors: BTreeMap<Op, String>,
    pub(crate) journal: Journal,
    pub(crate) borrowed: bool,
    pub(crate) install_log: Option<String>,
}

impl Session {
    pub(crate) fn new(journal: Journal, borrowed: bool) -> Self {
        Self {
            updates: None,
            read: Vec::new(),
            host: Host::default(),
            checked: None,
            job: None,
            failed: None,
            rules: Rules {
                guard: false,
                hold: false,
                helper: Helper::Off,
                holding: None,
            },
            in_use: None,
            time: DEFAULT_TIME,
            ask: None,
            busy: BTreeSet::new(),
            errors: BTreeMap::new(),
            journal,
            borrowed,
            install_log: None,
        }
    }

    pub(crate) fn list(&self) -> &[Update] {
        self.updates
            .as_ref()
            .map_or(&[], |updates| updates.list.as_slice())
    }

    pub(crate) fn available(&self) -> Vec<&Update> {
        self.list()
            .iter()
            .filter(|update| update.installable())
            .collect()
    }

    pub(crate) fn held(&self) -> Vec<&Update> {
        self.list()
            .iter()
            .filter(|update| !update.installable())
            .collect()
    }

    pub(crate) fn update(&self, id: &str) -> Option<&Update> {
        self.list().iter().find(|update| update.id == id)
    }

    pub(crate) fn finish(&self) -> Vec<Finish> {
        let mut finish = self.host.finish.clone();
        for row in &self.journal.finish {
            if !finish
                .iter()
                .any(|known| known.id == row.id || known.label == row.label)
            {
                finish.push(row.clone());
            }
        }
        finish
    }

    pub(crate) fn locked(&self) -> bool {
        !self.host.admin
    }

    pub(crate) fn mint(&self) -> Option<&Mint> {
        self.host.mint.as_ref()
    }

    pub(crate) fn hours(&self) -> Hours {
        self.mint()
            .and_then(|mint| mint.hours)
            .unwrap_or(MINT_DEFAULT)
    }

    pub(crate) fn auto(&self) -> Auto {
        self.mint().map(|mint| mint.auto).unwrap_or_default()
    }

    pub(crate) fn run_at(&self) -> Option<String> {
        let mint = self.mint()?;
        if !mint.auto.packages || self.holding_all() {
            return None;
        }
        let hours = self.hours();
        Some(match mint.next {
            Some(next) => format!("Mint's run at {}", Clock::of(next)),
            None => format!("Mint's run between {} and {}", hours.from, hours.until),
        })
    }

    pub(crate) fn choices(&self) -> Vec<Clock> {
        choices(self.hours())
    }

    pub(crate) fn moved(&self) -> bool {
        effective(self.hours(), self.time) != self.time
    }

    pub(crate) fn planned(&self) -> Clock {
        self.host
            .restart_at
            .map_or_else(|| effective(self.hours(), self.time), Clock::of)
    }

    pub(crate) fn holding_all(&self) -> bool {
        self.rules.hold && self.rules.helper == Helper::Ready
    }

    pub(crate) fn want_helper(&self) -> bool {
        (self.rules.guard || self.rules.hold) && !matches!(self.rules.helper, Helper::Failed(_))
    }

    pub(crate) fn want_hold(&self) -> bool {
        self.rules.hold || (self.rules.guard && self.in_use.is_some())
    }

    pub(crate) fn auto_changed(&self) -> Vec<(AutoKey, bool)> {
        let auto = self.auto();
        self.journal
            .auto
            .iter()
            .filter(|change| auto.get(change.key) != change.was)
            .map(|change| (change.key, change.was))
            .collect()
    }

    pub(crate) fn apply_updates(&mut self, updates: Updates, now: NaiveDateTime) {
        self.read = updates.list;
        self.updates = Some(Updates {
            list: visible(&self.read, &self.host.skips),
            problems: updates.problems,
        });
        self.checked = Some(now);
    }

    pub(crate) fn apply_host(&mut self, host: Host) {
        if self.host.restart_at.is_none() {
            if let Some(at) = host.restart_at {
                self.time = Clock::of(at);
            }
        }
        let skips_changed = host.skips != self.host.skips;
        self.host = host;
        if skips_changed {
            self.show_unskipped();
        }
    }

    fn show_unskipped(&mut self) {
        if let Some(updates) = self.updates.as_mut() {
            updates.list = visible(&self.read, &self.host.skips);
        }
    }

    pub(crate) fn skips(&self) -> &[String] {
        &self.host.skips
    }

    pub(crate) fn apply(&mut self, act: &Act, now: NaiveDateTime) -> Result<Option<Task>, String> {
        if !matches!(act, Act::Ask(_)) {
            self.ask = None;
        }
        match act {
            Act::Install(ids) => self.begin_install(ids).map(Some),
            Act::Guard => {
                self.rules.guard = !self.rules.guard;
                self.switched(self.rules.guard);
                Ok(Some(Task::Rules))
            }
            Act::Hold(on) => {
                self.rules.hold = *on;
                self.switched(*on);
                Ok(Some(Task::Rules))
            }
            Act::Retry => {
                self.switched(true);
                Ok(Some(Task::Rules))
            }
            Act::Tonight(on) => {
                if *on {
                    self.refuse_during_install()?;
                }
                let at = on.then(|| effective(self.hours(), self.time).next_after(now));
                self.start(Op::Tonight)?;
                Ok(Some(Task::Restart(at)))
            }
            Act::Time(clock) => {
                let was = std::mem::replace(&mut self.time, *clock);
                if self.host.restart_at.is_none() {
                    return Ok(None);
                }
                let moved = self.apply(&Act::Tonight(true), now);
                if moved.is_err() {
                    self.time = was;
                }
                moved
            }
            Act::Hours(hours) => {
                let target = hours.unwrap_or(MINT_DEFAULT);
                let restart = self.host.restart_at.and_then(|at| {
                    let moved = effective(target, self.time).next_after(now);
                    (moved != at).then_some(moved)
                });
                let mint = self
                    .mint()
                    .ok_or("Mint's Update Manager is not set up here")?;
                let (startup_delay, was) = (mint.startup_delay, mint.hours);
                self.start(Op::Hours)?;
                Ok(Some(Task::Hours {
                    hours: *hours,
                    startup_delay,
                    restart,
                    was,
                }))
            }
            Act::Auto(key, on) => {
                let was = self.auto().get(*key);
                self.start(Op::Auto(*key))?;
                Ok(Some(Task::Auto {
                    key: *key,
                    on: *on,
                    was,
                }))
            }
            Act::Ask(proposal) => {
                self.ask = Some(*proposal);
                Ok(None)
            }
            Act::RestartNow => {
                self.refuse_during_install()?;
                self.start(Op::RestartNow)?;
                Ok(Some(Task::RestartNow))
            }
            Act::Open(path) => Ok(Some(Task::Open(path.clone()))),
            Act::Skip(id) => {
                let update = self.update(id).ok_or("That update is gone")?;
                let add = update.skip_entries();
                self.change_skips(add, Vec::new())
            }
            Act::Never(id) => {
                let update = self.update(id).ok_or("That update is gone")?;
                let add = update.never_entries();
                let remove = self
                    .host
                    .skips
                    .iter()
                    .filter(|entry| {
                        entry
                            .split_once('=')
                            .is_some_and(|(source, _)| add.iter().any(|never| never == source))
                    })
                    .cloned()
                    .collect();
                self.change_skips(add, remove)
            }
            Act::Unskip(entry) => self.change_skips(Vec::new(), vec![entry.clone()]),
        }
    }

    fn change_skips(
        &mut self,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> Result<Option<Task>, String> {
        self.start(Op::Skips)?;
        Ok(Some(Task::Skips {
            add,
            remove,
            system: !self.locked(),
        }))
    }

    fn switched(&mut self, on: bool) {
        let failed = matches!(self.rules.helper, Helper::Failed(_));
        if failed && (on || !(self.rules.guard || self.rules.hold)) {
            self.rules.helper = Helper::Off;
        }
    }

    fn refuse_during_install(&self) -> Result<(), String> {
        if self.job.is_some() {
            return Err("Wait for the install that is running to finish first".into());
        }
        Ok(())
    }

    fn start(&mut self, op: Op) -> Result<(), String> {
        if !self.busy.insert(op) {
            return Err("That change is still being made".into());
        }
        self.errors.remove(&op);
        Ok(())
    }

    fn begin_install(&mut self, ids: &[String]) -> Result<Task, String> {
        if self.job.is_some() {
            return Err("An install is already running".into());
        }
        let updates: Vec<Update> = self
            .list()
            .iter()
            .filter(|update| update.installable() && ids.contains(&update.id))
            .cloned()
            .collect();
        if updates.is_empty() {
            return Err("Nothing here is left to install".into());
        }
        self.failed = None;
        self.job = Some(Job {
            updates: updates.clone(),
            done: 0,
            started: false,
        });
        Ok(Task::Install(updates))
    }

    pub(crate) fn progress(&mut self, done: usize) {
        if let Some(job) = self.job.as_mut() {
            job.done = done;
            job.started = true;
        }
    }

    pub(crate) fn installed(&mut self, result: Result<(), Failure>) {
        let Some(job) = self.job.take() else {
            return;
        };
        match result {
            Ok(()) => {
                let installed =
                    |update: &Update| job.updates.iter().any(|done| done.id == update.id);
                if let Some(updates) = self.updates.as_mut() {
                    updates.list.retain(|update| !installed(update));
                }
                self.read.retain(|update| !installed(update));
                self.journal.installed(&job.updates);
            }
            Err(failure) => {
                self.failed = Some(Failed {
                    updates: job.updates,
                    failure,
                })
            }
        }
    }

    pub(crate) fn done(&mut self, task: &Task, result: Result<(), Refusal>, now: NaiveDateTime) {
        let op = match task {
            Task::Hours { .. } => Op::Hours,
            Task::Auto { key, .. } => Op::Auto(*key),
            Task::Restart(_) => Op::Tonight,
            Task::RestartNow => Op::RestartNow,
            Task::Skips { .. } => Op::Skips,
            Task::Install(_) | Task::Rules | Task::Open(_) => return,
        };
        self.busy.remove(&op);
        if let Err(refusal) = result {
            self.errors.insert(op, refusal.to_string());
            return;
        }
        match task {
            Task::Hours { hours, was, .. } => {
                self.journal.hours_written(*was, hours.is_some(), now)
            }
            Task::Auto { key, on, was } => self.journal.auto_written(*key, *was, *on, now),
            Task::Restart(at) => self.journal.restart = at.map(|_| now),
            Task::Skips { add, remove, .. } => {
                self.host.skips.retain(|entry| !remove.contains(entry));
                for entry in add {
                    if !self.host.skips.contains(entry) {
                        self.host.skips.push(entry.clone());
                    }
                }
                self.show_unskipped();
            }
            _ => {}
        }
    }
}

pub(crate) fn position(updates: &[Update], done: usize) -> Option<(&Update, usize)> {
    let mut before = 0;
    for update in updates {
        if done < before + update.count() {
            return Some((update, done - before));
        }
        before += update.count();
    }
    updates.last().map(|update| (update, update.count()))
}

pub(crate) fn choices(hours: Hours) -> Vec<Clock> {
    let first = hours.until.plus(30);
    let hour = first.minutes() / 60;
    std::iter::once(first)
        .chain((1..=LATER_CHOICES).map(|step| Clock::new((hour + step) * 60)))
        .collect()
}

pub(crate) fn effective(hours: Hours, time: Clock) -> Clock {
    let first = choices(hours)[0];
    if time.night() < first.night() {
        first
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::{Package, Reason, Unit};

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("time")
    }

    fn update(id: &str, count: usize, held: bool) -> Update {
        Update {
            id: id.into(),
            tag: "Update Manager".into(),
            label: id.into(),
            name: id.into(),
            why: String::new(),
            restart: false,
            held: held.then_some(crate::updates::Held::Phasing(10)),
            unit: Unit::Packages,
            packages: (0..count)
                .map(|index| Package {
                    name: format!("{id}{index}"),
                    detail: String::new(),
                    bytes: None,
                    target: format!("{id}{index}=1"),
                    source: id.into(),
                    version: "1".into(),
                })
                .collect(),
            running: None,
            finish: String::new(),
            stops_sound: false,
            caution: None,
            unattended: true,
        }
    }

    fn session() -> Session {
        let mut session = Session::new(Journal::default(), false);
        session.apply_updates(
            Updates {
                list: vec![
                    update("a", 2, false),
                    update("b", 1, false),
                    update("c", 3, true),
                ],
                problems: Vec::new(),
            },
            at("2026-10-14 12:00"),
        );
        session.host.mint = Some(Mint {
            hours: Some(MINT_DEFAULT),
            startup_delay: 60,
            ours: false,
            next: Some(at("2026-10-15 00:37")),
            last: None,
            auto: Auto {
                packages: true,
                spices: true,
                flatpak: false,
            },
        });
        session
    }

    #[test]
    fn restart_choices_start_half_an_hour_after_the_hours() {
        let session = session();
        let choices: Vec<String> = session.choices().iter().map(ToString::to_string).collect();
        assert_eq!(
            choices,
            ["01:30", "02:00", "03:00", "04:00", "05:00", "06:00"]
        );
        assert_eq!(effective(MINT_DEFAULT, Clock::hm(0, 30)), Clock::hm(1, 30));
        assert_eq!(effective(MINT_DEFAULT, Clock::hm(3, 0)), Clock::hm(3, 0));
        assert_eq!(effective(MINT_DEFAULT, Clock::hm(22, 0)), Clock::hm(1, 30));
        assert_eq!(session.run_at().as_deref(), Some("Mint's run at 00:37"));
    }

    #[test]
    fn install_takes_what_can_install_and_one_job_at_a_time() {
        let mut session = session();
        let now = at("2026-10-14 12:00");
        let task = session
            .apply(&Act::Install(vec!["a".into(), "c".into()]), now)
            .expect("install");
        assert!(matches!(task, Some(Task::Install(ref updates)) if updates.len() == 1));
        assert!(session.apply(&Act::Install(vec!["b".into()]), now).is_err());
        session.progress(1);
        let job = session.job.as_ref().expect("job");
        assert_eq!(
            position(&job.updates, job.done).map(|(update, done)| (update.id.as_str(), done)),
            Some(("a", 1))
        );
        session.installed(Ok(()));
        assert!(session.job.is_none());
        assert!(session.update("a").is_none());
        session
            .apply(&Act::Install(vec!["b".into()]), now)
            .expect("install");
        session.installed(Err(Failure {
            done: 0,
            reason: Reason::Locked(None),
        }));
        assert!(session.failed.is_some());
        assert!(session.update("b").is_some());
    }

    #[test]
    fn changes_finish_with_their_journal_entries() {
        let mut session = session();
        let now = at("2026-10-14 12:00");
        let hours = Hours {
            from: Clock::hm(2, 0),
            until: Clock::hm(3, 0),
            startup: false,
        };
        let task = session
            .apply(&Act::Hours(Some(hours)), now)
            .expect("hours")
            .expect("task");
        assert!(session.apply(&Act::Hours(None), now).is_err());
        session.done(&task, Ok(()), now);
        assert!(session.busy.is_empty());
        assert_eq!(
            session.journal.hours.map(|change| change.was),
            Some(Some(MINT_DEFAULT))
        );

        let task = session
            .apply(&Act::Auto(AutoKey::Flatpak, true), now)
            .expect("auto")
            .expect("task");
        session.done(&task, Err(Refusal::Dismissed), now);
        assert_eq!(
            session
                .errors
                .get(&Op::Auto(AutoKey::Flatpak))
                .map(String::as_str),
            Some("the password prompt was closed")
        );
        assert!(session.journal.auto.is_empty());
    }

    #[test]
    fn a_set_restart_moves_with_the_hours() {
        let mut session = session();
        let now = at("2026-10-14 12:00");
        session.host.restart_at = Some(at("2026-10-15 01:30"));
        let later = Hours {
            from: Clock::hm(2, 0),
            until: Clock::hm(3, 0),
            startup: true,
        };
        let task = session.apply(&Act::Hours(Some(later)), now).expect("hours");
        assert!(
            matches!(task, Some(Task::Hours { restart: Some(moved), .. }) if moved == at("2026-10-15 03:30"))
        );
        assert!(!session.moved());
        if let Some(mint) = session.host.mint.as_mut() {
            mint.hours = Some(later);
        }
        assert!(session.moved());
    }

    #[test]
    fn the_guard_holds_only_while_something_plays() {
        let mut session = session();
        let now = at("2026-10-14 12:00");
        session.apply(&Act::Guard, now).expect("guard");
        assert!(session.want_helper());
        assert!(!session.want_hold());
        session.in_use = Some(InUse {
            label: "Luna 2".into(),
            bluetooth: true,
        });
        assert!(session.want_hold());
        session.rules.helper = Helper::Failed(Refusal::Dismissed);
        assert!(!session.want_helper());
        session.apply(&Act::Retry, now).expect("retry");
        assert!(session.rules.guard);
        assert_eq!(session.rules.helper, Helper::Off);
        assert!(session.want_helper());
    }

    #[test]
    fn a_rule_the_helper_failed_for_switches_off_without_asking_again() {
        let mut session = session();
        let now = at("2026-10-14 12:00");
        session.apply(&Act::Guard, now).expect("guard");
        session.apply(&Act::Hold(true), now).expect("hold");
        session.rules.helper = Helper::Failed(Refusal::Dismissed);
        assert_eq!(session.apply(&Act::Hold(false), now), Ok(Some(Task::Rules)));
        assert!(!session.rules.hold);
        assert!(session.rules.guard);
        assert!(matches!(session.rules.helper, Helper::Failed(_)));
        assert!(!session.want_helper());
        session.apply(&Act::Guard, now).expect("guard off");
        assert!(!session.rules.guard);
        assert_eq!(session.rules.helper, Helper::Off);
        assert!(!session.want_helper());
        session.rules.helper = Helper::Failed(Refusal::TimedOut);
        session.apply(&Act::Hold(true), now).expect("hold again");
        assert_eq!(session.rules.helper, Helper::Off);
        assert!(session.want_helper());
    }

    #[test]
    fn a_hold_the_helper_could_not_start_holds_nothing() {
        let mut session = session();
        session
            .apply(&Act::Hold(true), at("2026-10-14 12:00"))
            .expect("hold");
        session.rules.helper = Helper::Starting;
        assert!(!session.holding_all());
        assert_eq!(session.run_at().as_deref(), Some("Mint's run at 00:37"));
        session.rules.helper = Helper::Ready;
        assert!(session.holding_all());
        assert_eq!(session.run_at(), None);
        session.rules.helper = Helper::Failed(Refusal::Dismissed);
        assert!(!session.holding_all());
        assert_eq!(session.run_at().as_deref(), Some("Mint's run at 00:37"));
    }

    #[test]
    fn a_restart_time_the_restart_cannot_take_stays_as_it_was() {
        let mut session = session();
        session.host.restart_at = Some(at("2026-10-15 01:30"));
        session.job = Some(Job {
            updates: Vec::new(),
            done: 0,
            started: false,
        });
        let was = session.time;
        assert_ne!(was, Clock::hm(5, 0));
        assert_eq!(
            session.apply(&Act::Time(Clock::hm(5, 0)), at("2026-10-14 12:00")),
            Err("Wait for the install that is running to finish first".into())
        );
        assert_eq!(session.time, was);
    }
}
