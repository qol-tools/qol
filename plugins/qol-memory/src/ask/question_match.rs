use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Topic,
    Fact,
    Definition,
    Method,
    Place,
    Time,
    Reason,
    Person,
    Boolean,
}

#[derive(Clone, Debug)]
pub(super) struct Question {
    intent: Intent,
    terms: Vec<String>,
}

impl Question {
    pub(super) fn parse(text: &str) -> Option<Self> {
        let normalized = normalize_question(text);
        let mut words: Vec<&str> = normalized.split_whitespace().collect();
        let mut intent = words
            .iter()
            .find_map(|word| match *word {
                "what" | "which" => Some(Intent::Fact),
                "how" => Some(Intent::Method),
                "where" => Some(Intent::Place),
                "when" => Some(Intent::Time),
                "why" => Some(Intent::Reason),
                "who" => Some(Intent::Person),
                _ => None,
            })
            .or_else(|| match words.first().copied() {
                Some("is" | "are" | "do" | "does" | "can" | "has" | "will") => {
                    Some(Intent::Boolean)
                }
                _ => None,
            })?;
        if let Some(position) = words.iter().position(|word| {
            matches!(
                *word,
                "what" | "which" | "how" | "where" | "when" | "why" | "who"
            )
        }) {
            words.rotate_left(position);
        }
        if intent == Intent::Fact
            && matches!(words.get(1), Some(&"is" | &"are" | &"s"))
            && !matches!(
                words.last(),
                Some(&"in" | &"of" | &"with" | &"from" | &"for")
            )
        {
            intent = Intent::Definition;
        }
        Self::from_words(intent, words)
    }

    pub(super) fn shorthand(text: &str) -> Option<Self> {
        let normalized = normalize_question(text);
        Self::from_words(Intent::Topic, normalized.split_whitespace().collect())
    }

    fn from_words(intent: Intent, words: Vec<&str>) -> Option<Self> {
        let terms = words
            .into_iter()
            .filter(|word| !super::stopword_set().contains(*word))
            .filter(|word| !matches!(*word, "s" | "please"))
            .filter(|word| {
                intent != Intent::Place
                    || !matches!(*word, "find" | "stored" | "located" | "location")
            })
            .map(|word| match (intent, word) {
                (Intent::Method | Intent::Topic, "open" | "start" | "run" | "launch" | "boot") => {
                    "launch".to_owned()
                }
                _ => crate::text::normalize(word),
            })
            .collect::<Vec<_>>();
        if intent == Intent::Topic
            && terms
                .iter()
                .filter(|term| term.as_str() != "launch")
                .collect::<HashSet<_>>()
                .len()
                < 2
        {
            return None;
        }
        (terms.len() >= 2).then_some(Self { intent, terms })
    }

    pub(super) fn covers(&self, query: &Self) -> bool {
        let topic = query.intent == Intent::Topic;
        if (!topic && self.intent != query.intent)
            || (!topic
                && self.intent == Intent::Definition
                && self.terms.len() != query.terms.len())
            || (self.intent == Intent::Method
                && !topic
                && query.terms.len() < 3
                && self.terms.len() != query.terms.len())
        {
            return false;
        }
        let negations = ["not", "never", "no", "without"];
        if !self
            .terms
            .iter()
            .filter(|term| negations.contains(&term.as_str()))
            .eq(query
                .terms
                .iter()
                .filter(|term| negations.contains(&term.as_str())))
        {
            return false;
        }
        if topic && self.intent != Intent::Method {
            return query.terms.iter().all(|term| {
                self.terms
                    .iter()
                    .any(|candidate| candidate == term || typo_matches(term, candidate))
            });
        }
        let mut stored = self.terms.iter();
        query
            .terms
            .iter()
            .all(|term| stored.any(|candidate| candidate == term || typo_matches(term, candidate)))
    }

    pub(super) fn same_subject(&self, other: &Self) -> bool {
        self.covers(other) || other.covers(self)
    }
}

