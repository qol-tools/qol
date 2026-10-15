use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pages {
    pub root: String,
    #[serde(default)]
    pub pages: IndexMap<String, Page>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub title: String,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colophon: Option<String>,
    #[serde(default)]
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Tone::is_normal")]
    pub tone: Tone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Control>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<Enter>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub info: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub attention: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub busy: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Normal,
    Muted,
    Attention,
    Danger,
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Toggle(bool),
    Button(String),
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enter {
    Open(String),
    Replace(String),
    Press,
    Back,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    #[serde(default)]
    pub open: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Press {
    pub page: String,
    pub row: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PressReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go: Option<Go>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<Pages>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Go {
    Back,
    Root,
    Open(String),
    Replace(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    Success(String),
    Failure(String),
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Tone {
    fn is_normal(&self) -> bool {
        *self == Tone::Normal
    }
}

impl Pages {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            pages: IndexMap::new(),
        }
    }

    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.get(id)
    }

    pub fn insert(&mut self, id: impl Into<String>, page: Page) {
        self.pages.insert(id.into(), page);
    }

    pub fn only(&self, open: &[String]) -> Pages {
        let mut only = Pages::new(self.root.clone());
        for id in std::iter::once(&self.root).chain(open) {
            if let Some(page) = self.pages.get(id) {
                only.pages.entry(id.clone()).or_insert_with(|| page.clone());
            }
        }
        only
    }

    pub fn dead_ends(&self) -> Vec<String> {
        let mut dead = Vec::new();
        for (id, page) in &self.pages {
            for row in page.rows() {
                if let Some(Enter::Open(target) | Enter::Replace(target)) = &row.enter {
                    if !self.pages.contains_key(target) {
                        dead.push(format!(
                            "row `{}` on page `{id}` opens missing page `{target}`",
                            row.id
                        ));
                    }
                }
            }
        }
        dead
    }

    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.pages.contains_key(&self.root) {
            problems.push(format!("the root page `{}` is missing", self.root));
        }
        for (id, page) in &self.pages {
            let mut seen = std::collections::HashSet::new();
            for row in page.rows() {
                if !seen.insert(row.id.as_str()) {
                    problems.push(format!("page `{id}` repeats row `{}`", row.id));
                }
                if row.info && row.enter.is_some() {
                    problems.push(format!(
                        "row `{}` on page `{id}` is info yet reacts to enter",
                        row.id
                    ));
                }
            }
        }
        problems
    }
}

impl Page {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            groups: Vec::new(),
            focus: None,
        }
    }

    pub fn focus(mut self, row: impl Into<String>) -> Self {
        self.focus = Some(row.into());
        self
    }

    pub fn group(mut self, group: Group) -> Self {
        if !group.rows.is_empty() {
            self.groups.push(group);
        }
        self
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.groups.iter().flat_map(|group| group.rows.iter())
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows().find(|row| row.id == id)
    }
}

impl Group {
    pub fn new(title: impl Into<String>, colophon: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            colophon: Some(colophon.into()),
            rows: Vec::new(),
        }
    }

    pub fn row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    pub fn push(&mut self, row: Row) {
        self.rows.push(row);
    }
}

impl Row {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn info(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            info: true,
            ..Self::new(id, label)
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn toggle(mut self, on: bool) -> Self {
        self.control = Some(Control::Toggle(on));
        self.enter = Some(Enter::Press);
        self
    }

    pub fn button(mut self, label: impl Into<String>) -> Self {
        self.control = Some(Control::Button(label.into()));
        self
    }

    pub fn check(mut self, checked: bool) -> Self {
        self.control = checked.then_some(Control::Check);
        self
    }

    pub fn open(mut self, page: impl Into<String>) -> Self {
        self.enter = Some(Enter::Open(page.into()));
        self
    }

    pub fn replace(mut self, page: impl Into<String>) -> Self {
        self.enter = Some(Enter::Replace(page.into()));
        self
    }

    pub fn press(mut self) -> Self {
        self.enter = Some(Enter::Press);
        self
    }

    pub fn back(mut self) -> Self {
        self.enter = Some(Enter::Back);
        self
    }

    pub fn attention(mut self, attention: bool) -> Self {
        self.attention = attention;
        self
    }

    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
}

impl PressReply {
    pub fn go(go: Go) -> Self {
        Self {
            go: Some(go),
            ..Self::default()
        }
    }

    pub fn notice(mut self, notice: Notice) -> Self {
        self.notice = Some(notice);
        self
    }

