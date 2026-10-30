use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use qol_config::pages::{Go, Group, Notice, Page, Pages, Row};

use crate::session::{Act, Session, Task};

mod home;
mod last;
mod restart;
mod settings;
mod skipped;
mod updates;
mod words;

pub(crate) const ROOT: &str = "main";
const SLICE: usize = 40;

pub(crate) struct Board {
    pub(crate) pages: Pages,
    acts: BTreeMap<(String, String), (Act, Option<Go>)>,
}

#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) go: Option<Go>,
    pub(crate) notice: Option<Notice>,
    pub(crate) task: Option<Task>,
}

struct Sheet {
    id: String,
    page: Page,
    group: Option<Group>,
    acts: Vec<(String, Act, Option<Go>)>,
}

impl Sheet {
    fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            page: Page::new(title),
            group: None,
            acts: Vec::new(),
        }
    }

    fn group(&mut self, title: impl Into<String>, colophon: impl Into<String>) {
        self.flush();
        self.group = Some(Group::new(title, colophon));
    }

    fn heading(&mut self, title: impl Into<String>) {
        self.flush();
        self.group = Some(Group {
            title: title.into(),
            ..Group::default()
        });
    }

    fn row(&mut self, row: Row) {
        if let Some(group) = self.group.as_mut() {
            group.push(row);
        }
    }

    fn press(&mut self, row: Row, act: Act) {
        self.acts.push((row.id.clone(), act, None));
        self.row(row.press());
    }

    fn press_then(&mut self, row: Row, act: Act, go: Go) {
        self.acts.push((row.id.clone(), act, Some(go)));
        self.row(row.press());
    }

    fn focus(&mut self, row: impl Into<String>) {
        self.page.focus = Some(row.into());
    }

    fn flush(&mut self) {
        if let Some(group) = self.group.take() {
            self.page = std::mem::take(&mut self.page).group(group);
        }
    }

    fn list(self, title: &str, rows: Vec<Row>) -> Vec<Sheet> {
        let id = self.id.clone();
        let name = self.page.title.clone();
        let count = rows.len();
        let slices = count.div_ceil(SLICE);
        let mut rows = rows.into_iter();
        let mut head = Some(self);
        let mut sheets = Vec::new();
        for index in 0..slices {
            let mut sheet = head
                .take()
                .unwrap_or_else(|| Sheet::new(slice(&id, index), name.as_str()));
            sheet.heading(title);
            for row in rows.by_ref().take(SLICE) {
                sheet.row(row);
            }
            if slices > 1 {
                let next = (index + 1) % slices;
                let from = next * SLICE;
                sheet.row(
                    Row::new(
                        "more",
                        format!("Show {} to {}", from + 1, (from + SLICE).min(count)),
                    )
                    .detail(format!("{count} in all"))
                    .replace(slice(&id, next)),
                );
            }
            sheets.push(sheet);
        }
        sheets.extend(head);
        sheets
    }
}

fn slice(id: &str, index: usize) -> String {
    match index {
        0 => id.to_string(),
        _ => format!("{id}~{}", index + 1),
    }
}

impl Board {
    fn add(&mut self, mut sheet: Sheet) {
        sheet.flush();
        for (row, act, go) in sheet.acts {
            self.acts.insert((sheet.id.clone(), row), (act, go));
        }
        self.pages.insert(sheet.id, sheet.page);
    }
}

pub(crate) fn build(session: &Session, now: NaiveDateTime) -> Board {
    let mut board = Board {
        pages: Pages::new(ROOT),
        acts: BTreeMap::new(),
    };
    board.add(home::page(session, now));
    for sheet in updates::pages(session) {
        board.add(sheet);
    }
    for sheet in restart::pages(session, now) {
        board.add(sheet);
    }
    for sheet in settings::pages(session, now) {
        board.add(sheet);
    }
    if let Some(sheet) = skipped::page(session) {
        board.add(sheet);
    }
    for sheet in last::pages(session, now) {
        board.add(sheet);
    }
    board
}

pub(crate) fn press(session: &mut Session, page: &str, row: &str, now: NaiveDateTime) -> Outcome {
    let board = build(session, now);
    let Some((act, go)) = board
        .acts
        .get(&(page.to_string(), row.to_string()))
        .cloned()
    else {
        return Outcome {
            notice: Some(Notice::Failure(
                "That row changed before the press reached it".into(),
            )),
            ..Outcome::default()
        };
    };
    match session.apply(&act, now) {
        Ok(task) => Outcome {
            go,
            notice: None,
            task,
        },
        Err(problem) => Outcome {
            notice: Some(Notice::Failure(problem)),
            ..Outcome::default()
        },
    }
}

#[cfg(test)]
mod tests;
