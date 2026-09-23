use serde_json::Value;

use super::{unit_key, ASSISTANT_KIND, CAPTURE_KIND};
use crate::store::BOILERPLATE_MARKERS;

pub const QA_SOURCE: &str = "auto-qa";

const QUESTION_OPENERS: [&str; 10] = [
    "how", "what", "where", "which", "why", "when", "who", "can", "does", "is",
];

const QUESTION_MIN: usize = 8;
const QUESTION_MAX: usize = 200;
const ANSWER_MIN: usize = 20;
const ANSWER_MAX: usize = 700;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Question {
    key: String,
    session: String,
    cwd: String,
    agent_home: String,
    text: String,
    ts: Option<String>,
    host: Option<String>,
    file: Option<String>,
}

fn is_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.contains('?') {
        return true;
    }
    trimmed.split_whitespace().next().is_some_and(|token| {
        let lowered = token.to_lowercase();
        let stripped = lowered.trim_end_matches(|ch: char| ch.is_ascii_punctuation());
        QUESTION_OPENERS.contains(&stripped)
    })
}

pub fn qa_units(units: &[Value]) -> Vec<Value> {
    extend(units, &mut None)
}

pub fn extend(units: &[Value], pending: &mut Option<Question>) -> Vec<Value> {
    let mut out = Vec::new();
    for unit in units {
        match unit.get("kind").and_then(Value::as_str) {
            Some("user") => {
                *pending = question_from(unit);
            }
            Some(ASSISTANT_KIND) => {
                if let Some(question) = pending.as_ref() {
                    if let Some(pair) = pair(question, unit) {
                        out.push(pair);
                        *pending = None;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn question_from(unit: &Value) -> Option<Question> {
    let question: Question = serde_json::from_value(unit.clone()).ok()?;
    let trimmed = question.text.trim();
    if !is_question(trimmed)
        || !(QUESTION_MIN..=QUESTION_MAX).contains(&trimmed.chars().count())
        || question.key.is_empty()
        || question.session.trim().is_empty()
        || question.cwd.trim().is_empty()
        || question.agent_home.trim().is_empty()
    {
        return None;
    }
    if BOILERPLATE_MARKERS
        .iter()
        .any(|marker| trimmed.contains(marker))
    {
        return None;
    }
    Some(question)
}

fn pair(question: &Question, answer: &Value) -> Option<Value> {
    if session_of(answer)? != question.session
        || answer.get("agent_home")?.as_str()? != question.agent_home
        || answer.get("cwd")?.as_str()? != question.cwd
        || answer.get("assistant_final").and_then(Value::as_bool) == Some(false)
    {
        return None;
    }
    let answer_text = answer.get("text").and_then(Value::as_str)?;
    if !(ANSWER_MIN..=ANSWER_MAX).contains(&answer_text.trim().chars().count())
        || is_progress_announcement(answer_text)
        || BOILERPLATE_MARKERS
            .iter()
            .any(|marker| answer_text.contains(marker))
    {
        return None;
    }
    let text = format!("Q: {} A: {}", flat(&question.text), flat(answer_text));
    let ts = ts_of(answer).or(question.ts.as_deref())?;
    let scope = serde_json::json!([question.agent_home, question.cwd]).to_string();
    Some(serde_json::json!({
        "key": unit_key(QA_SOURCE, &scope, None, &text),
        "source": QA_SOURCE,
        "agent_home": question.agent_home,
        "host": question.host,
        "file": question.file,
        "cwd": question.cwd,
        "kind": CAPTURE_KIND,
        "ts": ts,
        "text": text,
        "session": question.session,
        "question_key": question.key,
        "answer_key": answer.get("key")?.as_str()?,
    }))
}

fn is_progress_announcement(text: &str) -> bool {
    let normalized = crate::text::collapse_ws_lower(text).replace('’', "'");
    let text = ["sure, ", "okay, ", "ok, "]
        .iter()
        .find_map(|prefix| normalized.strip_prefix(prefix))
        .unwrap_or(&normalized);
    let future = [
        "i'll ",
        "i will ",
        "we'll ",
        "we will ",
        "let me ",
        "let's ",
        "i'm going to ",
        "i am going to ",
    ];
    let ongoing = ["i'm ", "i am ", "we're ", "we are "];
    announces_action(text, &future, "check inspect investigate look read run test verify search start review trace")
        || announces_action(text, &ongoing, "checking inspecting investigating looking reading running testing verifying searching starting reviewing tracing working")
}

fn announces_action(text: &str, prefixes: &[&str], actions: &str) -> bool {
    prefixes
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|action| {
            let action = action.trim_end_matches(|ch: char| ch.is_ascii_punctuation());
            actions.split_whitespace().any(|verb| verb == action)
        })
}

fn flat(text: &str) -> String {
    text.trim().replace('\n', " ")
}

fn session_of(unit: &Value) -> Option<&str> {
    unit.get("session").and_then(Value::as_str)
}

fn ts_of(unit: &Value) -> Option<&str> {
    unit.get("ts").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::super::COMPACTION_KIND;
    use super::*;

    fn user_unit(text: &str, session: &str, cwd: &str) -> Value {
        serde_json::json!({
            "key": unit_key("test", session, None, text),
            "kind": "user",
            "agent_home": "/home/test-agent",
            "host": "source-host",
            "file": "source.jsonl",
            "session": session,
            "cwd": cwd,
            "ts": "2026-08-01T09:00:00.000Z",
            "text": text
        })
    }

    fn assistant_unit(text: &str, session: &str) -> Value {
        serde_json::json!({
            "key": unit_key("test", session, None, text),
            "kind": "assistant",
            "agent_home": "/home/test-agent",
            "cwd": "/repo",
            "session": session,
            "ts": "2026-08-01T09:00:01.000Z",
            "text": text
        })
    }

    #[test]
    fn is_question_matches_openers_and_question_marks() {
        assert!(is_question("how does the distill lock work"));
        assert!(is_question("Why."));
        assert!(is_question("CAN you hear me"));
        assert!(is_question("tell me about the store?"));
        assert!(is_question("hmm?"));
        assert!(!is_question("this is a plain statement"));
        assert!(!is_question(""));
    }

    #[test]
    fn pairs_next_same_session_assistant_and_skips_cross_session() {
        let units = vec![
            user_unit("how does the distill lock work", "s1", "/repo"),
            assistant_unit("the distill lock is a file lock held during append", "s2"),
            assistant_unit("the distill lock is held while appending units", "s1"),
        ];
        let out = qa_units(&units);
        assert_eq!(out.len(), 1);
        assert!(out[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Q: how does the distill lock work"));

        let sessionless = vec![
            serde_json::json!({
                "kind": "user",
                "cwd": "/repo",
                "ts": "2026-08-01T09:00:00.000Z",
                "text": "how does the distill lock work"
            }),
            serde_json::json!({
                "kind": "assistant",
                "ts": "2026-08-01T09:00:01.000Z",
                "text": "the distill lock is held while appending units"
            }),
        ];
        assert!(qa_units(&sessionless).is_empty());
    }

    #[test]
    fn bounds_are_enforced_on_both_sides() {
        let answer = "the answer spells out the whole flow in detail";
        let units = vec![
            user_unit("what?", "s1", "/repo"),
            assistant_unit(answer, "s1"),
            user_unit(&format!("what is {}", "x".repeat(195)), "s1", "/repo"),
            assistant_unit(answer, "s1"),
            user_unit("what is the shortest valid question here", "s1", "/repo"),
            assistant_unit("too short", "s1"),
            assistant_unit(&"y".repeat(701), "s1"),
            assistant_unit(answer, "s1"),
        ];
        let out = qa_units(&units);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]["text"],
            format!("Q: what is the shortest valid question here A: {answer}")
        );
    }

    #[test]
    fn derived_units_mirror_capture_shape_with_stable_keys() {
        let units = vec![
            user_unit("where is the\ningest state stored", "s1", "/repo"),
            assistant_unit("it lives beside units.jsonl in the store root", "s1"),
        ];
        let out = qa_units(&units);
        assert_eq!(out.len(), 1);
        let unit = &out[0];
        assert_eq!(unit["kind"], CAPTURE_KIND);
        assert_eq!(unit["source"], QA_SOURCE);
        assert_eq!(unit["cwd"], "/repo");
        assert_eq!(unit["session"], "s1");
        assert_eq!(unit["ts"], "2026-08-01T09:00:01.000Z");
        let text = format!(
            "Q: {} A: {}",
            "where is the ingest state stored", "it lives beside units.jsonl in the store root"
        );
        assert_eq!(unit["text"], text);
        assert_eq!(
            unit["key"],
            unit_key(QA_SOURCE, r#"["/home/test-agent","/repo"]"#, None, &text)
        );
        assert_eq!(unit["question_key"], units[0]["key"]);
        assert_eq!(unit["answer_key"], units[1]["key"]);
        assert_eq!(unit["host"], "source-host");
        let again = qa_units(&units);
        assert_eq!(again[0]["key"], unit["key"]);
    }

    #[test]
    fn boilerplate_questions_are_skipped() {
        let units = vec![
            user_unit(
                "how does the [qol session bridge] header work",
                "s1",
                "/repo",
            ),
            assistant_unit("it injects a bounded task into another terminal", "s1"),
        ];
        assert!(qa_units(&units).is_empty());
    }

    #[test]
    fn capture_and_compaction_units_never_pair() {
        let capture = serde_json::json!({
            "kind": CAPTURE_KIND,
            "session": "s1",
            "cwd": "/repo",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": "how does anything work"
        });
        let compaction = serde_json::json!({
            "kind": COMPACTION_KIND,
            "session": "s1",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": "this summary walks the whole conversation in enough detail to pair"
        });
        let units = vec![
            capture,
            user_unit("how does the pair pass work", "s1", "/repo"),
            compaction,
            assistant_unit("the pass walks units in order and pairs them", "s1"),
        ];
        let out = qa_units(&units);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]["text"],
            "Q: how does the pair pass work A: the pass walks units in order and pairs them"
        );
    }

    #[test]
    fn progress_announcements_do_not_consume_the_pending_question() {
        let answer = "The configuration lives beside the launcher plugin manifest.";
        for progress in [
            "I'll inspect the launcher configuration and report back.",
            "I’m checking the launcher configuration before answering.",
            "Let me search the repository for the launcher configuration.",
            "Sure, I will verify the launcher configuration first.",
            "We are investigating the launcher configuration location.",
            "I'm going to read the launcher configuration now.",
        ] {
            let units = [
                user_unit("where is the launcher configuration", "s1", "/repo"),
                assistant_unit(progress, "s1"),
                assistant_unit(answer, "s1"),
            ];
            let out = qa_units(&units);
            assert_eq!(out.len(), 1, "progress={progress}");
            assert!(out[0]["text"].as_str().unwrap().ends_with(answer));
        }
        for text in [
            "I checked the launcher configuration; it lives beside the plugin manifest.",
            "The launcher is checking its configuration on startup.",
            "I will use the launcher plugin directory as its permanent configuration location.",
        ] {
            assert!(!is_progress_announcement(text), "answer={text}");
        }
    }

    #[test]
    fn qa_identity_separates_homes_and_requires_matching_source_context() {
        let question = user_unit("where is the launcher configuration", "s1", "/repo");
        let answer = assistant_unit(
            "The configuration lives beside the launcher plugin manifest.",
            "s1",
        );
        let original = qa_units(&[question.clone(), answer.clone()]);
        for field in ["session", "cwd", "agent_home"] {
            let mut other = answer.clone();
            other[field] = serde_json::json!("different-context");
            assert!(
                qa_units(&[question.clone(), other]).is_empty(),
                "field={field}"
            );
        }
        for field in ["key", "session", "cwd", "agent_home"] {
            let mut unknown = question.clone();
            unknown.as_object_mut().unwrap().remove(field);
            assert!(
                qa_units(&[unknown, answer.clone()]).is_empty(),
                "missing={field}"
            );
        }
        let mut other_question = question;
        let mut other_answer = answer;
        other_question["agent_home"] = serde_json::json!("/other-home");
        other_answer["agent_home"] = serde_json::json!("/other-home");
        let other = qa_units(&[other_question, other_answer]);
        assert_eq!(other.len(), 1);
        assert_ne!(original[0]["key"], other[0]["key"]);
    }
}
