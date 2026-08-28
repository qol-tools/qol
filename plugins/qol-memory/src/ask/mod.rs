use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::aliases::AliasMap;
use crate::retrieval::cache;
use crate::retrieval::{bm25_ranks, build_index, snippet, DocRef};
use crate::retrieval_log::{self, Exclusion, RetrievalEvent};
use crate::skills::{self, Freshness, Served, SkillsIndex};
use crate::store::{dedupe_user_units, is_boilerplate_unit, NotesLayer, Store, Unit, UnitsLayer};
use crate::text;

pub mod rows;

const SNIPPET_WINDOW: usize = 240;
const SKILL_CAP: usize = 2048;
const TOP_NOTE_LIMIT: usize = 5;

#[derive(Debug)]
pub struct AskRequest {
    pub query: String,
    pub k: usize,
    pub brief: bool,
    pub exclude_session: Option<String>,
}

#[derive(Debug)]
pub struct LogOptions {
    pub source: String,
    pub cwd: Option<String>,
    pub fact: Option<String>,
    pub no_log: bool,
}

fn stopword_set() -> &'static HashSet<&'static str> {
    static STOPWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    STOPWORDS.get_or_init(|| {
        HashSet::from([
            "what", "when", "where", "which", "who", "how", "do", "does", "did", "is", "are",
            "the", "a", "an", "to", "for", "of", "in", "on", "with", "and", "or", "me", "you",
            "my", "we", "i", "it", "have", "has", "be", "been", "was", "were", "many", "much",
            "exist", "really", "want", "should", "could", "would", "can", "work", "fix", "this",
            "that", "these", "those", "there", "about", "get", "make", "use", "tell", "explain",
        ])
    })
}

fn recency_cls() -> &'static HashSet<&'static str> {
    static RECENCY_CLS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    RECENCY_CLS.get_or_init(|| {
        HashSet::from([
            "count",
            "status",
            "version",
            "flag",
            "config",
            "decision",
            "decision-deter",
        ])
    })
}

#[allow(dead_code)]
fn stale_cls() -> &'static HashSet<&'static str> {
    static STALE_CLS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    STALE_CLS.get_or_init(|| HashSet::from(["count", "status", "version"]))
}

fn curated_kinds() -> &'static HashSet<&'static str> {
    static CURATED_KINDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    CURATED_KINDS.get_or_init(|| HashSet::from(["artifact", "decision", "decision-deter"]))
}

fn kind_rank(kind: Option<&str>) -> i64 {
    match kind {
        Some("decision-deter") => 3,
        Some("artifact") => 2,
        Some("decision") => 1,
        _ => 0,
    }
}

fn family_digit_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("[0-9]+").expect("valid regex"))
}

fn family_tail_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("\\(.*\\)$").expect("valid regex"))
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Gates {
    #[serde(rename = "NO_MEMORY_COV")]
    pub no_memory_cov: f64,
    #[serde(rename = "FLOOR")]
    pub floor: f64,
    #[serde(rename = "NOTE_COV")]
    pub note_cov: f64,
    #[serde(rename = "NOTE_SCORE")]
    pub note_score: f64,
    #[serde(rename = "UNIT_COV")]
    pub unit_cov: f64,
    #[serde(rename = "UNIT_SCORE")]
    pub unit_score: f64,
    #[serde(rename = "UNIT_MARGIN")]
    pub unit_margin: f64,
    #[serde(rename = "HIGH_MARGIN")]
    pub high_margin: f64,
}

impl Gates {
    pub const DEFAULTS: Gates = Gates {
        no_memory_cov: 0.5,
        floor: 6.0,
        note_cov: 0.5,
        note_score: 6.0,
        unit_cov: 1.0,
        unit_score: 8.0,
        unit_margin: 1.5,
        high_margin: 1.8,
    };

    pub fn from_env() -> Gates {
        Gates::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Gates {
        Gates {
            no_memory_cov: lookup_gate(&lookup, "MEM_NO_COV", Gates::DEFAULTS.no_memory_cov),
            floor: lookup_gate(&lookup, "MEM_FLOOR", Gates::DEFAULTS.floor),
            note_cov: lookup_gate(&lookup, "MEM_NOTE_COV", Gates::DEFAULTS.note_cov),
            note_score: lookup_gate(&lookup, "MEM_NOTE_SCORE", Gates::DEFAULTS.note_score),
            unit_cov: lookup_gate(&lookup, "MEM_UNIT_COV", Gates::DEFAULTS.unit_cov),
            unit_score: lookup_gate(&lookup, "MEM_UNIT_SCORE", Gates::DEFAULTS.unit_score),
            unit_margin: lookup_gate(&lookup, "MEM_UNIT_MARGIN", Gates::DEFAULTS.unit_margin),
            high_margin: lookup_gate(&lookup, "MEM_HIGH_MARGIN", Gates::DEFAULTS.high_margin),
        }
    }

    pub fn is_default(&self) -> bool {
        self.no_memory_cov == Gates::DEFAULTS.no_memory_cov
            && self.floor == Gates::DEFAULTS.floor
            && self.note_cov == Gates::DEFAULTS.note_cov
            && self.note_score == Gates::DEFAULTS.note_score
            && self.unit_cov == Gates::DEFAULTS.unit_cov
            && self.unit_score == Gates::DEFAULTS.unit_score
            && self.unit_margin == Gates::DEFAULTS.unit_margin
            && self.high_margin == Gates::DEFAULTS.high_margin
    }
}

fn lookup_gate(lookup: &impl Fn(&str) -> Option<String>, name: &str, default: f64) -> f64 {
    lookup(name)
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(default)
}

#[derive(Clone, PartialEq)]
struct UnitHit {
    key: String,
    score: f64,
    kind: String,
    source: Option<String>,
    session: Option<String>,
    cwd: Option<String>,
    ts: Option<String>,
    text: String,
}

#[derive(Clone, PartialEq)]
struct NoteHit {
    key: String,
    cls: String,
    text: String,
    source_key: Option<String>,
    source_ts: Option<String>,
    source_kind: Option<String>,
    score: f64,
}

impl NoteHit {
    fn family_key(&self) -> String {
        let head = self.text.split(" | ").next().unwrap_or_default();
        let hashed = family_digit_regex().replace_all(head, "#").to_lowercase();
        let stripped = hashed
            .strip_suffix(" in the corpus")
            .unwrap_or(&hashed)
            .to_string();
        let cut = family_tail_regex().replace_all(&stripped, "").to_string();
        let trimmed = cut.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        format!("{}:{}", self.cls, text::utf16_slice(trimmed, 0, 60))
    }
}

fn distinct_score(qt: &[String], text: &str) -> (usize, usize) {
    let lower = text.to_lowercase();
    let matched = qt.iter().filter(|t| lower.contains(t.as_str())).count();
    (matched, qt.len())
}

fn phrased_coverage(qt: &[String], note_text: Option<&str>) -> f64 {
    let Some(text) = note_text else {
        return 0.0;
    };
    let (matched, total) = distinct_score(qt, text);
    if total == 0 {
        return 0.0;
    }
    matched as f64 / total as f64
}

fn weighted_note_cov(qt: &[String], note_text: &str, idf: &HashMap<String, f64>) -> f64 {
    let lower = note_text.to_lowercase();
    let mut num = 0.0;
    let mut den = 0.0;
    for t in qt {
        let w = idf.get(t).copied().unwrap_or(0.0);
        den += w;
        if lower.contains(t.as_str()) {
            num += w;
        }
    }
    if den == 0.0 {
        0.0
    } else {
        num / den
    }
}

fn fixed2_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let sign = value.is_sign_negative();
    let bits = value.abs().to_bits();
    let exp_field = ((bits >> 52) & 0x7ff) as i64;
    let frac_bits = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if exp_field == 0 {
        (frac_bits, -1074i64)
    } else {
        (frac_bits | (1u64 << 52), exp_field - 1075)
    };
    let base = (mantissa as u128) * 100;
    let n = if exponent >= 0 {
        match u128::checked_shl(base, exponent as u32) {
            Some(shifted) => shifted,
            None => base << 67,
        }
    } else {
        let shift = (-exponent) as u32;
        if shift > 110 {
            0
        } else {
            let doubled = base.saturating_mul(2);
            let divisor = 1u128 << shift;
            (doubled + divisor) >> (shift + 1)
        }
    };
    let rendered = format!("{}.{:02}", n / 100, n % 100);
    if sign && n > 0 {
        format!("-{rendered}")
    } else {
        rendered
    }
}

