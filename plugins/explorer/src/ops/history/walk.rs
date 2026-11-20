use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::TempDir;

use super::{
    start, Action, Change, Choices, Direction, History, Outcome, Plan, Reply, Settled, State, Step,
    StepId, Top, Undone,
};
use crate::ops::print::Print;
use crate::ops::{self, Clash, Done, NameProblem, Progress, Resolve, Tools, Transfer, Verb};

const WAIT: Duration = Duration::from_secs(30);
const STAMP_BASE: u64 = 1_000_000_000;
const STEPS: usize = 25;
const UNWIND_CAP: usize = 60;
const RESOLVES: [Resolve; 4] = [
    Resolve::Keep,
    Resolve::Replace,
    Resolve::Merge,
    Resolve::KeepBoth,
];
const MIX: [(Pick, usize); 16] = [
    (Pick::Copy, 9),
    (Pick::Move, 9),
    (Pick::Duplicate, 5),
    (Pick::Rename, 7),
    (Pick::CaseRename, 3),
    (Pick::NewFile, 4),
    (Pick::NewFolder, 4),
    (Pick::Trash, 8),
    (Pick::Undo, 26),
    (Pick::Rewrite, 5),
    (Pick::Add, 3),
    (Pick::Remove, 4),
    (Pick::OutsideRename, 3),
    (Pick::EmptyBin, 3),
    (Pick::Redo, 12),
    (Pick::RoundTrip, 7),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node {
    Folder,
    File(Vec<u8>),
    Other,
}

pub(super) struct Transferred {
    pub(super) step: StepId,
    pub(super) seen: Vec<Progress>,
}

impl Transferred {
    pub(super) fn done(&self) -> Option<&Done> {
        self.seen.iter().find_map(|progress| match progress {
            Progress::Finished(done) => Some(done),
            Progress::Bytes { .. }
            | Progress::Arriving(_)
            | Progress::Placed(_)
            | Progress::Amended { .. }
            | Progress::Retracted(_) => None,
        })
    }
}

pub(super) enum Undid {
    Nothing,
    Dropped(StepId),
    Ran {
        step: StepId,
        planned: Vec<usize>,
        outcomes: Vec<(usize, Outcome)>,
        asked: Vec<usize>,
    },
}

pub(super) struct Rig {
    _dir: TempDir,
    pub(super) work: PathBuf,
    pub(super) bin_root: PathBuf,
    pub(super) tools: Tools,
    pub(super) history: History,
}

impl Rig {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let tools_dir = dir.path().join("tools");
        let tools = Tools::at(&tools_dir);
        Self {
            _dir: dir,
            work,
            bin_root: tools_dir.join("bin"),
            tools,
            history: History::default(),
        }
    }

    pub(super) fn held(&self) -> BTreeSet<Vec<u8>> {
        contents(&[self.work.as_path(), self.bin_root.as_path()])
    }

    pub(super) fn keeps(&self, before: &BTreeSet<Vec<u8>>, what: &str) {
        let after = self.held();
        let lost: Vec<String> = before
            .difference(&after)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .collect();
        assert!(lost.is_empty(), "lost {lost:?} after {what}");
    }

    pub(super) fn transfer(
        &mut self,
        verb: Verb,
        sources: &[PathBuf],
        into: &Path,
        mut choose: impl FnMut(&Clash) -> Resolve,
    ) -> Transferred {
        let step = self.history.open(Action::Transfer {
            verb,
            sources: sources.to_vec(),
            into: into.to_path_buf(),
        });
        let request = Transfer {
            verb,
            sources: sources.to_vec(),
            into: into.to_path_buf(),
        };
        let seen = match ops::survey(request) {
            Ok(mut survey) => {
                let fitted: Vec<Resolve> = survey
                    .clashes
                    .iter()
                    .map(|clash| clash.fitted(choose(clash)))
                    .collect();
                for (clash, choice) in survey.clashes.iter_mut().zip(fitted) {
                    clash.choice = choice;
                }
                let job = ops::start(survey, self.tools.clone(), || {});
                drain(&job.progress)
            }
            Err(_) => Vec::new(),
        };
        for progress in &seen {
            match progress {
                Progress::Placed(change) => self.history.record(step, Change::clone(change)),
                Progress::Amended { to, source } => self.history.amend(step, to, *source),
                Progress::Retracted(place) => self.history.retract(step, place),
                Progress::Bytes { .. } | Progress::Arriving(_) | Progress::Finished(_) => {}
            }
        }
        self.history.close(step);
        Transferred { step, seen }
    }

    pub(super) fn rename(&mut self, path: &Path, name: &str) -> Result<PathBuf, NameProblem> {
        let at = Print::read(path).map_err(|error| NameProblem::Io(error.to_string()))?;
        let to = ops::rename(path, name)?;
        if to.as_path() != path {
            let step = self.history.open(Action::Rename {
                from: path.to_path_buf(),
                to: to.clone(),
            });
            self.history.record(
                step,
                Change::Moved {
                    from: path.to_path_buf(),
                    to: to.clone(),
                    at,
                },
            );
            self.history.close(step);
        }
        Ok(to)
    }

    pub(super) fn make_file(&mut self, into: &Path, name: &str) -> Result<PathBuf, NameProblem> {
        let made = ops::new_file(into, name)?;
        self.made(Action::NewFile(made.clone()), &made);
        Ok(made)
    }

    pub(super) fn make_folder(&mut self, into: &Path, name: &str) -> Result<PathBuf, NameProblem> {
        let made = ops::new_folder(into, name)?;
        self.made(Action::NewFolder(made.clone()), &made);
        Ok(made)
    }

    fn made(&mut self, action: Action, path: &Path) {
        let at = Print::read(path).unwrap();
        let step = self.history.open(action);
        self.history.record(
            step,
            Change::Made {
                path: path.to_path_buf(),
                at,
            },
        );
        self.history.close(step);
    }

    pub(super) fn trash(&mut self, paths: &[PathBuf]) -> StepId {
        let step = self.history.open(Action::Trash(paths.to_vec()));
        let trashing = ops::trash(self.tools.bin.as_ref(), paths);
        for (from, held) in trashing.trashed {
            self.history.record(step, Change::Trashed { from, held });
        }
        self.history.close(step);
        step
    }

    pub(super) fn top_undo(&self) -> StepId {
        match self.history.top() {
            Top::Undo(step) => step.id,
            Top::Nothing | Top::Empty(_) | Top::Erased(_) => panic!("there is no step to undo"),
        }
    }

    pub(super) fn undo(&mut self, choose: impl FnMut(usize, &Clash) -> Resolve) -> Undid {
        let (step, dropping) = match self.history.top() {
            Top::Nothing => return Undid::Nothing,
            Top::Empty(found) | Top::Erased(found) => (found.id, true),
            Top::Undo(found) => (found.id, false),
        };
        if dropping {
            self.history.drop_step(step);
            return Undid::Dropped(step);
        }
        self.run(step, Direction::Undo, choose)
    }

    pub(super) fn redo(&mut self, choose: impl FnMut(usize, &Clash) -> Resolve) -> Undid {
        let Some(step) = self.history.redo_top().map(|found| found.id) else {
            return Undid::Nothing;
        };
        self.run(step, Direction::Redo, choose)
    }

    fn run(
        &mut self,
        step: StepId,
        direction: Direction,
        mut choose: impl FnMut(usize, &Clash) -> Resolve,
    ) -> Undid {
        let plan = self
            .history
            .plan(step, direction)
            .expect("the step is in the history");
        let planned: Vec<usize> = plan.items.iter().map(|item| item.index).collect();
        let mut replies = self.run_plan(&plan, direction, None);
        let mut asked = Vec::new();
        let rows = replies.iter().find_map(|reply| match reply {
            Reply::Asks(rows) => Some(rows.clone()),
            Reply::Item(..) | Reply::Done => None,
        });
        if let Some(rows) = rows {
            let mut choices = Choices::new();
            for (index, clash) in &rows {
                choices.insert(*index, choose(*index, clash));
                asked.push(*index);
            }
            replies = self.run_plan(&plan, direction, Some(choices));
        }
        let outcomes = outcomes_of(replies);
        self.history.settle(&Settled {
            step,
            outcomes: outcomes.clone(),
        });
        Undid::Ran {
            step,
            planned,
            outcomes,
            asked,
        }
    }

    fn run_plan(&self, plan: &Plan, direction: Direction, choices: Option<Choices>) -> Vec<Reply> {
        let undoing = start(
            plan.clone(),
            direction,
            choices,
            self.tools.clone(),
            None,
            || {},
        );
        drain(&undoing.result)
    }
}

