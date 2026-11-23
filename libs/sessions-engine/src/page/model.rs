use std::collections::HashSet;

use serde::{Deserialize, Serialize};

const TITLE_MAX: usize = 80;
const TAB_MAX: usize = 16;
const ITEM_ID_MAX: usize = 32;
const BLOCKS_MAX: usize = 24;
const ITEMS_MAX: usize = 40;
const LINE_MAX: usize = 240;
const TEXT_MAX: usize = 1_000;
const URL_MAX: usize = 2_048;
pub(super) const NOTE_MAX: usize = 2_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tone {
    #[default]
    Info,
    Good,
    Warn,
    Bad,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StepState {
    Done,
    Now,
    #[default]
    Todo,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Verdict {
    Pass,
    Fail,
    Skip,
}

impl Verdict {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) label: String,
    pub(crate) value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tone: Option<Tone>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    pub(crate) id: String,
    pub(crate) text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) state: StepState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Link {
    pub(crate) label: String,
    pub(crate) url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Action {
    pub(crate) id: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Block {
    Kv {
        rows: Vec<Row>,
    },
    Checklist {
        items: Vec<Check>,
    },
    Steps {
        items: Vec<Step>,
    },
    Links {
        items: Vec<Link>,
    },
    Callout {
        #[serde(default)]
        tone: Tone,
        text: String,
    },
    Actions {
        items: Vec<Action>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) owner: String,
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tab: Option<String>,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Answer {
    pub(crate) item: String,
    pub(crate) verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
    pub(crate) at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Stored {
    pub(crate) page: Page,
    pub(crate) set_ms: u64,
    pub(crate) updated_ms: u64,
    #[serde(default)]
    pub(crate) answers: Vec<Answer>,
}

impl Page {
    pub(super) fn check_ids(&self) -> impl Iterator<Item = &str> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Checklist { items } => Some(items),
                Block::Kv { .. }
                | Block::Steps { .. }
                | Block::Links { .. }
                | Block::Callout { .. }
                | Block::Actions { .. } => None,
            })
            .flatten()
            .map(|check| check.id.as_str())
    }

    pub(super) fn checked(self) -> Result<Self, String> {
        if !crate::record::is_key(&self.id) {
            return Err(format!(
                "page id `{}` must be lowercase letters, digits and dashes, at most 64 characters",
                self.id
            ));
        }
        if !crate::record::is_key(&self.owner) {
            return Err(format!(
                "page owner `{}` must be the spawn key of the fork that owns the page",
                self.owner
            ));
        }
        if self.blocks.is_empty() || self.blocks.len() > BLOCKS_MAX {
            return Err(format!("a page has 1 to {BLOCKS_MAX} blocks"));
        }
        let mut ids = HashSet::new();
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| block.checked(&mut ids))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            title: line("title", &self.title, TITLE_MAX)?,
            tab: self.tab.map(|tab| line("tab", &tab, TAB_MAX)).transpose()?,
            blocks,
            ..self
        })
    }
}

impl Block {
    fn checked(self, ids: &mut HashSet<String>) -> Result<Self, String> {
        Ok(match self {
            Self::Kv { rows } => Self::Kv {
                rows: items("kv", rows, |row| {
                    Ok(Row {
                        label: line("kv label", &row.label, LINE_MAX)?,
                        value: line("kv value", &row.value, LINE_MAX)?,
                        tone: row.tone,
                    })
                })?,
            },
            Self::Checklist { items: checks } => Self::Checklist {
                items: items("checklist", checks, |check| {
                    Ok(Check {
                        id: item_id(&check.id, ids)?,
                        text: line("checklist text", &check.text, LINE_MAX)?,
                        detail: check
                            .detail
                            .map(|detail| line("checklist detail", &detail, TEXT_MAX))
                            .transpose()?,
                    })
                })?,
            },
            Self::Steps { items: steps } => Self::Steps {
                items: items("steps", steps, |step| {
                    Ok(Step {
                        text: line("steps text", &step.text, LINE_MAX)?,
                        state: step.state,
                    })
                })?,
            },
            Self::Links { items: links } => Self::Links {
                items: items("links", links, |link| {
                    Ok(Link {
                        label: line("links label", &link.label, LINE_MAX)?,
                        url: url(&link.url)?,
                    })
                })?,
            },
            Self::Callout { tone, text } => Self::Callout {
                tone,
                text: line("callout text", &text, TEXT_MAX)?,
            },
            Self::Actions { items: actions } => Self::Actions {
                items: items("actions", actions, |action| {
                    Ok(Action {
                        id: item_id(&action.id, ids)?,
                        label: line("actions label", &action.label, LINE_MAX)?,
                    })
                })?,
            },
        })
    }
}

fn items<T>(
    kind: &str,
    list: Vec<T>,
    check: impl FnMut(T) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    if list.is_empty() || list.len() > ITEMS_MAX {
        return Err(format!("a {kind} block has 1 to {ITEMS_MAX} entries"));
    }
    list.into_iter().map(check).collect()
}

fn item_id(id: &str, ids: &mut HashSet<String>) -> Result<String, String> {
    let shaped = !id.is_empty()
        && id.len() <= ITEM_ID_MAX
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !shaped {
        return Err(format!(
            "item id `{id}` must be lowercase letters, digits and dashes, at most {ITEM_ID_MAX} characters"
        ));
    }
    if !ids.insert(id.to_owned()) {
        return Err(format!("item id `{id}` is used twice on the page"));
    }
    Ok(id.to_owned())
}

fn url(text: &str) -> Result<String, String> {
    let shaped = (text.starts_with("https://") || text.starts_with("http://"))
        && text.len() <= URL_MAX
        && !text
            .chars()
            .any(|character| character.is_whitespace() || is_hidden(character));
    if shaped {
        Ok(text.to_owned())
    } else {
        Err(format!(
            "link url `{text}` must be one http or https address of at most {URL_MAX} characters"
        ))
    }
}

pub(super) fn line(field: &str, text: &str, max: usize) -> Result<String, String> {
    let cleaned = text
        .chars()
        .map(
            |character| {
                if is_hidden(character) {
                    ' '
                } else {
                    character
                }
            },
        )
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return Err(format!("{field} is empty"));
    }
    if cleaned.chars().count() > max {
        return Err(format!("{field} is over {max} characters"));
    }
    Ok(cleaned)
}

fn is_hidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}