pub fn run(store: &Store, aliases: &AliasMap, req: &AskRequest) -> Result<AskOutput> {
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    run_with_layers(store, aliases, req, &units, &notes)
}

pub fn run_with_layers(
    store: &Store,
    aliases: &AliasMap,
    req: &AskRequest,
    units: &UnitsLayer,
    notes_layer: &NotesLayer,
) -> Result<AskOutput> {
    let user_units_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| crate::store::in_answer_pool(&unit.kind))
        .cloned()
        .collect();
    let user_units = dedupe_user_units(&user_units_input);

    let exclude_session: Option<String> = req
        .exclude_session
        .clone()
        .filter(|session| !session.is_empty());

    let mut qtokens0 = text::tokens(&req.query);
    qtokens0.retain(|token| !stopword_set().contains(token.as_str()));
    let qtokens = crate::aliases::expand_tokens(&qtokens0, aliases);

    let answer_pool: Vec<Unit> = user_units
        .iter()
        .filter(|unit| {
            !is_boilerplate_unit(unit)
                && exclude_session
                    .as_deref()
                    .is_none_or(|skip| unit.session.as_deref() != Some(skip))
        })
        .cloned()
        .collect();

    let answer_layer = exclude_session.as_deref().map_or_else(
        || "pool".to_string(),
        |session| format!("pool-x-{}", text::utf16_slice(session, 0, 8)),
    );
    let answer_refs = doc_refs(&answer_pool);
    let answer_idx =
        cache::build_or_load(store.root(), &answer_layer, &answer_refs, Some(&units.path));
    let units_query =
        crate::aliases::expand_tokens_keep(&text::tokens(&req.query), aliases).join(" ");
    let answer_ranked: Vec<UnitHit> = bm25_ranks(&units_query, &answer_idx, req.k)
        .into_iter()
        .filter_map(|ranked| {
            answer_pool
                .iter()
                .find(|unit| unit.key == ranked.key)
                .map(|unit| UnitHit {
                    key: ranked.key,
                    score: ranked.score,
                    kind: unit.kind.clone(),
                    source: unit.source.clone(),
                    session: unit.session.clone(),
                    cwd: unit.cwd.clone(),
                    ts: unit.ts.clone(),
                    text: unit.text.clone(),
                })
        })
        .collect();

    let user_refs = doc_refs(&user_units);
    let all_idx = cache::build_or_load(store.root(), "user", &user_refs, Some(&units.path));
    let ranked_all: Vec<UnitHit> = bm25_ranks(&units_query, &all_idx, req.k)
        .into_iter()
        .filter_map(|ranked| {
            user_units
                .iter()
                .find(|unit| unit.key == ranked.key)
                .map(|unit| UnitHit {
                    key: ranked.key,
                    score: ranked.score,
                    kind: unit.kind.clone(),
                    source: unit.source.clone(),
                    session: unit.session.clone(),
                    cwd: unit.cwd.clone(),
                    ts: unit.ts.clone(),
                    text: unit.text.clone(),
                })
        })
        .collect();
    let top_units: Vec<UnitOut> = ranked_all
        .iter()
        .map(|hit| UnitOut {
            key: hit.key.clone(),
            score: hit.score,
            kind: hit.kind.clone(),
            text: hit.text.clone(),
            source: hit.source.clone(),
            session: hit.session.clone(),
            cwd: hit.cwd.clone(),
            ts: hit.ts.clone(),
            snippet: snippet(&hit.text, &qtokens, SNIPPET_WINDOW),
        })
        .collect();

    let notes: Vec<crate::store::Note> = notes_layer.items.clone();
    let notes_refs = notes_refs(&notes);
    let notes_idx = if notes.is_empty() {
        None
    } else {
        Some(cache::build_or_load(
            store.root(),
            "notes",
            &notes_refs,
            None,
        ))
    };
    let notes_query = qtokens.join(" ");
    let top_notes: Vec<NoteHit> = match &notes_idx {
        Some(idx) => bm25_ranks(&notes_query, idx, TOP_NOTE_LIMIT)
            .into_iter()
            .filter_map(|ranked| {
                notes
                    .iter()
                    .find(|note| note.key == ranked.key)
                    .map(|note| NoteHit {
                        key: note.key.clone(),
                        cls: note.cls.clone(),
                        text: note.text.clone(),
                        source_key: note.source_key.clone(),
                        source_ts: note.source_ts.clone(),
                        source_kind: note.source_kind.clone(),
                        score: ranked.score,
                    })
            })
            .collect(),
        None => Vec::new(),
    };

    let skills_out = build_skills_out(store, &req.query, req.brief)?;

    let note_top: Option<&NoteHit> = top_notes.first();
    let unit_top: Option<UnitHit> = answer_ranked.first().cloned();
    let unit_major: Option<&UnitHit> = answer_ranked.get(1);
    let raw_unit_margin = match (&unit_top, unit_major) {
        (Some(top), Some(major)) => top.score / major.score,
        (Some(_), None) => f64::INFINITY,
        _ => 0.0,
    };

    let has_multi_intent = top_notes.len() >= 2
        && top_notes
            .get(1)
            .map(|second| {
                distinct_score(&qtokens, &second.text).0 >= 2
                    && second.family_key() != top_notes[0].family_key()
            })
            .unwrap_or(false);

    let mut note_resolved: Option<NoteHit> = top_notes.first().cloned();
    let mut note_superseded: Option<Vec<NoteHit>> = None;
    if let Some(resolved) = &note_resolved {
        if recency_cls().contains(resolved.cls.as_str()) {
            let same_family: Vec<NoteHit> = top_notes
                .iter()
                .filter(|hit| {
                    hit.family_key() == resolved.family_key() && hit.source_ts != resolved.source_ts
                })
                .cloned()
                .collect();
            if !same_family.is_empty() {
                let mut by_ts = Vec::with_capacity(same_family.len() + 1);
                by_ts.push(resolved.clone());
                by_ts.extend(same_family);
                by_ts.sort_by(|a, b| {
                    text::parse_iso_millis(b.source_ts.as_deref())
                        .cmp(&text::parse_iso_millis(a.source_ts.as_deref()))
                });
                let newest = by_ts.remove(0);
                note_superseded = Some(
                    by_ts
                        .into_iter()
                        .filter(|hit| hit.key != newest.key)
                        .collect(),
                );
                note_resolved = Some(newest);
            }
        }
    }

    let gates = Gates::from_env();
    let note_cov = phrased_coverage(&qtokens, note_resolved.as_ref().map(|n| n.text.as_str()));
    let unit_cov = unit_top.as_ref().map_or(0.0, |top| {
        distinct_score(&qtokens, &top.text).0 as f64 / std::cmp::max(1, qtokens.len()) as f64
    });

    if let (Some(resolved), Some(idx)) = (&note_resolved, &notes_idx) {
        if weighted_note_cov(&qtokens, &resolved.text, &idx.idf) < gates.note_cov {
            let alt = top_notes.iter().find(|hit| {
                hit.key != resolved.key
                    && weighted_note_cov(&qtokens, &hit.text, &idx.idf) >= gates.note_cov
                    && hit.score >= gates.note_score
            });
            if let Some(hit) = alt {
                note_resolved = Some(hit.clone());
                note_superseded = None;
            }
        }
    }

    let next_family_note = note_resolved.as_ref().and_then(|resolved| {
        top_notes
            .iter()
            .find(|hit| hit.key != resolved.key && hit.family_key() != resolved.family_key())
    });
    let mut note_decisive = true;
    if let (Some(resolved), Some(next_family)) = (&note_resolved, next_family_note) {
        if resolved.score == next_family.score {
            let tied: Vec<NoteHit> = top_notes
                .iter()
                .filter(|hit| hit.score == resolved.score)
                .cloned()
                .collect();
            let newest_ts = tied
                .iter()
                .map(|hit| text::parse_iso_millis(hit.source_ts.as_deref()))
                .max()
                .unwrap_or(0);
            let newest_tied: Vec<NoteHit> = tied
                .iter()
                .filter(|hit| text::parse_iso_millis(hit.source_ts.as_deref()) == newest_ts)
                .cloned()
                .collect();
            let best_kind = newest_tied
                .iter()
                .map(|hit| kind_rank(hit.source_kind.as_deref()))
                .max()
                .unwrap_or(0);
            let kind_tied: Vec<NoteHit> = newest_tied
                .into_iter()
                .filter(|hit| kind_rank(hit.source_kind.as_deref()) == best_kind)
                .collect();
            if kind_tied.len() == 1 {
                note_resolved = Some(kind_tied.into_iter().next().expect("single element"));
            } else {
                note_decisive = false;
            }
        }
    }

    let note_cov_r = match (&note_resolved, &notes_idx) {
        (Some(resolved), Some(idx)) => weighted_note_cov(&qtokens, &resolved.text, &idx.idf),
        _ => 0.0,
    };
    let max_cov = f64::max(note_cov_r, unit_cov);
    let fam_relevant = note_resolved.as_ref().is_some_and(|resolved| {
        !qtokens.is_empty() && {
            let lower = resolved.text.to_lowercase();
            let hits = qtokens
                .iter()
                .filter(|t| lower.contains(t.as_str()))
                .count();
            hits >= std::cmp::max(2, qtokens.len().div_ceil(2))
        }
    });
    let has_recency_answer = note_superseded
        .as_ref()
        .map(|list| !list.is_empty())
        .unwrap_or(false)
        && fam_relevant
        && note_resolved
            .as_ref()
            .map(|resolved| resolved.score >= gates.note_score)
            .unwrap_or(false);

    let mut verdict = "no-memory".to_string();
    let mut confidence = "none".to_string();
    let mut answer: Option<Answer> = None;
    let mut related: Vec<Related> = Vec::new();

    let below_floor = |score: Option<f64>| score.unwrap_or(0.0) < gates.floor;

    let reason;

    if (max_cov < gates.no_memory_cov && !has_recency_answer)
        || (below_floor(note_top.map(|note| note.score))
            && below_floor(unit_top.as_ref().map(|top| top.score))
            && !has_recency_answer)
    {
        reason = format!(
            "no memory above the answer threshold (max_cov={}, floor={})",
            fixed2_string(max_cov),
            gates.floor
        );
    } else {
        let note_winner = note_resolved.as_ref().is_some_and(|resolved| {
            note_decisive
                && curated_kinds().contains(resolved.source_kind.as_deref().unwrap_or(""))
                && (note_cov_r >= gates.note_cov
                    || (fam_relevant && note_superseded.as_ref().is_some_and(|s| !s.is_empty())))
                && resolved.score >= gates.note_score
        });
        let unit_winner = unit_top.as_ref().is_some_and(|top| {
            unit_cov >= gates.unit_cov
                && top.score >= gates.unit_score
                && !is_boilerplate_unit_user(top)
                && raw_unit_margin >= gates.unit_margin
        });

        if note_winner {
            let resolved = note_resolved
                .as_ref()
                .expect("winner keeps a resolved note");
            let next_family = top_notes
                .iter()
                .find(|hit| hit.key != resolved.key && hit.family_key() != resolved.family_key());
            let margin = next_family.map_or(f64::INFINITY, |hit| resolved.score / hit.score);
            let high = margin >= gates.high_margin
                && !note_superseded.as_ref().is_some_and(|s| !s.is_empty());
            let rounded_margin = text::to_fixed2(margin.min(99.0));
            let superseded_for_output = note_superseded.as_ref().filter(|list| !list.is_empty());
            answer = Some(Answer {
                text: resolved.text.clone(),
                layer: "note".to_string(),
                key: resolved.key.clone(),
                cls: Some(resolved.cls.clone()),
                source_kind: resolved.source_kind.clone().unwrap_or_default(),
                source_ts: resolved.source_ts.clone(),
                session: None,
                score: text::to_fixed2(resolved.score),
                margin: Some(rounded_margin),
                superseded: Some(superseded_for_output.map(|list| {
                    list.iter()
                        .map(|hit| Superseded {
                            text: hit.text.clone(),
                            source_ts: hit.source_ts.clone(),
                        })
                        .collect()
                })),
            });
            verdict = "answered".to_string();
            confidence = if high { "high" } else { "medium" }.to_string();
            reason = format!(
                "notes layer {} answer, margin {}x{}",
                resolved.cls,
                text::to_fixed2(margin.min(99.0)),
                if superseded_for_output.is_some() {
                    ", recency-resolved (superseded a stale fact)"
                } else {
                    ""
                }
            );
            if has_multi_intent {
                let second = top_notes.iter().find(|hit| {
                    hit.key != resolved.key
                        && hit.family_key() != resolved.family_key()
                        && distinct_score(&qtokens, &hit.text).0 >= 2
                });
                if let Some(hit) = second {
                    related.push(Related {
                        text: hit.text.clone(),
                        cls: hit.cls.clone(),
                        source_ts: hit.source_ts.clone(),
                    });
                }
            }
        } else if unit_winner {
            let top = unit_top.as_ref().expect("unit winner keeps a top unit");
            answer = Some(Answer {
                text: snippet(&top.text, &qtokens, SNIPPET_WINDOW),
                layer: "unit".to_string(),
                key: top.key.clone(),
                cls: None,
                source_kind: top.kind.clone(),
                source_ts: top.ts.clone(),
                session: top.session.clone(),
                score: text::to_fixed2(top.score),
                margin: None,
                superseded: None,
            });
            verdict = "answered".to_string();
            confidence = "medium".to_string();
            reason = if top.kind == "capture" {
                "units layer answer (agent capture), confidence capped medium".to_string()
            } else {
                "units layer answer (user's own words), confidence capped medium".to_string()
            };
        } else {
            verdict = "candidates".to_string();
            confidence = "low".to_string();
            reason = format!(
                "no decisive answer: note_cov={} unit_cov={}",
                fixed2_string(note_cov_r),
                fixed2_string(unit_cov)
            );
        }
    }

    let live_units = units.run == "live";
    let stale_layer = !live_units
        && notes_layer
            .run
            .as_deref()
            .map(|notes_run| text::run_dir_millis(notes_run) < text::run_dir_millis(&units.run))
            .unwrap_or(false);

    let recalled: Vec<Recalled> = top_notes
        .iter()
        .map(|note| Recalled {
            key: note.key.clone(),
            cls: note.cls.clone(),
            score: text::to_fixed2(note.score),
            source_kind: note.source_kind.clone(),
            source_ts: note.source_ts.clone(),
        })
        .collect();

    let out_notes: Vec<NoteOut> = top_notes
        .iter()
        .map(|note| {
            if req.brief {
                NoteOut {
                    key: note.key.clone(),
                    cls: note.cls.clone(),
                    text: if verdict == "answered" {
                        Some(note.text.clone())
                    } else {
                        None
                    },
                    source_key: None,
                    source_ts: None,
                    source_kind: None,
                    score: text::to_fixed2(note.score),
                }
            } else {
                NoteOut {
                    key: note.key.clone(),
                    cls: note.cls.clone(),
                    text: Some(note.text.clone()),
                    source_key: note.source_key.clone(),
                    source_ts: note.source_ts.clone(),
                    source_kind: note.source_kind.clone(),
                    score: note.score,
                }
            }
        })
        .collect();

    Ok(AskOutput {
        query: req.query.clone(),
        verdict,
        confidence,
        reason,
        gates,
        non_default_gates: !gates.is_default(),
        answer,
        recalled,
        related,
        signals: Signals {
            top_note_score: note_top.map(|note| text::to_fixed2(note.score)),
            top_unit_score: unit_top.as_ref().map(|top| text::to_fixed2(top.score)),
            unit_margin: unit_top.as_ref().map(|_| {
                let usable = if raw_unit_margin.is_nan() {
                    0.0
                } else {
                    raw_unit_margin
                };
                text::to_fixed2(usable)
            }),
            note_token_coverage: text::to_fixed2(note_cov),
            unit_token_coverage: text::to_fixed2(unit_cov),
            max_token_coverage: text::to_fixed2(f64::max(note_cov, unit_cov)),
            notes_run_ts: notes_layer.run.clone(),
            snapshot_run_ts: units.run.clone(),
            live_units,
            stale_layer,
            recency_resolved: note_superseded.as_ref().map(|list| !list.is_empty()),
        },
        counts: Counts {
            units: user_units.len(),
            notes: notes.len(),
        },
        skills: skills_out,
        units: if req.brief { None } else { Some(top_units) },
        notes: out_notes,
    })
}