pub(super) fn outcomes_of(replies: Vec<Reply>) -> Vec<(usize, Outcome)> {
    replies
        .into_iter()
        .filter_map(|reply| match reply {
            Reply::Item(index, outcome) => Some((index, *outcome)),
            Reply::Asks(_) | Reply::Done => None,
        })
        .collect()
}

pub(super) fn drain<T>(receiver: &Receiver<T>) -> Vec<T> {
    let mut seen = Vec::new();
    loop {
        match receiver.recv_timeout(WAIT) {
            Ok(item) => seen.push(item),
            Err(RecvTimeoutError::Disconnected) => return seen,
            Err(RecvTimeoutError::Timeout) => {
                panic!("nothing arrived for {WAIT:?} after {} messages", seen.len())
            }
        }
    }
}

pub(super) fn write_at(path: &Path, text: &str, stamp: u64) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(STAMP_BASE + stamp))
        .unwrap();
}

fn tree(root: &Path, read: bool) -> BTreeMap<PathBuf, Node> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            let node = if meta.is_dir() {
                pending.push(path.clone());
                Node::Folder
            } else if meta.is_file() {
                Node::File(if read {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                })
            } else {
                Node::Other
            };
            found.insert(path, node);
        }
    }
    found
}

pub(super) fn snapshot(root: &Path) -> BTreeMap<PathBuf, Node> {
    tree(root, true)
}