    pub fn with_pages(mut self, pages: Pages) -> Self {
        self.pages = Some(pages);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Pages {
        let mut pages = Pages::new("main");
        pages.insert(
            "main",
            Page::new("Update Manager").group(
                Group::new("updates", "what is ready to install or finish.")
                    .row(
                        Row::new("waiting", "4 updates waiting")
                            .detail("All 4 can install")
                            .open("waiting")
                            .attention(true),
                    )
                    .row(Row::new("guard", "Wait while in use").toggle(true)),
            ),
        );
        pages.insert(
            "waiting",
            Page::new("waiting").group(
                Group::new("waiting", "each tagged with its updater.").row(
                    Row::info("apparmor", "AppArmor 4.0.1")
                        .detail("3 packages, held by Ubuntu")
                        .tag("Update Manager")
                        .dim(true),
                ),
            ),
        );
        pages
    }

    #[test]
    fn wire_shape_is_compact_and_tagged() {
        let value = serde_json::to_value(sample()).expect("serialize");
        assert_eq!(value["root"], "main");
        let rows = &value["pages"]["main"]["groups"][0]["rows"];
        assert_eq!(
            rows[0],
            json!({
                "id": "waiting",
                "label": "4 updates waiting",
                "detail": "All 4 can install",
                "enter": {"open": "waiting"},
                "attention": true
            })
        );
        assert_eq!(
            rows[1],
            json!({
                "id": "guard",
                "label": "Wait while in use",
                "control": {"toggle": true},
                "enter": "press"
            })
        );
        let back: Pages = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, sample());
    }

    #[test]
    fn empty_groups_are_dropped() {
        let page = Page::new("x").group(Group::new("empty", "nothing here."));
        assert!(page.groups.is_empty());
    }

    #[test]
    fn problems_name_repeats_and_dead_ends_name_missing_targets() {
        assert!(sample().problems().is_empty());
        assert!(sample().dead_ends().is_empty());
        let mut broken = sample();
        broken.root = "home".into();
        broken.insert(
            "dup",
            Page::new("dup").group(
                Group::new("g", "c.")
                    .row(Row::new("a", "A").open("nowhere"))
                    .row(Row::new("a", "A again").replace("elsewhere"))
                    .row(Row::info("b", "B").press()),
            ),
        );
        assert_eq!(
            broken.problems(),
            [
                "the root page `home` is missing",
                "page `dup` repeats row `a`",
                "row `b` on page `dup` is info yet reacts to enter",
            ]
        );
        assert_eq!(
            broken.dead_ends(),
            [
                "row `a` on page `dup` opens missing page `nowhere`",
                "row `a` on page `dup` opens missing page `elsewhere`",
            ]
        );
    }

    #[test]
    fn only_keeps_the_root_and_the_open_pages_that_exist() {
        let only = sample().only(&["waiting".into(), "gone".into(), "main".into()]);
        assert_eq!(only.root, "main");
        assert_eq!(only.pages.keys().collect::<Vec<_>>(), ["main", "waiting"]);
        assert!(only.problems().is_empty());
        assert_eq!(
            sample().only(&[]).pages.keys().collect::<Vec<_>>(),
            ["main"]
        );
    }

    #[test]
    fn a_press_names_the_open_pages_only_when_there_are_some() {
        let bare = Press {
            page: "main".into(),
            row: "guard".into(),
            open: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&bare).expect("serialize"),
            json!({"page": "main", "row": "guard"})
        );
        let parsed: Press =
            serde_json::from_value(json!({"page": "main", "row": "guard", "open": ["main"]}))
                .expect("parse press");
        assert_eq!(parsed.open, ["main"]);
        let stack: Stack = serde_json::from_value(json!({})).expect("empty stack");
        assert!(stack.open.is_empty());
        assert_eq!(
            serde_json::to_value(Row::new("next", "Next").replace("waiting~2")).expect("serialize")
                ["enter"],
            json!({"replace": "waiting~2"})
        );
    }

    #[test]
    fn press_reply_parses_every_go() {
        for (wire, go) in [
            (json!("back"), Go::Back),
            (json!("root"), Go::Root),
            (
                json!({"open": "install:all"}),
                Go::Open("install:all".into()),
            ),
            (json!({"replace": "ask"}), Go::Replace("ask".into())),
        ] {
            let reply: PressReply =
                serde_json::from_value(json!({"go": wire, "notice": {"failure": "no"}}))
                    .expect("parse reply");
            assert_eq!(reply.go, Some(go));
            assert_eq!(reply.notice, Some(Notice::Failure("no".into())));
        }
        let empty: PressReply = serde_json::from_value(json!({})).expect("empty reply");
        assert_eq!(empty, PressReply::default());
    }
}