fn is_boilerplate_unit_user(hit: &UnitHit) -> bool {
    crate::store::BOILERPLATE_MARKERS
        .iter()
        .any(|marker| hit.text.contains(marker))
}

pub(crate) fn doc_refs(items: &[Unit]) -> Vec<DocRef<'_>> {
    items
        .iter()
        .map(|item| DocRef {
            key: item.key.as_str(),
            text: item.text.as_str(),
        })
        .collect()
}

pub(crate) fn notes_refs(items: &[crate::store::Note]) -> Vec<DocRef<'_>> {
    items
        .iter()
        .map(|item| DocRef {
            key: item.key.as_str(),
            text: item.text.as_str(),
        })
        .collect()
}

fn build_skills_out(store: &Store, query: &str, brief: bool) -> Result<SkillsOut> {
    let index: Option<SkillsIndex> = skills::load_index(&store.skills_index_path());
    let env_root: Option<PathBuf> = std::env::var_os("QOL_MEMORY_SKILLS_ROOT")
        .filter(|raw| !raw.is_empty())
        .map(PathBuf::from);
    let skills_root: Option<PathBuf> = index
        .as_ref()
        .and_then(|index| index.root.as_ref())
        .map(PathBuf::from)
        .or(env_root);

    let freshness = match (&index, &skills_root) {
        (None, _) => Freshness::NotIndexed,
        (Some(index), None) => {
            if index.walked_at.is_none() {
                Freshness::NotIndexed
            } else {
                Freshness::Unavailable
            }
        }
        (Some(index), Some(root)) => skills::probe_fresh(index, root),
    };

    let mut hits_full: Vec<SkillHitFull> = Vec::new();
    if let (Some(index), Some(root)) = (&index, &skills_root) {
        if !index.skills.is_empty() {
            let meta_texts: Vec<String> = index.skills.iter().map(skills::build_meta_doc).collect();
            let meta_docs: Vec<DocRef> = index
                .skills
                .iter()
                .zip(meta_texts.iter())
                .map(|(skill, text)| DocRef {
                    key: skill.id.as_str(),
                    text,
                })
                .collect();
            let skills_idx = build_index(&meta_docs);
            let mut qt = skills::pool_tokens(query);
            qt.retain(|token| !stopword_set().contains(token.as_str()));
            let ranked = bm25_ranks(query, &skills_idx, TOP_NOTE_LIMIT);
            let mut seen = HashSet::new();
            for hit in ranked {
                if seen.contains(&hit.key) {
                    continue;
                }
                seen.insert(hit.key.clone());
                let Some(skill_meta) = index.skills.iter().find(|skill| skill.id == hit.key) else {
                    continue;
                };
                let best = skills::best_section(skill_meta, root, &qt, &skills_idx.idf, SKILL_CAP);
                let hint = best.as_ref().map(|best| best.h.as_str());
                let served = skills::serve_section(skill_meta, root, hint, SKILL_CAP);
                let skill_hit = SkillHitFull {
                    id: skill_meta.id.clone(),
                    name: skill_meta.name.clone(),
                    score: text::to_fixed2(hit.score),
                    section: match &served {
                        Served::Ok { section, .. } => Some(section.clone()),
                        Served::Failed { .. } => best.map(|best| best.h),
                    },
                    content: match &served {
                        Served::Ok { content, .. } => Some(content.clone()),
                        Served::Failed { .. } => None,
                    },
                    truncated: matches!(
                        served,
                        Served::Ok {
                            truncated: true,
                            ..
                        }
                    ),
                    hash_match: matches!(
                        served,
                        Served::Ok {
                            hash_match: true,
                            ..
                        }
                    ),
                    status: match &served {
                        Served::Ok { .. } => "served".to_string(),
                        Served::Failed { reason } => reason.clone(),
                    },
                    head: index.repo.as_ref().and_then(|repo| repo.head.clone()),
                    dirty: index.repo.as_ref().and_then(|repo| repo.dirty),
                };
                hits_full.push(skill_hit);
            }
        }
    }

    let status_string = match (&index, &freshness) {
        (_, Freshness::Fresh) => "served".to_string(),
        (None, _) => "not-indexed".to_string(),
        (Some(_), other) => other.as_str().to_string(),
    };
    Ok(SkillsOut {
        status: status_string,
        root: match &index {
            Some(index) => index.root.clone(),
            None => skills_root.as_ref().map(|root| root.display().to_string()),
        },
        head: index
            .as_ref()
            .and_then(|index| index.repo.as_ref())
            .and_then(|repo| repo.head.clone()),
        dirty: index
            .as_ref()
            .and_then(|index| index.repo.as_ref())
            .and_then(|repo| repo.dirty),
        hits: hits_full
            .into_iter()
            .map(|hit| hit.project(brief))
            .collect(),
    })
}