pub(super) fn contents(roots: &[&Path]) -> BTreeSet<Vec<u8>> {
    roots
        .iter()
        .flat_map(|root| snapshot(root).into_values())
        .filter_map(|node| match node {
            Node::File(bytes) if !bytes.is_empty() => Some(bytes),
            Node::File(_) | Node::Folder | Node::Other => None,
        })
        .collect()
}

pub(super) fn files(root: &Path) -> BTreeMap<String, String> {
    snapshot(root)
        .into_iter()
        .filter_map(|(path, node)| match node {
            Node::File(bytes) => {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                Some((relative, String::from_utf8_lossy(&bytes).into_owned()))
            }
            Node::Folder | Node::Other => None,
        })
        .collect()
}

pub(super) fn listing(rows: &[(&str, &str)]) -> BTreeMap<String, String> {
    rows.iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

pub(super) fn texts(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = files(root).into_values().collect();
    found.sort();
    found
}

fn own_paths(step: &Step) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for item in &step.items {
        match &item.change {
            Change::Moved { from, to, .. }
            | Change::Copied { from, to, .. }
            | Change::Crossed { from, to, .. } => {
                found.push(from.clone());
                found.push(to.clone());
            }
            Change::Replaced { at, held } => {
                found.push(at.clone());
                found.extend(held.iter().map(|held| held.item.clone()));
            }
            Change::Matched { from, held } | Change::Trashed { from, held } => {
                found.push(from.clone());
                found.extend(held.iter().map(|held| held.item.clone()));
            }
            Change::Emptied { folder, .. } => found.push(folder.clone()),
            Change::Made { path, .. } | Change::Erased { path } => found.push(path.clone()),
        }
        match &item.state {
            State::Undone(undone) => found.extend(undone_paths(undone)),
            State::Done => {}
        }
    }
    found
}

fn undone_paths(undone: &Undone) -> Vec<PathBuf> {
    match undone {
        Undone::MovedBack { to, .. } | Undone::Restored { to, .. } => vec![to.clone()],
        Undone::CopiedBack { to, far, .. } => {
            let mut found = vec![to.clone()];
            found.extend(far.iter().map(|held| held.item.clone()));
            found
        }
        Undone::Binned(held) => held.iter().map(|held| held.item.clone()).collect(),
        Undone::Removed | Undone::Remade { .. } => Vec::new(),
    }
}

fn belongs(path: &Path, own: &[PathBuf]) -> bool {
    own.iter().any(|item| {
        path.starts_with(item)
            || item.starts_with(path)
            || path.ancestors().any(|ancestor| beside(ancestor, item))
    })
}

fn beside(path: &Path, item: &Path) -> bool {
    if path == item || path.parent() != item.parent() {
        return false;
    }
    let (Some(name), Some(own)) = (path.file_name(), item.file_name()) else {
        return false;
    };
    let name = name.to_string_lossy();
    let own = own.to_string_lossy();
    let stem = item
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.starts_with(&format!("{own} (")) || name.starts_with(&format!("{stem} ("))
}

