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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trail: Vec<TrailEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<DetailField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetailField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrailEntry {
    pub at: String,
    pub tag: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub struck: bool,
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
        let mut trail = vec![TrailEntry {
            at: date_of(answer.source_ts.as_deref()),
            tag: "true now".to_string(),
            text: answer.text.clone(),
            struck: false,
        }];
        if let Some(Some(superseded)) = &answer.superseded {
            for entry in superseded {
                trail.push(TrailEntry {
                    at: date_of(entry.source_ts.as_deref()),
                    tag: "superseded".to_string(),
                    text: entry.text.clone(),
                    struck: true,
                });
            }
        }
        let mut detail = Vec::new();
        detail.extend(detail_field("verdict", Some(output.verdict.clone())));
        detail.extend(detail_field("confidence", Some(output.confidence.clone())));
        detail.extend(detail_field("layer", Some(answer.layer.clone())));
        detail.extend(detail_field("class", answer.cls.clone()));
        detail.extend(detail_field("source", Some(answer.source_kind.clone())));
        detail.extend(detail_field("when", answer.source_ts.clone()));
        detail.extend(detail_field("score", Some(format!("{:.2}", answer.score))));
        detail.extend(detail_field(
            "margin",
            answer.margin.map(|raw| format!("{raw:.2}")),
        ));
        detail.extend(detail_field("session", answer.session.clone()));
        detail.extend(detail_field("key", Some(answer.key.clone())));
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
            trail,
            detail,
        });
        used.insert(answer.key.clone());
    }

    if let Some(units_out) = &output.units {
        for unit in units_out {
            if rows.len() >= MAX_ROWS {
                break;
            }
            if unit.kind == crate::ingest::CAPTURE_KIND && !used.contains(&unit.key) {
                let mut detail = Vec::new();
                detail.extend(detail_field("kind", Some(unit.kind.clone())));
                detail.extend(detail_field("when", unit.ts.clone()));
                detail.extend(detail_field("session", unit.session.clone()));
                detail.extend(detail_field("cwd", unit.cwd.clone()));
                detail.extend(detail_field("score", Some(format!("{:.2}", unit.score))));
                detail.extend(detail_field("key", Some(unit.key.clone())));
                rows.push(FlowRow {
                    title: title_of(&unit.text),
                    subtitle: Some(format!("{} {}", unit.kind, date_of(unit.ts.as_deref()))),
                    copy: unit.text.clone(),
                    key: unit.key.clone(),
                    kind: unit.kind.clone(),
                    trail: vec![TrailEntry {
                        at: date_of(unit.ts.as_deref()),
                        tag: unit.kind.clone(),
                        text: unit.text.clone(),
                        struck: false,
                    }],
                    detail,
                });
                used.insert(unit.key.clone());
            }
        }
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
        let mut detail = Vec::new();
        detail.extend(detail_field("kind", Some(kind.clone())));
        detail.extend(detail_field("when", recall.source_ts.clone()));
        detail.extend(detail_field("score", Some(format!("{:.2}", recall.score))));
        detail.extend(detail_field("key", Some(recall.key.clone())));
        rows.push(FlowRow {
            title: title_of(&text),
            subtitle: Some(format!("{} {}", kind, date_of(recall.source_ts.as_deref()))),
            copy: text.clone(),
            key: recall.key.clone(),
            kind: kind.clone(),
            trail: vec![TrailEntry {
                at: date_of(recall.source_ts.as_deref()),
                tag: kind.clone(),
                text: text.clone(),
                struck: false,
            }],
            detail,
        });
        used.insert(recall.key.clone());
    }

    for hit in &output.skills.hits {
        if rows.len() >= MAX_ROWS {
            break;
        }
        let name = hit.name.clone().unwrap_or_else(|| hit.id.clone());
        let section = hit.section.clone().unwrap_or_default();
        let mut detail = Vec::new();
        detail.extend(detail_field("skill", Some(hit.id.clone())));
        detail.extend(detail_field("section", hit.section.clone()));
        detail.extend(detail_field("status", Some(hit.status.clone())));
        detail.extend(detail_field("head", hit.head.clone()));
        detail.extend(detail_field(
            "dirty",
            hit.dirty.map(|flag| flag.to_string()),
        ));
        rows.push(FlowRow {
            title: format!("{}: {}", name, section).trim().to_string(),
            subtitle: Some(format!("skill {}", hit.id)),
            copy: hit.content.clone().flatten().unwrap_or_default(),
            key: hit.id.clone(),
            kind: "skill".to_string(),
            trail: Vec::new(),
            detail,
        });
    }

    rows
}