struct SkillHitFull {
    id: String,
    name: String,
    score: f64,
    section: Option<String>,
    content: Option<String>,
    truncated: bool,
    hash_match: bool,
    status: String,
    head: Option<String>,
    dirty: Option<bool>,
}

impl SkillHitFull {
    fn project(self, brief: bool) -> SkillHit {
        if brief {
            SkillHit {
                id: self.id,
                name: None,
                score: self.score,
                section: self.section,
                content: None,
                truncated: None,
                hash_match: None,
                status: self.status,
                head: None,
                dirty: None,
            }
        } else {
            SkillHit {
                id: self.id,
                name: Some(self.name),
                score: self.score,
                section: self.section,
                content: Some(self.content),
                truncated: Some(self.truncated),
                hash_match: Some(self.hash_match),
                status: self.status,
                head: self.head,
                dirty: self.dirty,
            }
        }
    }
}

pub fn run_and_log(
    store: &Store,
    aliases: &AliasMap,
    req: &AskRequest,
    log: &LogOptions,
) -> Result<AskOutput> {
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    run_and_log_with_layers(store, aliases, req, log, &units, &notes)
}

pub fn run_and_log_with_layers(
    store: &Store,
    aliases: &AliasMap,
    req: &AskRequest,
    log: &LogOptions,
    units: &UnitsLayer,
    notes: &NotesLayer,
) -> Result<AskOutput> {
    let started = Instant::now();
    let out = run_with_layers(store, aliases, req, units, notes)?;
    let latency_ms = started.elapsed().as_millis() as u64;

    if !log.no_log {
        let exclude_session: Option<String> = req
            .exclude_session
            .clone()
            .filter(|session| !session.is_empty());
        let event = RetrievalEvent {
            ts: text::now_iso(),
            source: log.source.clone(),
            session: exclude_session.clone(),
            cwd: log.cwd.clone(),
            query: out.query.clone(),
            verdict: out.verdict.clone(),
            confidence: out.confidence.clone(),
            correctness: retrieval_log::correctness_of(
                &out.verdict,
                out.answer.as_ref().map(|answer| answer.text.as_str()),
                log.fact.as_deref(),
                &log.source,
            ),
            latency_ms,
            k: req.k,
            exclusion: Exclusion {
                exclude_session: exclude_session.is_some(),
                non_default_gates: out.non_default_gates,
            },
            gates: serde_json::to_value(out.gates).unwrap_or(Value::Null),
            signals: serde_json::to_value(&out.signals).unwrap_or(Value::Null),
            answer_key: out.answer.as_ref().map(|answer| answer.key.clone()),
            recalled_keys: out
                .recalled
                .iter()
                .map(|recall| recall.key.clone())
                .collect(),
            counts: serde_json::to_value(&out.counts).unwrap_or(Value::Null),
        };
        retrieval_log::append(store.root(), &event);
    }

    Ok(out)
}