fn stray(
    before: &BTreeMap<PathBuf, Node>,
    after: &BTreeMap<PathBuf, Node>,
    own: &[PathBuf],
) -> Option<PathBuf> {
    let mut touched: BTreeSet<&PathBuf> = BTreeSet::new();
    for (path, node) in before {
        if after.get(path) != Some(node) {
            touched.insert(path);
        }
    }
    for (path, node) in after {
        if before.get(path) != Some(node) {
            touched.insert(path);
        }
    }
    let own: Vec<PathBuf> = own.iter().map(|path| folded(path)).collect();
    touched
        .into_iter()
        .find(|path| !belongs(&folded(path), &own))
        .cloned()
}

fn folded(path: &Path) -> PathBuf {
    if folds_case() {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

fn folds_case() -> bool {
    static FOLDS: OnceLock<bool> = OnceLock::new();
    *FOLDS.get_or_init(|| {
        let probe = tempfile::tempdir().unwrap();
        File::create(probe.path().join("case")).unwrap();
        probe.path().join("CASE").exists()
    })
}

fn children(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    found.sort();
    found
}

struct Rng(u64);

impl Rng {
    fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.draw() % bound as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.draw() % 100 < percent
    }
}

#[derive(Clone, Copy)]
enum Pick {
    Copy,
    Move,
    Duplicate,
    Rename,
    CaseRename,
    NewFile,
    NewFolder,
    Trash,
    Undo,
    Rewrite,
    Add,
    Remove,
    OutsideRename,
    EmptyBin,
    Redo,
    RoundTrip,
}

struct Reverted {
    outcomes: Vec<(usize, Outcome)>,
    asked: Vec<usize>,
}

fn opens(pick: Pick) -> bool {
    match pick {
        Pick::Copy
        | Pick::Move
        | Pick::Duplicate
        | Pick::Rename
        | Pick::CaseRename
        | Pick::NewFile
        | Pick::NewFolder
        | Pick::Trash => true,
        Pick::Undo
        | Pick::Redo
        | Pick::RoundTrip
        | Pick::Rewrite
        | Pick::Add
        | Pick::Remove
        | Pick::OutsideRename
        | Pick::EmptyBin => false,
    }
}

struct Walk {
    seed: u64,
    rng: Rng,
    rig: Rig,
    counter: u64,
    steps: Vec<String>,
}

impl Walk {
    fn new(seed: u64) -> Self {
        let mut walk = Self {
            seed,
            rng: Rng(seed),
            rig: Rig::new(),
            counter: 0,
            steps: Vec::new(),
        };
        walk.populate();
        walk
    }

    fn populate(&mut self) {
        let work = self.rig.work.clone();
        fs::create_dir(work.join("empty")).unwrap();
        for top in 0..3 {
            let folder = work.join(format!("d{top}"));
            for inner in 0..2 {
                let sub = folder.join(format!("s{inner}"));
                fs::create_dir_all(&sub).unwrap();
                for _ in 0..2 {
                    let name = self.fresh("f", ".txt");
                    self.write_new(&sub.join(name));
                }
            }
            let name = self.fresh("f", ".txt");
            self.write_new(&folder.join(name));
        }
        for _ in 0..2 {
            let name = self.fresh("f", ".txt");
            self.write_new(&work.join(name));
        }
    }

    fn run(&mut self) {
        for _ in 0..STEPS {
            let pick = self.pick();
            self.step(pick);
        }
        self.unwind("undo everything");
        self.steps.push("redo everything".into());
        for _ in 0..UNWIND_CAP {
            if self.rig.history.redo_top().is_none() {
                break;
            }
            self.revert(Direction::Redo);
        }
        self.unwind("undo everything again");
    }

    fn unwind(&mut self, label: &str) {
        self.steps.push(label.into());
        for _ in 0..UNWIND_CAP {
            if matches!(self.rig.history.top(), Top::Nothing) {
                break;
            }
            self.undo();
        }
    }

    fn pick(&mut self) -> Pick {
        let total: usize = MIX.iter().map(|(_, weight)| weight).sum();
        let mut roll = self.rng.below(total);
        for (pick, weight) in MIX {
            if roll < weight {
                return pick;
            }
            roll -= weight;
        }
        Pick::Undo
    }

    fn step(&mut self, pick: Pick) {
        let newest = self.rig.history.steps().last().map(|step| step.id);
        self.act(pick);
        let opened = self.rig.history.steps().last().map(|step| step.id) != newest;
        if opens(pick) && opened {
            assert!(
                self.rig.history.redo_top().is_none(),
                "a new action left something to redo"
            );
        }
    }

    fn act(&mut self, pick: Pick) {
        match pick {
            Pick::Copy => self.transfer(Verb::Copy),
            Pick::Move => self.transfer(Verb::Move),
            Pick::Duplicate => self.transfer(Verb::Duplicate),
            Pick::Rename => self.rename(false),
            Pick::CaseRename => self.rename(true),
            Pick::NewFile => self.make(false),
            Pick::NewFolder => self.make(true),
            Pick::Trash => self.trash(),
            Pick::Undo => {
                self.undo();
                if self.rng.chance(35) {
                    self.undo();
                }
            }
            Pick::Rewrite => self.rewrite(),
            Pick::Add => self.add(),
            Pick::Remove => self.remove(),
            Pick::OutsideRename => self.outside_rename(),
            Pick::EmptyBin => self.empty_bin(),
            Pick::Redo => {
                self.redo();
                if self.rng.chance(35) {
                    self.redo();
                }
            }
            Pick::RoundTrip => self.round_trip(),
        }
    }

    fn rel(&self, path: &Path) -> String {
        let text = path
            .strip_prefix(&self.rig.work)
            .unwrap_or(path)
            .display()
            .to_string();
        if text.is_empty() {
            ".".into()
        } else {
            text
        }
    }

    fn fresh(&mut self, stem: &str, extension: &str) -> String {
        self.counter += 1;
        format!("{stem}{}{extension}", self.counter)
    }

    fn write_new(&mut self, path: &Path) {
        self.counter += 1;
        let text = format!("content {} of seed {}", self.counter, self.seed);
        write_at(path, &text, self.counter * 10);
    }

    fn some_folder(&mut self) -> PathBuf {
        let mut folders = vec![self.rig.work.clone()];
        folders.extend(
            tree(&self.rig.work, false)
                .into_iter()
                .filter(|(_, node)| *node == Node::Folder)
                .map(|(path, _)| path),
        );
        let index = self.rng.below(folders.len());
        folders[index].clone()
    }

    fn some_file(&mut self) -> Option<PathBuf> {
        let files: Vec<PathBuf> = tree(&self.rig.work, false)
            .into_iter()
            .filter(|(_, node)| matches!(node, Node::File(_)))
            .map(|(path, _)| path)
            .collect();
        if files.is_empty() {
            return None;
        }
        let index = self.rng.below(files.len());
        Some(files[index].clone())
    }

    fn existing_name(&mut self, folder: &Path) -> Option<String> {
        let kids = children(folder);
        if kids.is_empty() {
            return None;
        }
        let kid = &kids[self.rng.below(kids.len())];
        Some(kid.file_name().unwrap().to_string_lossy().into_owned())
    }

    fn pick_items(&mut self) -> Vec<PathBuf> {
        let mut busy = vec![self.rig.work.clone()];
        busy.extend(
            tree(&self.rig.work, false)
                .into_iter()
                .filter(|(_, node)| *node == Node::Folder)
                .map(|(path, _)| path),
        );
        busy.retain(|folder| !children(folder).is_empty());
        if busy.is_empty() {
            return Vec::new();
        }
        let folder = busy[self.rng.below(busy.len())].clone();
        let mut kids = children(&folder);
        let wanted = 1 + self.rng.below(kids.len().min(3));
        let mut picked = Vec::new();
        for _ in 0..wanted {
            let index = self.rng.below(kids.len());
            picked.push(kids.remove(index));
        }
        picked
    }

    fn transfer(&mut self, verb: Verb) {
        let sources = self.pick_items();
        if sources.is_empty() {
            self.steps.push(format!("{verb:?}: nothing to pick"));
            return;
        }
        let into = if verb == Verb::Duplicate {
            sources[0].parent().unwrap().to_path_buf()
        } else {
            self.some_folder()
        };
        let shown: Vec<String> = sources.iter().map(|path| self.rel(path)).collect();
        let label = format!("{verb:?} [{}] into {}", shown.join(", "), self.rel(&into));
        self.steps.push(label.clone());
        let before = self.rig.held();
        let mut picked = Vec::new();
        let rng = &mut self.rng;
        let ran = self.rig.transfer(verb, &sources, &into, |_| {
            let choice = RESOLVES[rng.below(RESOLVES.len())];
            picked.push(choice);
            choice
        });
        self.steps.push(format!("  clash choices {picked:?}"));
        assert!(
            ran.seen.is_empty() || ran.done().is_some(),
            "the job thread ended before it finished"
        );
        self.rig.keeps(&before, &label);
    }

    fn rename(&mut self, case_only: bool) {
        let Some(path) = self.pick_items().into_iter().next() else {
            self.steps.push("rename: nothing to pick".into());
            return;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let new = if case_only {
            if name.to_uppercase() == name {
                name.to_lowercase()
            } else {
                name.to_uppercase()
            }
        } else if self.rng.chance(12) {
            let parent = path.parent().unwrap().to_path_buf();
            self.existing_name(&parent).unwrap_or_else(|| name.clone())
        } else if path.is_dir() {
            self.fresh("r", "")
        } else {
            self.fresh("r", ".txt")
        };
        let label = format!("rename {} to {new}", self.rel(&path));
        self.steps.push(label.clone());
        let before = self.rig.held();
        let result = self.rig.rename(&path, &new);
        self.steps.push(format!("  -> {result:?}"));
        self.rig.keeps(&before, &label);
    }

    fn make(&mut self, folder: bool) {
        let into = self.some_folder();
        let name = if self.rng.chance(10) {
            self.existing_name(&into)
        } else {
            None
        };
        let name = match name {
            Some(name) => name,
            None if folder => self.fresh("n", ""),
            None => self.fresh("n", ".txt"),
        };
        let label = format!(
            "new {} {name} in {}",
            if folder { "folder" } else { "file" },
            self.rel(&into)
        );
        self.steps.push(label.clone());
        let before = self.rig.held();
        let result = if folder {
            self.rig.make_folder(&into, &name)
        } else {
            self.rig.make_file(&into, &name)
        };
        self.steps.push(format!("  -> {result:?}"));
        self.rig.keeps(&before, &label);
    }

    fn trash(&mut self) {
        let items = self.pick_items();
        if items.is_empty() {
            self.steps.push("trash: nothing to pick".into());
            return;
        }
        let shown: Vec<String> = items.iter().map(|path| self.rel(path)).collect();
        let label = format!("trash [{}]", shown.join(", "));
        self.steps.push(label.clone());
        let before = self.rig.held();
        self.rig.trash(&items);
        self.rig.keeps(&before, &label);
    }

    fn undo(&mut self) {
        self.revert(Direction::Undo);
    }

    fn redo(&mut self) {
        self.revert(Direction::Redo);
    }

    fn revert(&mut self, direction: Direction) -> Option<Reverted> {
        let before = self.rig.held();
        let work_before = snapshot(&self.rig.work);
        let bin_before = snapshot(&self.rig.bin_root);
        let top = match direction {
            Direction::Undo => match self.rig.history.top() {
                Top::Undo(step) => Some(step.clone()),
                Top::Nothing | Top::Empty(_) | Top::Erased(_) => None,
            },
            Direction::Redo => self.rig.history.redo_top().cloned(),
        };
        self.steps.push(match &top {
            Some(step) => format!("{direction:?} {:?}", step.action),
            None => format!("{direction:?} with nothing to do"),
        });
        let rng = &mut self.rng;
        let choose = |_: usize, _: &Clash| {
            if rng.chance(50) {
                Resolve::Keep
            } else {
                Resolve::KeepBoth
            }
        };
        let undid = match direction {
            Direction::Undo => self.rig.undo(choose),
            Direction::Redo => self.rig.redo(choose),
        };
        match undid {
            Undid::Nothing => {
                self.steps.push("  no step".into());
                self.unchanged(&work_before, &bin_before);
                None
            }
            Undid::Dropped(step) => {
                self.steps
                    .push("  dropped a step with nothing to undo".into());
                assert!(
                    self.rig.history.step(step).is_none(),
                    "a dropped step is still in the history"
                );
                self.unchanged(&work_before, &bin_before);
                None
            }
            Undid::Ran {
                step,
                planned,
                outcomes,
                asked,
            } => {
                let summary: Vec<String> = outcomes
                    .iter()
                    .map(|(index, outcome)| match outcome {
                        Outcome::Undone(undone) => format!("{index}: undone {undone:?}"),
                        Outcome::Redone(change) => format!("{index}: redone {change:?}"),
                        Outcome::Stays(reason) => format!("{index}: stays {reason:?}"),
                        Outcome::Leaves(reason) => format!("{index}: leaves {reason:?}"),
                    })
                    .collect();
                self.steps
                    .push(format!("  asked {asked:?}, ran {}", summary.join("; ")));
                let top = top.expect("a step ran that was on top");
                assert_eq!(step, top.id, "the {direction:?} ran on another step");
                let replied: Vec<usize> = outcomes.iter().map(|(index, _)| *index).collect();
                assert_eq!(planned, replied, "an item got no outcome");
                let work_after = snapshot(&self.rig.work);
                if let Some(path) = stray(&work_before, &work_after, &own_paths(&top)) {
                    panic!(
                        "the {direction:?} changed {} outside its own step",
                        self.rel(&path)
                    );
                }
                self.rig.keeps(&before, "a step run");
                self.settled();
                Some(Reverted { outcomes, asked })
            }
        }
    }

    fn round_trip(&mut self) {
        let top = match self.rig.history.top() {
            Top::Undo(step) => Some(step.id),
            Top::Nothing | Top::Empty(_) | Top::Erased(_) => None,
        };
        let Some(id) = top else {
            self.steps.push("round trip: nothing to undo".into());
            return;
        };
        self.steps.push("round trip".into());
        let work_before = snapshot(&self.rig.work);
        let bin_before = texts(&self.rig.bin_root);
        let Some(undone) = self.revert(Direction::Undo) else {
            return;
        };
        let fully = self.rig.history.step(id).is_some_and(|step| step.undone());
        if !fully {
            self.steps
                .push("  not fully undone, nothing to redo".into());
            return;
        }
        assert_eq!(
            self.rig.history.redo_top().map(|step| step.id),
            Some(id),
            "a fully undone step is not first in line to redo"
        );
        let Some(redone) = self.revert(Direction::Redo) else {
            panic!("a fully undone step could not be redone");
        };
        let clean = undone.asked.is_empty()
            && redone.asked.is_empty()
            && undone
                .outcomes
                .iter()
                .all(|(_, outcome)| matches!(outcome, Outcome::Undone(_)))
            && redone
                .outcomes
                .iter()
                .all(|(_, outcome)| matches!(outcome, Outcome::Redone(_)));
        if !clean {
            return;
        }
        assert_eq!(
            snapshot(&self.rig.work),
            work_before,
            "undo then redo did not give back the same files"
        );
        assert_eq!(
            texts(&self.rig.bin_root),
            bin_before,
            "undo then redo did not give back the same trash"
        );
        let step = self.rig.history.step(id).expect("the redone step is kept");
        assert_eq!(
            step.waiting(),
            step.items.len(),
            "a redone item is not done"
        );
    }

    fn unchanged(
        &self,
        work_before: &BTreeMap<PathBuf, Node>,
        bin_before: &BTreeMap<PathBuf, Node>,
    ) {
        assert_eq!(
            &snapshot(&self.rig.work),
            work_before,
            "an undo with nothing to undo changed the work folder"
        );
        assert_eq!(
            &snapshot(&self.rig.bin_root),
            bin_before,
            "an undo with nothing to undo changed the bin"
        );
    }

    fn settled(&self) {
        match self.rig.history.top() {
            Top::Undo(step) | Top::Empty(step) | Top::Erased(step) => {
                assert!(
                    !step.undone(),
                    "the top step has every item undone: {step:?}"
                );
            }
            Top::Nothing => {}
        }
        if let Some(step) = self.rig.history.redo_top() {
            assert!(
                step.pending() && !step.open,
                "the step to redo has nothing to redo: {step:?}"
            );
        }
    }

    fn rewrite(&mut self) {
        let Some(path) = self.some_file() else {
            self.steps.push("outside rewrite: no file".into());
            return;
        };
        self.steps
            .push(format!("outside rewrite {}", self.rel(&path)));
        self.write_new(&path);
    }

    fn add(&mut self) {
        let into = self.some_folder();
        let name = self.fresh("o", ".txt");
        self.steps
            .push(format!("outside add {name} in {}", self.rel(&into)));
        self.write_new(&into.join(name));
    }

    fn remove(&mut self) {
        let Some(path) = self.some_file() else {
            self.steps.push("outside remove: no file".into());
            return;
        };
        self.steps
            .push(format!("outside remove {}", self.rel(&path)));
        fs::remove_file(&path).unwrap();
    }

    fn outside_rename(&mut self) {
        let Some(path) = self.some_file() else {
            self.steps.push("outside rename: no file".into());
            return;
        };
        let name = self.fresh("o", ".txt");
        self.steps
            .push(format!("outside rename {} to {name}", self.rel(&path)));
        fs::rename(&path, path.with_file_name(name)).unwrap();
    }

    fn empty_bin(&mut self) {
        self.steps.push("outside empty bin".into());
        if self.rig.bin_root.exists() {
            fs::remove_dir_all(&self.rig.bin_root).unwrap();
        }
    }
}

fn walk(seed: u64) {
    let mut walk = Walk::new(seed);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| walk.run()));
    if let Err(cause) = outcome {
        let reason = cause
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| cause.downcast_ref::<&str>().map(|text| (*text).to_owned()))
            .unwrap_or_default();
        panic!(
            "seed {seed}: {reason}\nsteps so far:\n{}",
            walk.steps.join("\n")
        );
    }
}