fn normalize_question(text: &str) -> String {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && !matches!(character, '+' | '#'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn typo_matches(query: &str, stored: &str) -> bool {
    if query.len() < 5
        || stored.len() < 5
        || !query.bytes().all(|b| b.is_ascii_alphabetic())
        || !stored.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return false;
    }
    let (left, right) = (query.as_bytes(), stored.as_bytes());
    if left.len() == right.len() {
        let differences = left
            .iter()
            .zip(right)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect::<Vec<_>>();
        return matches!(differences.as_slice(), [a, b] if *b == *a + 1 && left[*a] == right[*b] && left[*b] == right[*a]);
    }
    let (short, long) = if left.len() < right.len() {
        (left, right)
    } else {
        (right, left)
    };
    if long.len() != short.len() + 1 {
        return false;
    }
    let first = short
        .iter()
        .zip(long)
        .position(|(a, b)| a != b)
        .unwrap_or(short.len());
    short[first..] == long[first + 1..]
}

#[derive(Clone, Debug)]
pub(super) struct Evidence {
    pub question: Question,
    pub recorded_question: String,
    pub recorded_answer: String,
    pub answer: String,
    pub display: String,
}

pub(super) fn evidence(text: &str) -> Option<Evidence> {
    if let Some(question_answer) = text.trim().strip_prefix("Q:") {
        let (question, answer) = question_answer.split_once(" A:")?;
        return Some(Evidence {
            question: Question::parse(question)?,
            recorded_question: question.trim().to_owned(),
            recorded_answer: answer.trim().to_owned(),
            answer: canonical_answer(answer),
            display: answer.trim().to_owned(),
        })
        .filter(|e| !e.answer.is_empty());
    }
    static QUESTIONS: OnceLock<Regex> = OnceLock::new();
    let captures = QUESTIONS
        .get_or_init(|| {
            Regex::new(
        r"(?i)(?:^|\.\s+)((?:what|which|how|where|when|why|who|does|do|is|are|can)\b[^:?\n]*?)[?:]"
    ).expect("recorded question regex")
        })
        .captures(text)?;
    let question = captures.get(1)?;
    let prefix = text[..captures.get(0)?.start()].trim();
    let suffix = text[captures.get(0)?.end()..].trim();
    let answer = if prefix.is_empty() { suffix } else { prefix };
    if answer.is_empty() || answer.chars().count() > 700 {
        return None;
    }
    Some(Evidence {
        question: Question::parse(question.as_str())?,
        recorded_question: question.as_str().to_owned(),
        recorded_answer: answer.to_owned(),
        answer: canonical_answer(answer),
        display: text.to_owned(),
    })
}

fn canonical_answer(answer: &str) -> String {
    let sentence = answer
        .trim()
        .split(". ")
        .next()
        .unwrap_or(answer)
        .trim_end_matches(['.', '?', '!']);
    if sentence.contains(['/', '\\', '`']) {
        crate::text::collapse_ws(sentence)
    } else {
        crate::text::collapse_ws_lower(sentence)
    }
}

pub(super) fn agree(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    fn commands(text: &str) -> HashSet<String> {
        static ALTERNATIVE: OnceLock<Regex> = OnceLock::new();
        let text = text.trim();
        let text = text
            .get(..4)
            .filter(|prefix| prefix.eq_ignore_ascii_case("run "))
            .map_or(text, |_| &text[4..]);
        if let Some(parts) = ALTERNATIVE
            .get_or_init(|| {
                Regex::new(r"^(\./[^\s()]+) ([A-Za-z0-9_-]+) \(or ([A-Za-z0-9_-]+)\)$")
                    .expect("command alternative regex")
            })
            .captures(text.trim_matches('`'))
        {
            return [
                format!("{} {}", &parts[1], &parts[2]),
                format!("{} {}", &parts[1], &parts[3]),
            ]
            .into_iter()
            .collect();
        }
        text.split(" or ")
            .map(|part| {
                let part = part.trim();
                let command = if part
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("run "))
                {
                    &part[4..]
                } else {
                    part
                };
                command.trim_matches('`')
            })
            .filter(|part| part.starts_with("./"))
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    }
    let a = commands(left);
    let b = commands(right);
    !a.is_empty() && !b.is_empty() && !a.is_disjoint(&b)
}