pub fn render_text(out: &AskOutput) -> String {
    let mut lines = Vec::new();
    lines.push(format!("verdict: {} ({})", out.verdict, out.confidence));
    lines.push(format!("reason: {}", out.reason));
    if out.verdict == "answered" {
        if let Some(answer) = &out.answer {
            let cls = match &answer.cls {
                Some(cls) => cls.clone(),
                None => "-".to_string(),
            };
            lines.push(format!(
                "answer [{}/{}]: {}",
                answer.layer, cls, answer.text
            ));
        }
    }
    lines.push("recalled:".to_string());
    for recall in &out.recalled {
        lines.push(format!(
            "  {}  {}  {}",
            recall.key, recall.cls, recall.score
        ));
    }
    if !out.skills.hits.is_empty() {
        lines.push("skills:".to_string());
        for hit in &out.skills.hits {
            let section = hit.section.clone().unwrap_or_else(|| "-".to_string());
            lines.push(format!("  {}  {}  {}", hit.id, hit.score, section));
        }
    }
    lines.join("\n")
}

pub fn status(store: &Store) -> Result<Value> {
    let units = store.read_units()?;
    let notes = store.read_notes()?;
    status_with_layers(store, &units, &notes)
}

pub fn status_with_layers(store: &Store, units: &UnitsLayer, notes: &NotesLayer) -> Result<Value> {
    let units_path = store.units_path();
    let units_present = units_path.exists();
    let units_bytes = std::fs::metadata(&units_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let sealed = store.root().join("units.seal.json").exists()
        && store.root().join("units.seal.gz").exists();

    let user_units_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| crate::store::in_answer_pool(&unit.kind))
        .cloned()
        .collect();
    let user_units = dedupe_user_units(&user_units_input);
    let pool_units: Vec<Unit> = user_units
        .iter()
        .filter(|unit| !is_boilerplate_unit(unit))
        .cloned()
        .collect();
    let pool_refs = doc_refs(&pool_units);
    let user_refs = doc_refs(&user_units);
    let note_items: Vec<crate::store::Note> = notes.items.clone();
    let note_refs = notes_refs(&note_items);

    let cache_label = |state: cache::CacheState| match state {
        cache::CacheState::Fresh => "fresh",
        cache::CacheState::Stale => "stale",
        cache::CacheState::Missing => "missing",
    };
    let pool_state = cache_label(cache::cache_state(
        store.root(),
        "pool",
        &pool_refs,
        Some(&units.path),
    ));
    let user_state = cache_label(cache::cache_state(
        store.root(),
        "user",
        &user_refs,
        Some(&units.path),
    ));
    let notes_state = cache_label(cache::cache_state(store.root(), "notes", &note_refs, None));

    let skills_value = match skills::load_index(&store.skills_index_path()) {
        Some(index) => json!({
            "present": true,
            "root": index.root,
            "head": index.repo.as_ref().and_then(|repo| repo.head.clone()),
            "dirty": index.repo.as_ref().and_then(|repo| repo.dirty),
            "walked_at": index.walked_at,
        }),
        None => json!({
            "present": false,
            "root": null,
            "head": null,
            "dirty": null,
            "walked_at": null,
        }),
    };

    let retrievals_bytes = std::fs::metadata(store.retrievals_path())
        .map(|meta| meta.len())
        .unwrap_or(0);
    let last_ts = retrieval_log::last_event_ts(store.root());

    Ok(json!({
        "store": store.root().display().to_string(),
        "exists": store.root().exists(),
        "units_file": {
            "present": units_present,
            "bytes": units_bytes,
            "sealed": sealed,
        },
        "notes_run": notes.run,
        "index_caches": {
            "pool": pool_state,
            "user": user_state,
            "notes": notes_state,
        },
        "skills": skills_value,
        "retrievals": {
            "bytes": retrievals_bytes,
            "last_ts": last_ts,
        },
        "candidates_pending": retrieval_log::count_pending_candidates(store.root()),
    }))
}

