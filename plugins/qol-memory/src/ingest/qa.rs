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
    let mut out = Vec::new();
    let mut pending: Option<&Value> = None;
    for unit in units {
        match unit.get("kind").and_then(Value::as_str) {
            Some("user") => {
                if is_valid_question(unit) {
                    pending = Some(unit);
                }
            }
            Some(ASSISTANT_KIND) => {
                if let Some(question) = pending {
                    if let Some(pair) = pair(question, unit) {
                        out.push(pair);
                        pending = None;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn is_valid_question(unit: &Value) -> bool {
    let Some(text) = unit.get("text").and_then(Value::as_str) else {
        return false;
    };
    if !is_question(text) {
        return false;
    }
    let trimmed = text.trim();
    if !(QUESTION_MIN..=QUESTION_MAX).contains(&trimmed.chars().count()) {
        return false;
    }
    if BOILERPLATE_MARKERS
        .iter()
        .any(|marker| trimmed.contains(marker))
    {
        return false;
    }
    let Some(cwd) = unit.get("cwd").and_then(Value::as_str) else {
        return false;
    };
    if cwd.trim().is_empty() {
        return false;
    }
    session_of(unit).is_some()
}

fn pair(question: &Value, answer: &Value) -> Option<Value> {
    let session = session_of(question)?;
    if session_of(answer)? != session {
        return None;
    }
    let answer_text = answer.get("text").and_then(Value::as_str)?;
    if !(ANSWER_MIN..=ANSWER_MAX).contains(&answer_text.trim().chars().count()) {
        return None;
    }
    let cwd = question.get("cwd").and_then(Value::as_str)?;
    let text = format!(
        "Q: {} A: {}",
        flat(question.get("text")?.as_str()?),
        flat(answer_text)
    );
    let ts = ts_of(answer).or_else(|| ts_of(question))?;
    Some(serde_json::json!({
        "key": unit_key(QA_SOURCE, cwd, None, &text),
        "source": QA_SOURCE,
        "cwd": cwd,
        "kind": CAPTURE_KIND,
        "ts": ts,
        "text": text,
        "session": session
    }))
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
            "kind": "user",
            "session": session,
            "cwd": cwd,
            "ts": "2026-08-01T09:00:00.000Z",
            "text": text
        })
    }

    fn assistant_unit(text: &str, session: &str) -> Value {
        serde_json::json!({
            "kind": "assistant",
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
        assert_eq!(unit["key"], unit_key(QA_SOURCE, "/repo", None, &text));
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
}