fn detail_field(label: &str, value: Option<String>) -> Option<DetailField> {
    let value = value.filter(|raw| !raw.is_empty())?;
    Some(DetailField {
        label: label.to_string(),
        value,
    })
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
    use crate::ask::Superseded;
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

    fn empty_layers() -> (UnitsLayer, NotesLayer) {
        (
            UnitsLayer {
                run: "live".to_string(),
                path: PathBuf::from("units.jsonl"),
                items: vec![],
            },
            NotesLayer {
                run: None,
                items: vec![],
            },
        )
    }

    fn output_fixture(answer_key: &str, recalled: Vec<Value>, units: Value) -> AskOutput {
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
                "key": answer_key,
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
            "units": units,
            "notes": []
        }))
        .expect("fixture parses into AskOutput")
    }

    #[test]
    fn from_output_surfaces_capture_units_after_the_answer() {
        let capture_unit_json = json!({
            "key": "c-1",
            "score": 12.0,
            "kind": "capture",
            "text": "captured fact text",
            "ts": "2026-08-02T12:00:00.000Z",
            "snippet": "captured fact text"
        });
        let output = output_fixture(
            "n-ans",
            vec![],
            json!([
                capture_unit_json,
                {
                    "key": "u-9",
                    "score": 11.0,
                    "kind": "user",
                    "text": "user unit text",
                    "snippet": "user unit text"
                }
            ]),
        );
        let empty_units = UnitsLayer {
            run: "live".to_string(),
            path: PathBuf::from("units.jsonl"),
            items: vec![],
        };
        let empty_notes = NotesLayer {
            run: None,
            items: vec![],
        };

        let rows = from_output(&output, &empty_units, &empty_notes);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "answer");
        assert_eq!(rows[1].kind, "capture");
        assert_eq!(rows[1].key, "c-1");
        assert_eq!(rows[1].subtitle.as_deref(), Some("capture 2026-08-02"));
        assert_eq!(rows[1].copy, "captured fact text");
        assert_eq!(rows[2].kind, "skill");

        let duplicate_output = output_fixture("c-1", vec![], json!([capture_unit_json]));
        let duplicate_rows = from_output(&duplicate_output, &empty_units, &empty_notes);
        assert_eq!(duplicate_rows.len(), 2);
        assert_eq!(duplicate_rows[0].kind, "answer");
        assert_eq!(duplicate_rows[0].key, "c-1");
        assert_eq!(duplicate_rows[1].kind, "skill");
    }

    #[test]
    fn answer_trail_orders_true_now_then_struck_superseded_oldest_last() {
        let mut output = output_fixture("n-ans", vec![], Value::Null);
        let answer = output.answer.as_mut().expect("answer present");
        answer.superseded = Some(Some(vec![
            Superseded {
                text: "newer stale text".to_string(),
                source_ts: Some("2026-08-03T08:00:00.000Z".to_string()),
            },
            Superseded {
                text: "older stale text".to_string(),
                source_ts: Some("2026-08-01T08:00:00.000Z".to_string()),
            },
        ]));
        let (units, notes) = empty_layers();

        let rows = from_output(&output, &units, &notes);

        assert_eq!(rows[0].trail.len(), 3);
        assert_eq!(rows[0].trail[0].tag, "true now");
        assert_eq!(rows[0].trail[0].at, "2026-08-04");
        assert_eq!(
            rows[0].trail[0].text,
            "the   clipboard ring survives tray restarts"
        );
        assert!(!rows[0].trail[0].struck);
        assert_eq!(rows[0].trail[1].tag, "superseded");
        assert_eq!(rows[0].trail[1].at, "2026-08-03");
        assert_eq!(rows[0].trail[1].text, "newer stale text");
        assert!(rows[0].trail[1].struck);
        assert_eq!(rows[0].trail[2].tag, "superseded");
        assert_eq!(rows[0].trail[2].at, "2026-08-01");
        assert_eq!(rows[0].trail[2].text, "older stale text");
        assert!(rows[0].trail[2].struck);
    }

    #[test]
    fn row_without_history_carries_exactly_one_trail_entry() {
        let output = output_fixture("n-ans", vec![], Value::Null);
        let (units, notes) = empty_layers();

        let rows = from_output(&output, &units, &notes);

        assert_eq!(rows[0].trail.len(), 1);
        assert_eq!(rows[0].trail[0].tag, "true now");
        assert_eq!(rows[0].trail[0].at, "2026-08-04");
        assert_eq!(
            rows[0].trail[0].text,
            "the   clipboard ring survives tray restarts"
        );
        assert!(!rows[0].trail[0].struck);
    }

    #[test]
    fn trail_is_absent_from_serialised_json_when_empty() {
        let row = FlowRow {
            title: "row title".to_string(),
            subtitle: None,
            copy: "row copy".to_string(),
            key: "k-1".to_string(),
            kind: "answer".to_string(),
            trail: vec![],
            detail: vec![],
        };

        let value = serde_json::to_value(&row).expect("row serialises");
        assert!(value.get("trail").is_none());

        let parsed: FlowRow = serde_json::from_value(value).expect("row without trail parses");
        assert!(parsed.trail.is_empty());
    }

    #[test]
    fn detail_is_absent_from_serialised_json_when_empty() {
        let row = FlowRow {
            title: "row title".to_string(),
            subtitle: None,
            copy: "row copy".to_string(),
            key: "k-1".to_string(),
            kind: "answer".to_string(),
            trail: vec![],
            detail: vec![],
        };

        let value = serde_json::to_value(&row).expect("row serialises");
        assert!(value.get("detail").is_none());

        let parsed: FlowRow = serde_json::from_value(value).expect("row without detail parses");
        assert!(parsed.detail.is_empty());
    }

    #[test]
    fn answer_row_detail_carries_verdict_confidence_key_in_order_and_omits_absent_margin() {
        let mut output = output_fixture("n-ans", vec![], Value::Null);
        let answer = output.answer.as_mut().expect("answer present");
        answer.margin = None;
        answer.session = Some("s-1".to_string());
        let (units, notes) = empty_layers();

        let rows = from_output(&output, &units, &notes);

        let labels: Vec<&str> = rows[0].detail.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "verdict",
                "confidence",
                "layer",
                "class",
                "source",
                "when",
                "score",
                "session",
                "key"
            ]
        );
        assert_eq!(rows[0].detail[0].value, "answered");
        assert_eq!(rows[0].detail[1].value, "high");
        assert_eq!(rows[0].detail[8].value, "n-ans");
        assert_eq!(
            rows[0]
                .detail
                .iter()
                .find(|f| f.label == "when")
                .map(|f| f.value.as_str()),
            Some("2026-08-04T08:00:00.000Z")
        );
    }

    #[test]
    fn capture_row_detail_carries_kind_when_key_and_omits_absent_cwd() {
        let output = output_fixture(
            "n-ans",
            vec![],
            json!([
                {
                    "key": "c-1",
                    "score": 12.0,
                    "kind": "capture",
                    "text": "captured fact text",
                    "ts": "2026-08-02T12:00:00.000Z",
                    "snippet": "captured fact text"
                }
            ]),
        );
        let empty_units = UnitsLayer {
            run: "live".to_string(),
            path: PathBuf::from("units.jsonl"),
            items: vec![],
        };
        let empty_notes = NotesLayer {
            run: None,
            items: vec![],
        };

        let rows = from_output(&output, &empty_units, &empty_notes);

        let labels: Vec<&str> = rows[1].detail.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, vec!["kind", "when", "score", "key"]);
        assert_eq!(rows[1].detail[0].value, "capture");
        assert_eq!(rows[1].detail[1].value, "2026-08-02T12:00:00.000Z");
        assert_eq!(rows[1].detail[2].value, "12.00");
        assert_eq!(rows[1].detail[3].value, "c-1");
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
        let output = output_fixture("n-ans", recalled, Value::Null);
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

    #[test]
    fn recalled_assistant_unit_resolves_to_an_assistant_row() {
        let output = output_fixture(
            "n-ans",
            vec![json!({
                "key": "a-1",
                "cls": "assistant",
                "score": 6.0,
                "source_ts": "2026-08-02T12:00:00.000Z",
                "layer": "unit"
            })],
            json!([
                {
                    "key": "a-1",
                    "score": 6.0,
                    "kind": "assistant",
                    "text": "assistant reply body",
                    "ts": "2026-08-02T12:00:00.000Z",
                    "snippet": "assistant reply body"
                }
            ]),
        );
        let units = UnitsLayer {
            run: "live".to_string(),
            path: PathBuf::from("units.jsonl"),
            items: vec![unit("a-1", "assistant", "assistant reply body")],
        };
        let notes = NotesLayer {
            run: None,
            items: vec![],
        };

        let rows = from_output(&output, &units, &notes);

        assert_eq!(rows[0].kind, "answer");
        assert_eq!(rows[1].kind, "assistant");
        assert_eq!(rows[1].key, "a-1");
        assert_eq!(rows[1].subtitle.as_deref(), Some("assistant 2026-08-02"));
        assert_eq!(rows[1].copy, "assistant reply body");
        assert_eq!(rows[1].trail[0].tag, "assistant");
    }
}