fn walks(seeds: Range<u64>) {
    for seed in seeds {
        walk(seed);
    }
}

#[test]
fn random_walks_lose_nothing_batch_1() {
    walks(1..26);
}

#[test]
fn random_walks_lose_nothing_batch_2() {
    walks(26..51);
}

#[test]
fn random_walks_lose_nothing_batch_3() {
    walks(51..76);
}

#[test]
fn random_walks_lose_nothing_batch_4() {
    walks(76..101);
}

#[test]
fn random_walks_lose_nothing_batch_5() {
    walks(101..126);
}

#[test]
fn random_walks_lose_nothing_batch_6() {
    walks(126..151);
}

#[test]
fn random_walks_lose_nothing_batch_7() {
    walks(151..176);
}

#[test]
fn random_walks_lose_nothing_batch_8() {
    walks(176..201);
}

#[test]
fn belongs_accepts_only_a_steps_own_paths_and_keep_both_names() {
    let own = vec![PathBuf::from("/w/a/f.txt"), PathBuf::from("/w/b/dir")];
    let cases = [
        ("/w/a/f.txt", true),
        ("/w/a", true),
        ("/w", true),
        ("/w/b/dir/x/y.txt", true),
        ("/w/a/f (2).txt", true),
        ("/w/a/f (2)", true),
        ("/w/b/dir (3)", true),
        ("/w/b/dir (3)/x.txt", true),
        ("/w/a/g.txt", false),
        ("/w/a/fx (2).txt", false),
        ("/w/a/other/f.txt", false),
        ("/w/b/dirx", false),
        ("/w/c", false),
        ("/w/c/f (2).txt", false),
    ];
    for (path, expected) in cases {
        assert_eq!(belongs(Path::new(path), &own), expected, "path: {path}");
    }
}

#[test]
fn stray_names_a_path_that_changed_outside_the_step() {
    let own = vec![PathBuf::from("/w/a/f.txt")];
    let before = BTreeMap::from([
        (PathBuf::from("/w/a/f.txt"), Node::File(b"one".to_vec())),
        (PathBuf::from("/w/a/g.txt"), Node::File(b"two".to_vec())),
    ]);
    let moved = BTreeMap::from([(PathBuf::from("/w/a/g.txt"), Node::File(b"two".to_vec()))]);
    let edited = BTreeMap::from([
        (PathBuf::from("/w/a/f.txt"), Node::File(b"one".to_vec())),
        (PathBuf::from("/w/a/g.txt"), Node::File(b"three".to_vec())),
    ]);
    assert_eq!(stray(&before, &moved, &own), None);
    assert_eq!(stray(&before, &before, &own), None);
    assert_eq!(
        stray(&before, &edited, &own),
        Some(PathBuf::from("/w/a/g.txt"))
    );
}
