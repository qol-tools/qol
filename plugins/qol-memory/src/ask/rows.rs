use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::AskOutput;
use crate::store::{NotesLayer, UnitsLayer};

pub const MAX_ROWS: usize = 8;
pub const TITLE_CHARS: usize = 140;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowRow {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub copy: String,
    pub key: String,
    pub kind: String,
}

pub fn title_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > TITLE_CHARS {
        let head: String = collapsed.chars().take(TITLE_CHARS).collect();
        format!("{head}...")
    } else {
        collapsed
    }
}

pub fn from_output(output: &AskOutput, units: &UnitsLayer, notes: &NotesLayer) -> Vec<FlowRow> {
    let mut rows: Vec<FlowRow> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    if let Some(answer) = &output.answer {
        rows.push(FlowRow {
            title: title_of(&answer.text),
            subtitle: Some(format!(
                "{} {} {}",
                output.verdict,
                answer.source_kind,
                date_of(answer.source_ts.as_deref())
            )),
            copy: answer.text.clone(),
            key: answer.key.clone(),
            kind: "answer".to_string(),
        });
        used.insert(answer.key.clone());
    }

    for recall in &output.recalled {
        if rows.len() >= MAX_ROWS {
            break;
        }
        if used.contains(&recall.key) {
            continue;
        }
        let hit = units
            .items
            .iter()
            .find(|unit| unit.key == recall.key)
            .map(|unit| (unit.text.clone(), unit.kind.clone()))
            .or_else(|| {
                notes
                    .items
                    .iter()
                    .find(|note| note.key == recall.key)
                    .map(|note| (note.text.clone(), note.cls.clone()))
            });
        let Some((text, kind)) = hit else {
            continue;
        };
        rows.push(FlowRow {
            title: title_of(&text),
            subtitle: Some(format!("{} {}", kind, date_of(recall.source_ts.as_deref()))),
            copy: text,
            key: recall.key.clone(),
            kind,
        });
        used.insert(recall.key.clone());
    }

    for hit in &output.skills.hits {
        if rows.len() >= MAX_ROWS {
            break;
        }
        let name = hit.name.clone().unwrap_or_else(|| hit.id.clone());
        let section = hit.section.clone().unwrap_or_default();
        rows.push(FlowRow {
            title: format!("{}: {}", name, section).trim().to_string(),
            subtitle: Some(format!("skill {}", hit.id)),
            copy: hit.content.clone().flatten().unwrap_or_default(),
            key: hit.id.clone(),
            kind: "skill".to_string(),
        });
    }

    rows
}