#[derive(Serialize, Deserialize)]
pub struct AskOutput {
    pub query: String,
    pub verdict: String,
    pub confidence: String,
    pub reason: String,
    pub gates: Gates,
    pub non_default_gates: bool,
    pub answer: Option<Answer>,
    pub recalled: Vec<Recalled>,
    pub related: Vec<Related>,
    pub signals: Signals,
    pub counts: Counts,
    pub skills: SkillsOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<UnitOut>>,
    pub notes: Vec<NoteOut>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Answer {
    pub text: String,
    pub layer: String,
    pub key: String,
    pub cls: Option<String>,
    pub source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub score: f64,
    pub margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded: Option<Option<Vec<Superseded>>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Superseded {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct Recalled {
    pub key: String,
    pub cls: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct Related {
    pub text: String,
    pub cls: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct Signals {
    pub top_note_score: Option<f64>,
    pub top_unit_score: Option<f64>,
    pub unit_margin: Option<f64>,
    pub note_token_coverage: f64,
    pub unit_token_coverage: f64,
    pub max_token_coverage: f64,
    pub notes_run_ts: Option<String>,
    pub snapshot_run_ts: String,
    pub live_units: bool,
    pub stale_layer: bool,
    pub recency_resolved: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Counts {
    pub units: usize,
    pub notes: usize,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct SkillsOut {
    pub status: String,
    pub root: Option<String>,
    pub head: Option<String>,
    pub dirty: Option<bool>,
    pub hits: Vec<SkillHit>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct SkillHit {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub score: f64,
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_match: Option<bool>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct UnitOut {
    pub key: String,
    pub score: f64,
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    pub snippet: String,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub struct NoteOut {
    pub key: String,
    pub cls: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("qol-memory-ask-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn write_units(root: &Path, body: &str) {
        fs::write(root.join("units.jsonl"), body).expect("write units.jsonl");
    }

    fn write_newest_run_notes(root: &Path, body: &str) {
        let run_dir = root.join("notes").join("2026-08-05T10-00-00-000Z");
        fs::create_dir_all(&run_dir).expect("create notes run dir");
        fs::write(run_dir.join("notes.jsonl"), body).expect("write notes.jsonl");
    }

    fn fixture_units() -> String {
        [
            "{\"key\":\"u-1\",\"session\":\"sess-live-aaa1\",\"kind\":\"user\",\"ts\":\"2026-08-01T09:00:00.000Z\",\"text\":\"the plugin lights daemon crashed after the bluetooth adapter slept\"}",
            "{\"key\":\"u-2\",\"session\":\"sess-live-bbb2\",\"kind\":\"user\",\"ts\":\"2026-08-01T10:00:00.000Z\",\"text\":\"alt tab preview caches window thumbnails between keystrokes\"}",
            "{\"key\":\"u-3\",\"session\":\"sess-live-ccc3\",\"kind\":\"user\",\"ts\":\"2026-08-01T11:00:00.000Z\",\"text\":\"tray icon tooltip shows battery percentage for gamepad controllers\"}",
            "{\"key\":\"u-b\",\"session\":\"sess-live-bbb2\",\"kind\":\"user\",\"ts\":\"2026-08-01T12:00:00.000Z\",\"text\":\"[qol session bridge] continued from a previous conversation\"}",
        ]
        .join("\n")
    }

    fn fixture_notes() -> String {
        let mut rows: Vec<String> = Vec::new();
        rows.push("{\"key\":\"n-count-new\",\"cls\":\"count\",\"source_kind\":\"decision-deter\",\"source_ts\":\"2026-08-06T08:00:00.000Z\",\"text\":\"count 4101 user units in the corpus\"}".to_string());
        rows.push("{\"key\":\"n-count-old\",\"cls\":\"count\",\"source_kind\":\"decision-deter\",\"source_ts\":\"2026-08-02T08:00:00.000Z\",\"text\":\"count 3922 user units in the corpus\"}".to_string());
        rows.push("{\"key\":\"n-decision\",\"cls\":\"decision\",\"source_kind\":\"decision\",\"source_ts\":\"2026-08-04T08:00:00.000Z\",\"text\":\"Decision: the plugin clipboard history ring now persists across tray restarts and survives the sandbox teardown\"}".to_string());
        for i in 0..26usize {
            rows.push(format!(
                "{{\"key\":\"n-fill-{i:02}\",\"cls\":\"observation\",\"source_kind\":\"count\",\"source_ts\":\"2026-08-03T07:{i:02}:00.000Z\",\"text\":\"zxnfrey{i:03} {}\"}}",
                "lorem ".repeat(90).trim_end()
            ));
        }
        rows.join("\n")
    }

    fn build_fixture(root: &Path) {
        write_units(root, &fixture_units());
        write_newest_run_notes(root, &fixture_notes());
    }

    fn run_ask(store: &Store, query: &str, brief: bool) -> AskOutput {
        run(
            store,
            &AliasMap::default(),
            &AskRequest {
                query: query.to_string(),
                k: 5,
                brief,
                exclude_session: None,
            },
        )
        .expect("ask runs")
    }

    fn as_f64(value: &Value) -> f64 {
        value.as_f64().expect("numeric json value")
    }

    #[test]
    fn family_key_replaces_digits_strips_corpus_and_paren_tails() {
        let note = |cls: &str, text: &str| NoteHit {
            key: "k".to_string(),
            cls: cls.to_string(),
            text: text.to_string(),
            source_key: None,
            source_ts: None,
            source_kind: None,
            score: 0.0,
        };
        assert_eq!(
            note("count", "count 3922 user units in the corpus").family_key(),
            "count:count # user units"
        );
        assert_eq!(
            note("count", "count 57 broken symlinks (lib/foo.rs)").family_key(),
            "count:count # broken symlinks"
        );
        assert_eq!(
            note("status", "Sessions overview refreshed in the corpus").family_key(),
            "status:sessions overview refreshed"
        );
        assert_eq!(
            note(
                "flag",
                "flag --strict-mode enabled (nested | separator kept)"
            )
            .family_key(),
            "flag:flag --strict-mode enabled (nested"
        );
    }

    #[test]
    fn distinct_score_counts_duplicate_query_terms_separately() {
        let qt = vec!["fix".to_string(), "fix".to_string(), "plugin".to_string()];
        let (matched, total) = distinct_score(&qt, "Fixed the plugin plugin registry");
        assert_eq!(matched, 3);
        assert_eq!(total, 3);
        let (zero_matched, zero_total) = distinct_score(&qt, "nothing relevant here");
        assert_eq!(zero_matched, 0);
        assert_eq!(zero_total, 3);
    }

    #[test]
    fn weighted_note_cov_uses_idf_weights_and_skips_unknown_terms() {
        let idf = HashMap::from([
            ("common".to_string(), 0.1),
            ("rare".to_string(), 5.0),
            ("absent".to_string(), 3.0),
        ]);
        let qt = vec![
            "common".to_string(),
            "rare".to_string(),
            "absent".to_string(),
        ];
        let cov = weighted_note_cov(&qt, "has Common tokens and the RARE one", &idf);
        assert!((cov - (0.1 + 5.0) / (0.1 + 5.0 + 3.0)).abs() < 1e-12);
        assert_eq!(weighted_note_cov(&[], "anything", &idf), 0.0);
        assert_eq!(
            weighted_note_cov(&["missing".to_string()], "anything", &idf),
            0.0
        );
    }

    #[test]
    fn gates_from_lookup_defaults_when_unparsable_and_overrides_when_parseable() {
        let defaults = Gates::from_lookup(|_| None);
        assert!(defaults.is_default());
        let mixed_values = HashMap::from([
            ("MEM_FLOOR".to_string(), "7.5".to_string()),
            ("MEM_NO_COV".to_string(), "".to_string()),
            ("MEM_UNIT_COV".to_string(), "junk".to_string()),
            ("MEM_HIGH_MARGIN".to_string(), "2.25".to_string()),
        ]);
        let overridden = Gates::from_lookup(|name| mixed_values.get(name).cloned());
        assert!(!overridden.is_default());
        assert_eq!(overridden.floor, 7.5);
        assert_eq!(overridden.no_memory_cov, Gates::DEFAULTS.no_memory_cov);
        assert_eq!(overridden.unit_cov, Gates::DEFAULTS.unit_cov);
        assert_eq!(overridden.high_margin, 2.25);
    }

    #[test]
    fn end_to_end_nonsense_query_reports_no_memory_without_recency_crash() {
        let root = temp_root("nonsense");
        build_fixture(&root);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, "zzzqqqwubblewobble", false);
        assert_eq!(out.verdict, "no-memory");
        assert_eq!(out.confidence, "none");
        assert_eq!(
            out.reason,
            "no memory above the answer threshold (max_cov=0.00, floor=6)"
        );
        assert_eq!(out.answer, None);
        assert_eq!(out.signals.max_token_coverage, 0.0);
        assert_eq!(out.signals.top_note_score, Some(0.0));
        assert_eq!(out.signals.top_unit_score, Some(0.0));
        assert_eq!(out.signals.unit_margin, Some(0.0));
        assert_eq!(out.signals.recency_resolved, Some(true));
        assert_eq!(
            out.counts,
            Counts {
                units: 4,
                notes: 29
            }
        );
        let value = serde_json::to_value(&out).expect("serialize");
        assert_eq!(value["signals"]["stale_layer"], false);
        assert_eq!(value["signals"]["live_units"], true);
        assert_eq!(value["non_default_gates"], false);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn end_to_end_decision_query_answers_from_the_curated_note() {
        let root = temp_root("decision");
        build_fixture(&root);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "does the clipboard history ring survive tray restarts",
            false,
        );
        assert_eq!(out.verdict, "answered");
        assert_eq!(out.confidence, "high");
        let answer = out.answer.as_ref().expect("note answer");
        assert_eq!(answer.layer, "note");
        assert_eq!(answer.key, "n-decision");
        assert_eq!(answer.cls.as_deref(), Some("decision"));
        assert_eq!(as_f64(&serde_json::to_value(answer.score).unwrap()), 14.38);
        assert_eq!(
            as_f64(&serde_json::to_value(answer.margin.unwrap()).expect("margin serializes")),
            99.0
        );
        assert_eq!(answer.superseded, Some(None));
        assert_eq!(out.reason, "notes layer decision answer, margin 99x");
        assert_eq!(out.signals.top_note_score, Some(14.38));
        assert_eq!(out.signals.top_unit_score, Some(0.73));
        assert_eq!(out.signals.unit_margin, Some(1.38));
        assert_eq!(out.signals.note_token_coverage, 0.86);
        assert_eq!(out.signals.recency_resolved, None);
        let value = serde_json::to_value(&out).expect("serialize");
        assert_eq!(value["answer"]["session"], Value::Null);

        let excluded = AskRequest {
            query: "does the clipboard history ring survive tray restarts".to_string(),
            k: 5,
            brief: false,
            exclude_session: Some("sess-live-aaa1".to_string()),
        };
        let excluded_out = run(&store, &AliasMap::default(), &excluded).expect("excluded ask runs");
        assert_eq!(excluded_out.counts.units, 4);
        assert!(root.join("idx-pool-x-sess-liv.json").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn end_to_end_recency_query_resolves_to_the_newer_count_note() {
        let root = temp_root("recency");
        build_fixture(&root);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "how many user units are there in the corpus now",
            false,
        );
        assert_eq!(out.verdict, "answered");
        assert_eq!(out.confidence, "medium");
        let answer = out.answer.as_ref().expect("note answer");
        assert_eq!(answer.key, "n-count-new");
        assert_eq!(
            as_f64(&serde_json::to_value(answer.margin.unwrap()).expect("margin serializes")),
            2.74
        );
        assert_eq!(
            out.reason,
            "notes layer count answer, margin 2.74x, recency-resolved (superseded a stale fact)"
        );
        let superseded = answer
            .superseded
            .as_ref()
            .and_then(|inner| inner.as_ref())
            .expect("superseded list");
        assert_eq!(superseded.len(), 1);
        assert_eq!(superseded[0].text, "count 3922 user units in the corpus");
        assert_eq!(
            superseded[0].source_ts.as_deref(),
            Some("2026-08-02T08:00:00.000Z")
        );
        assert_eq!(out.signals.recency_resolved, Some(true));
        assert_eq!(out.signals.top_note_score, Some(6.56));
        assert_eq!(out.signals.unit_margin, Some(f64::INFINITY));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn end_to_end_empty_notes_dir_does_not_crash_and_yields_non_note_verdict() {
        let root = temp_root("empty-notes");
        write_units(&root, &fixture_units());
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, "zzzqqqwubblewobble", false);
        assert_eq!(out.verdict, "no-memory");
        assert_eq!(out.counts, Counts { units: 4, notes: 0 });
        assert_eq!(out.signals.notes_run_ts, None);
        assert_eq!(out.signals.recency_resolved, None);
        assert_eq!(out.recalled.len(), 0);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn end_to_end_brief_omits_units_and_note_text_unless_answered() {
        let root = temp_root("brief");
        build_fixture(&root);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let brief_unanswered = run_ask(&store, "zzzqqqwubblewobble", true);
        let value = serde_json::to_value(&brief_unanswered).expect("serialize");
        assert!(value.get("units").is_none());
        let notes = value["notes"].as_array().expect("brief notes array");
        assert!(!notes.is_empty());
        for note in notes {
            assert!(note.get("text").is_none(), "brief notes omit text");
            assert!(note.get("source_key").is_none());
            assert_eq!(note.as_object().expect("object").len(), 3);
        }

        let full_unanswered = run_ask(&store, "zzzqqqwubblewobble", false);
        let full_value = serde_json::to_value(&full_unanswered).expect("serialize");
        let full_units = full_value["units"].as_array().expect("full units array");
        assert!(!full_units.is_empty());

        let brief_answered = run_ask(
            &store,
            "does the clipboard history ring survive tray restarts",
            true,
        );
        let brief_answered_value = serde_json::to_value(&brief_answered).expect("serialize");
        assert!(brief_answered_value.get("units").is_none());
        let answered_notes = brief_answered_value["notes"]
            .as_array()
            .expect("brief notes array");
        let decision_entry = answered_notes
            .iter()
            .find(|note| note["key"] == "n-decision")
            .expect("decision note present");
        assert_eq!(
            decision_entry["text"],
            "Decision: the plugin clipboard history ring now persists across tray restarts and survives the sandbox teardown"
        );
        assert_eq!(decision_entry.as_object().expect("object").len(), 4);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_answers_from_a_capture_unit_with_capture_provenance() {
        let root = temp_root("capture-unit");
        let capture = crate::ingest::capture_unit(
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            "/tmp/proj",
            "2026-08-01T09:00:00.000Z",
        );
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines = vec![capture.to_string()];
        for (index, text) in fillers.iter().enumerate() {
            let filler = json!({
                "key": format!("filler-{index:02}"),
                "kind": "user",
                "ts": "2026-08-01T08:00:00.000Z",
                "text": text
            });
            lines.push(filler.to_string());
        }
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "answered");
        assert_eq!(out.confidence, "medium");
        let answer = out.answer.as_ref().expect("unit answer");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.source_kind, "capture");
        assert_eq!(
            out.reason,
            "units layer answer (agent capture), confidence capped medium"
        );
        assert_eq!(
            answer.key,
            capture["key"].as_str().expect("capture key string")
        );
        fs::remove_dir_all(&root).ok();
    }
}
