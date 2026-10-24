use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use gpui::{Keystroke, Modifiers};
use qol_gpui::{PathEdit, PathKey};

use super::ops::Work;
use super::ops_text;
use super::Browser;
use crate::listing::Entry;
use crate::ops::{self, Done, Undo};

pub(super) struct Renaming {
    pub target: PathBuf,
    pub edit: PathEdit,
    original: String,
}

impl Renaming {
    fn open(entry: &Entry) -> Self {
        let name = entry.name.to_string_lossy().into_owned();
        let mut edit = PathEdit::new(name.clone());
        let stem = stem_chars(&name, entry.opens_as_folder);
        if stem == name.chars().count() {
            edit.select_all();
        } else {
            edit.place_caret(stem);
            edit.key(&select_to_start(), || None);
        }
        Self {
            target: entry.path.clone(),
            edit,
            original: name,
        }
    }
}

fn select_to_start() -> Keystroke {
    Keystroke {
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::none()
        },
        key: "home".to_string(),
        key_char: None,
    }
}

fn stem_chars(name: &str, folder: bool) -> usize {
    if folder {
        return name.chars().count();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
}

impl Browser {
    pub fn trash_picks(&mut self) -> bool {
        let paths = self.operands();
        if paths.is_empty() {
            return false;
        }
        self.ops.notice = None;
        self.failure = None;
        self.trash_in_background(paths, true);
        true
    }

    fn trash_in_background(&mut self, paths: Vec<PathBuf>, journaled: bool) {
        let sender = self.ops.sender();
        let wake = self.waker();
        thread::spawn(move || {
            let _ = sender.send(Work::Trashed(ops::trash(&paths), journaled));
            wake();
        });
    }

    pub(super) fn trashed(&mut self, done: Done, journaled: bool) {
        if !done.placed.is_empty() {
            if journaled {
                self.ops.journal.record(&done);
            }
            self.ops.notice = Some(ops_text::trashed_note(done.placed.len()));
            self.picks.clear();
        }
        if !journaled {
            self.continue_undo();
        }
        self.failure = ops_text::failure_text(&done.failures);
        self.reload();
    }

    pub fn rename_open(&mut self) -> bool {
        if self.path.is_some() || self.ops.rename.is_some() {
            return false;
        }
        let Some(entry) = self.entry(self.cursor) else {
            return false;
        };
        let renaming = Renaming::open(entry);
        self.failure = None;
        self.ops.rename = Some(renaming);
        true
    }

    pub fn rename_field(&self) -> Option<&PathEdit> {
        self.ops.rename.as_ref().map(|renaming| &renaming.edit)
    }

    pub fn renaming(&self, entry: &Entry) -> bool {
        self.ops
            .rename
            .as_ref()
            .is_some_and(|renaming| renaming.target == entry.path)
    }

    pub fn rename_key(&mut self, keystroke: &Keystroke) -> bool {
        let Some(renaming) = self.ops.rename.as_mut() else {
            return false;
        };
        match renaming.edit.key(keystroke, || None) {
            PathKey::Changed => {
                self.failure = None;
                true
            }
            PathKey::Handled | PathKey::Ignored => true,
            PathKey::Cancel => {
                self.ops.rename = None;
                self.failure = None;
                true
            }
            PathKey::Submit => {
                self.commit_rename();
                true
            }
        }
    }

    fn commit_rename(&mut self) {
        let Some(renaming) = self.ops.rename.as_ref() else {
            return;
        };
        let name = renaming.edit.text().to_string();
        let target = renaming.target.clone();
        if name == renaming.original {
            self.ops.rename = None;
            return;
        }
        match ops::rename(&target, &name) {
            Ok(renamed) => {
                self.ops
                    .journal
                    .record_rename(target.clone(), renamed.clone());
                self.picks.rename(&target, renamed);
                self.ops.rename = None;
                self.failure = None;
                self.ops.notice = Some(format!("Renamed to {name}. ctrl+z undoes it."));
                self.reload();
            }
            Err(problem) => {
                self.failure = Some(ops_text::problem_text(&problem, &name));
            }
        }
    }

    pub fn new_folder(&mut self) -> bool {
        if self.loading() || self.path.is_some() {
            return false;
        }
        self.ops.rename = None;
        let folder = self.folder().to_path_buf();
        let free = ops::free_name(&folder, OsStr::new("New folder"));
        let name = ops_text::name_of(&free);
        match ops::new_folder(&folder, &name) {
            Ok(made) => {
                self.ops.journal.record_new_folder(made.clone());
                self.ops.rename_after = Some(made);
                self.failure = None;
                self.reload();
            }
            Err(problem) => {
                self.failure = Some(ops_text::problem_text(&problem, &name));
            }
        }
        true
    }

    pub(super) fn open_pending_rename(&mut self) -> bool {
        let Some(row) = self
            .ops
            .rename_after
            .as_deref()
            .and_then(|made| self.row_of(made))
        else {
            return false;
        };
        self.ops.rename_after = None;
        self.cursor = row;
        self.rename_open()
    }

    pub fn undo(&mut self) -> bool {
        let Some(undo) = self.ops.journal.undo() else {
            self.failure = Some("Nothing to undo".to_string());
            return false;
        };
        self.failure = None;
        self.ops.notice = Some("Undone.".to_string());
        match undo {
            Undo::Transfer { back, restore } => {
                self.ops.undoing = back;
                self.ops.restoring = restore;
                self.continue_undo();
            }
            Undo::Rename { from, to } => self.undo_rename(&from, &to),
            Undo::Remove(made) => self.undo_new_folder(&made),
            Undo::Restore(trashed) => self.restore_in_background(trashed),
            Undo::Trash { copies, restore } => {
                self.ops.restoring = restore;
                self.trash_in_background(copies, false);
            }
        }
        true
    }

    pub(super) fn continue_undo(&mut self) {
        if let Some(transfer) = self.ops.undoing.pop() {
            let _ = fs::create_dir_all(&transfer.into);
            self.begin_journaled(transfer, false);
            return;
        }
        let restore = std::mem::take(&mut self.ops.restoring);
        if !restore.is_empty() {
            self.restore_in_background(restore);
        }
    }

    fn restore_in_background(&mut self, trashed: Vec<PathBuf>) {
        let sender = self.ops.sender();
        let wake = self.waker();
        thread::spawn(move || {
            let _ = sender.send(Work::Restored(ops::restore(&trashed)));
            wake();
        });
    }

    fn undo_rename(&mut self, current: &Path, original: &Path) {
        let name = ops_text::name_of(original);
        match ops::rename(current, &name) {
            Ok(restored) => {
                self.picks.rename(current, restored);
                self.reload();
            }
            Err(problem) => {
                self.ops.notice = None;
                self.failure = Some(ops_text::problem_text(&problem, &name));
            }
        }
    }

    fn undo_new_folder(&mut self, made: &Path) {
        match fs::remove_dir(made) {
            Ok(()) => self.reload(),
            Err(error) => {
                self.ops.notice = None;
                self.failure = Some(format!(
                    "{} was not removed: {error}",
                    ops_text::name_of(made)
                ));
            }
        }
    }
}