fn date_of(ts: Option<&str>) -> String {
    ts.map(|raw| raw.chars().take(10).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use std::path::PathBuf;

    use super::*;
    use crate::store::{Note, Unit};

    #[test]
    fn title_of_collapses_and_caps() {
        assert_eq!(
            title_of("  the   clipboard \n ring\t survives  "),
            "the clipboard ring survives"
        );
        let expected = format!("{}...", "x".repeat(TITLE_CHARS));
        assert_eq!(title_of(&"x".repeat(300)), expected);
        assert_eq!(title_of(&"x".repeat(TITLE_CHARS)), "x".repeat(TITLE_CHARS));
    }

    fn unit(key: &str, kind: &str, text: &str) -> Unit {
        Unit {
            key: key.to_string(),
            source: None,
            agent_home: None,
            file: None,
            session: None,
            cwd: None,
            kind: kind.to_string(),
            ts: None,
            text: text.to_string(),
        }
    }

    fn note(key: &str, cls: &str, text: &str) -> Note {
        Note {
            key: key.to_string(),
            cls: cls.to_string(),
            text: text.to_string(),
            source_key: None,
            source_ts: None,
            source_kind: None,
        }
    }

    fn output_fixture(recalled: Vec<Value>) -> AskOutput {
        serde_json::from_value(json!({
            "query": "how does the flow render",
            "verdict": "answered",
            "confidence": "high",
            "reason": "notes layer decision answer",
            "gates": {
                "NO_MEMORY_COV": 0.5,
                "FLOOR": 6.0,
                "NOTE_COV": 0.5,
                "NOTE_SCORE": 6.0,
                "UNIT_COV": 1.0,
                "UNIT_SCORE": 8.0,
                "UNIT_MARGIN": 1.5,
                "HIGH_MARGIN": 1.8
            },
            "non_default_gates": false,
            "answer": {
                "text": "the   clipboard ring survives tray restarts",
                "layer": "note",
                "key": "n-ans",
                "cls": "decision",
                "source_kind": "decision",
                "source_ts": "2026-08-04T08:00:00.000Z",
                "score": 14.0,
                "margin": 9.0
            },
            "recalled": recalled,
            "related": [],
            "signals": {
                "top_note_score": 14.0,
                "top_unit_score": 1.0,
                "unit_margin": 1.5,
                "note_token_coverage": 0.8,
                "unit_token_coverage": 0.5,
                "max_token_coverage": 0.8,
                "notes_run_ts": "2026-08-05T10-00-00-000Z",
                "snapshot_run_ts": "live",
                "live_units": true,
                "stale_layer": false,
                "recency_resolved": null
            },
            "counts": { "units": 8, "notes": 1 },
            "skills": {
                "status": "served",
                "hits": [
                    {
                        "id": "qol-arch-code",
                        "name": "qol-arch-code",
                        "score": 7.0,
                        "section": "strategy",
                        "content": "strategy body text",
                        "status": "served"
                    }
                ]
            },
            "notes": []
        }))
        .expect("fixture parses into AskOutput")
    }

    #[test]
    fn from_output_orders_answer_recalled_skills_and_caps() {
        let mut recalled = vec![
            json!({ "key": "n-ans", "cls": "decision", "score": 9.0 }),
            json!({
                "key": "u-1",
                "cls": "observation",
                "score": 5.2,
                "source_ts": "2026-08-02T12:00:00.000Z"
            }),
            json!({
                "key": "n-2",
                "cls": "flag",
                "score": 4.1,
                "source_ts": "2026-08-03T08:00:00.000Z"
            }),
            json!({ "key": "gone", "cls": "observation", "score": 3.0 }),
        ];
        for index in 2..=8usize {
            recalled.push(json!({
                "key": format!("u-{index}"),
                "cls": "observation",
                "score": 2.0
            }));
        }
        let output = output_fixture(recalled);
        let units = UnitsLayer {
            run: "live".to_string(),
            path: PathBuf::from("units.jsonl"),
            items: vec![
                unit("u-1", "capture", "unit one text"),
                unit("u-2", "user", "unit two text"),
                unit("u-3", "user", "unit three text"),
                unit("u-4", "user", "unit four text"),
                unit("u-5", "user", "unit five text"),
                unit("u-6", "user", "unit six text"),
                unit("u-7", "user", "unit seven text"),
                unit("u-8", "user", "unit eight text"),
            ],
        };
        let notes = NotesLayer {
            run: Some("2026-08-05T10-00-00-000Z".to_string()),
            items: vec![note("n-2", "flag", "note two text")],
        };

        let rows = from_output(&output, &units, &notes);

        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows[0].kind, "answer");
        assert_eq!(rows[0].key, "n-ans");
        assert_eq!(rows[0].title, "the clipboard ring survives tray restarts");
        assert_eq!(
            rows[0].subtitle.as_deref(),
            Some("answered decision 2026-08-04")
        );
        assert_eq!(rows[0].copy, "the   clipboard ring survives tray restarts");
        assert_eq!(rows[1].kind, "capture");
        assert_eq!(rows[1].key, "u-1");
        assert_eq!(rows[1].copy, "unit one text");
        assert_eq!(rows[1].subtitle.as_deref(), Some("capture 2026-08-02"));
        assert_eq!(rows[2].kind, "flag");
        assert_eq!(rows[2].key, "n-2");
        assert_eq!(rows[2].subtitle.as_deref(), Some("flag 2026-08-03"));
        assert_eq!(rows[7].key, "u-6");
        assert!(rows.iter().all(|row| row.key != "gone"));
        assert!(rows.iter().all(|row| row.kind != "skill"));
    }
}
