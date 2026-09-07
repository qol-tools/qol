use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_home;
use crate::aliases::AliasMap;
use crate::retrieval::cache;
use crate::retrieval::{bm25_ranks, build_index, snippet, DocRef, Index};
use crate::retrieval_log::{self, Exclusion, RetrievalEvent};
use crate::skills::{self, Freshness, Served, SkillsIndex};
use crate::store::{
    dedupe_user_units, is_boilerplate_unit, Note, NotesLayer, Store, Unit, UnitsLayer,
};
use crate::text;

mod question_match;
pub mod rows;
mod selection;
pub(crate) mod semantic;

const SNIPPET_WINDOW: usize = 240;
const SKILL_CAP: usize = 2048;
const TOP_NOTE_LIMIT: usize = 5;
const NOTE_FETCH_LIMIT: usize = 40;
const RELATED_LIMIT: usize = 5;
const RECALLED_UNIT_LIMIT: usize = 5;
const RECALLED_LIMIT: usize = 8;
const AGREE_JACCARD: f64 = 0.5;

#[derive(Debug)]
pub struct AskRequest {
    pub query: String,
    pub k: usize,
    pub brief: bool,
    pub exclude_session: Option<String>,
    pub agent_home: Option<String>,
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
    #[serde(rename = "NOTE_MARGIN")]
    pub note_margin: f64,
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
        note_margin: 1.25,
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
            note_margin: lookup_gate(&lookup, "MEM_NOTE_MARGIN", Gates::DEFAULTS.note_margin),
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
            && self.note_margin == Gates::DEFAULTS.note_margin
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
    host: Option<String>,
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
    source_host: Option<String>,
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
    let floor = idf.values().copied().fold(0.0, f64::max);
    let lower = note_text.to_lowercase();
    let mut num = 0.0;
    let mut den = 0.0;
    for t in qt {
        let w = idf.get(t).copied().unwrap_or(floor);
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

fn must_match_tokens(qt: &[String], idf: &HashMap<String, f64>) -> Vec<String> {
    let floor = idf.values().copied().fold(0.0, f64::max);
    let max_w = qt
        .iter()
        .map(|token| idf.get(token).copied().unwrap_or(floor))
        .fold(0.0, f64::max);
    qt.iter()
        .filter(|token| match idf.get(token.as_str()) {
            Some(weight) => *weight >= 0.6 * max_w,
            None => false,
        })
        .cloned()
        .collect()
}

fn note_covers_must_match(
    resolved: &NoteHit,
    superseded: Option<&Vec<NoteHit>>,
    must_match: &[String],
) -> bool {
    let lower = resolved.text.to_lowercase();
    let missing: Vec<&String> = must_match
        .iter()
        .filter(|token| !lower.contains(token.as_str()))
        .collect();
    if missing.is_empty() {
        return true;
    }
    let Some(superseded) = superseded else {
        return false;
    };
    if superseded.is_empty() {
        return false;
    }
    missing.iter().all(|token| {
        superseded
            .iter()
            .all(|hit| !hit.text.to_lowercase().contains(token.as_str()))
    })
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

pub struct WarmIndexes<'a> {
    pub answer: &'a Index,
    pub all: &'a Index,
    pub notes: &'a Index,
    pub user_units: &'a [Unit],
    pub answer_pool: &'a [Unit],
    pub visible_notes: &'a [Note],
    pub by_key: &'a HashMap<String, usize>,
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
    run_with_warm(store, aliases, req, units, notes_layer, None)
}

pub(crate) fn run_with_warm(
    store: &Store,
    aliases: &AliasMap,
    req: &AskRequest,
    units: &UnitsLayer,
    notes_layer: &NotesLayer,
    warm: Option<&WarmIndexes<'_>>,
) -> Result<AskOutput> {
    let registry = qol_agent_homes::Registry::load();
    let caller = registry.resolve_caller(req.agent_home.as_deref());
    let slug = agent_home::cache_slug(&caller);

    let exclude_session: Option<String> = req
        .exclude_session
        .clone()
        .filter(|session| !session.is_empty());

    let mut qtokens0 = text::tokens(&req.query);
    qtokens0.retain(|token| !stopword_set().contains(token.as_str()));
    let qtokens = crate::aliases::expand_tokens(&qtokens0, aliases);

    let user_units_input: Vec<Unit>;
    let user_units_owned: Vec<Unit>;
    let answer_pool_owned: Vec<Unit>;
    let user_units: &[Unit];
    let answer_pool: &[Unit];
    if let Some(indexes) = warm {
        user_units = indexes.user_units;
        answer_pool = indexes.answer_pool;
    } else {
        user_units_input = units
            .items
            .iter()
            .filter(|unit| {
                crate::store::in_answer_pool(&unit.kind)
                    && agent_home::visible(unit, &caller, &registry)
            })
            .cloned()
            .collect();
        user_units_owned = dedupe_user_units(&user_units_input);
        answer_pool_owned = user_units_owned
            .iter()
            .filter(|unit| {
                !is_boilerplate_unit(unit)
                    && exclude_session
                        .as_deref()
                        .is_none_or(|skip| unit.session.as_deref() != Some(skip))
            })
            .cloned()
            .collect();
        user_units = &user_units_owned;
        answer_pool = &answer_pool_owned;
    }

    let warm_answer = warm.map(|indexes| indexes.answer);
    let answer_owned;
    let answer_idx: &Index = match warm_answer {
        Some(index) => index,
        None => {
            let answer_layer = exclude_session.as_deref().map_or_else(
                || format!("pool-{slug}"),
                |session| format!("pool-x-{}-{slug}", text::utf16_slice(session, 0, 8)),
            );
            answer_owned = cache::build_or_load(
                store.root(),
                &answer_layer,
                &doc_refs(answer_pool),
                Some(&units.path),
            );
            &answer_owned
        }
    };
    let units_query =
        crate::aliases::expand_tokens_keep(&text::tokens(&req.query), aliases).join(" ");
    let widened = warm_answer.is_some() && exclude_session.is_some() && req.k > 0;
    let answer_fetch = if widened { req.k * 2 + 16 } else { req.k };
    let mut answer_ranked: Vec<UnitHit> = bm25_ranks(&units_query, answer_idx, answer_fetch)
        .into_iter()
        .filter_map(|ranked| {
            let unit = match warm {
                Some(indexes) => {
                    let position = *indexes.by_key.get(&ranked.key)?;
                    indexes.user_units.get(position)?
                }
                None => answer_pool.iter().find(|unit| unit.key == ranked.key)?,
            };
            if exclude_session
                .as_deref()
                .is_some_and(|skip| unit.session.as_deref() == Some(skip))
            {
                return None;
            }
            Some(UnitHit {
                key: ranked.key,
                score: ranked.score,
                kind: unit.kind.clone(),
                source: unit.source.clone(),
                session: unit.session.clone(),
                cwd: unit.cwd.clone(),
                ts: unit.ts.clone(),
                host: unit.host.clone(),
                text: unit.text.clone(),
            })
        })
        .collect();
    if widened {
        answer_ranked.truncate(req.k);
    }

    let all_owned;
    let all_idx: &Index = match warm {
        Some(indexes) => indexes.all,
        None => {
            all_owned = cache::build_or_load(
                store.root(),
                &format!("user-{slug}"),
                &doc_refs(user_units),
                Some(&units.path),
            );
            &all_owned
        }
    };
    let ranked_all: Vec<UnitHit> = bm25_ranks(&units_query, all_idx, req.k)
        .into_iter()
        .filter_map(|ranked| {
            let unit = match warm {
                Some(indexes) => {
                    let position = *indexes.by_key.get(&ranked.key)?;
                    indexes.user_units.get(position)?
                }
                None => user_units.iter().find(|unit| unit.key == ranked.key)?,
            };
            Some(UnitHit {
                key: ranked.key,
                score: ranked.score,
                kind: unit.kind.clone(),
                source: unit.source.clone(),
                session: unit.session.clone(),
                cwd: unit.cwd.clone(),
                ts: unit.ts.clone(),
                host: unit.host.clone(),
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
            host: hit.host.clone(),
            snippet: snippet(&hit.text, &qtokens, SNIPPET_WINDOW),
        })
        .collect();

    let notes_owned: Vec<Note>;
    let notes: &[Note] = match warm {
        Some(indexes) => indexes.visible_notes,
        None => {
            notes_owned = visible_notes(&notes_layer.items, &units.items, &caller, &registry);
            &notes_owned
        }
    };
    let notes_index_owned;
    let notes_idx: Option<&Index> = match warm {
        Some(indexes) => Some(indexes.notes),
        None if notes.is_empty() => None,
        None => {
            notes_index_owned = cache::build_or_load(
                store.root(),
                &format!("notes-{slug}"),
                &notes_refs(notes),
                None,
            );
            Some(&notes_index_owned)
        }
    };
    let notes_query = qtokens.join(" ");
    let ranked_note_hits: Vec<(&crate::store::Note, f64)> = match &notes_idx {
        Some(idx) => bm25_ranks(&notes_query, idx, NOTE_FETCH_LIMIT)
            .into_iter()
            .filter_map(|ranked| {
                notes
                    .iter()
                    .find(|note| note.key == ranked.key)
                    .map(|note| (note, ranked.score))
            })
            .collect(),
        None => Vec::new(),
    };
    let top_notes: Vec<NoteHit> = ranked_note_hits
        .iter()
        .filter(|(note, _)| crate::store::is_claim_note(note))
        .take(TOP_NOTE_LIMIT)
        .map(|(note, score)| note_hit(note, *score))
        .collect();
    let related: Vec<Related> = ranked_note_hits
        .iter()
        .filter(|(note, _)| !crate::store::is_claim_note(note))
        .take(RELATED_LIMIT)
        .map(|(note, _)| Related {
            text: note.text.clone(),
            cls: note.cls.clone(),
            source_ts: note.source_ts.clone(),
        })
        .collect();

    let skills_out = build_skills_out(store, &req.query, req.brief)?;

    let note_top: Option<&NoteHit> = top_notes.first();
    let disliked = crate::feedback::disliked_by_norm(store.root());
    let query_norm = retrieval_log::normalize_query(&req.query);
    let capture_selection = selection::select(
        &req.query,
        answer_pool,
        exclude_session.as_deref(),
        disliked.get(&query_norm),
    );
    let selected_capture = capture_selection.winner.as_ref().map(|winner| UnitHit {
        key: winner.unit.key.clone(),
        score: answer_ranked
            .iter()
            .find(|hit| hit.key == winner.unit.key)
            .map_or(0.0, |hit| hit.score),
        kind: winner.unit.kind.clone(),
        source: winner.unit.source.clone(),
        session: winner.unit.session.clone(),
        cwd: winner.unit.cwd.clone(),
        ts: winner.unit.ts.clone(),
        host: winner.unit.host.clone(),
        text: winner.evidence.display.clone(),
    });
    let unit_top: Option<UnitHit> = selected_capture.or_else(|| {
        answer_ranked
            .iter()
            .find(|hit| {
                disliked
                    .get(&query_norm)
                    .is_none_or(|keys| !keys.contains(&hit.key))
                    && (hit.kind != crate::ingest::CAPTURE_KIND
                        || question_match::evidence(&hit.text).is_none())
            })
            .cloned()
    });
    let raw_unit_margin = match &unit_top {
        Some(top) => {
            let competitor = answer_ranked.iter().find(|hit| {
                hit.key != top.key
                    && disliked
                        .get(&query_norm)
                        .is_none_or(|keys| !keys.contains(&hit.key))
                    && text::token_jaccard(&top.text, &hit.text) < AGREE_JACCARD
            });
            match competitor {
                Some(major) => top.score / major.score,
                None => f64::INFINITY,
            }
        }
        None => 0.0,
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
    let unit_question_match = capture_selection.winner.is_some();

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
    let mut outcome = Outcome::Unsupported;
    let mut reason_code = Reason::BelowThreshold;
    let mut answer: Option<Answer> = None;

    let below_floor = |score: Option<f64>| score.unwrap_or(0.0) < gates.floor;

    let reason;

    if capture_selection.conflicts > 0 {
        verdict = "candidates".to_owned();
        confidence = "low".to_owned();
        outcome = Outcome::Conflicting;
        reason_code = Reason::ConflictingCaptures;
        reason = "matching captures contain conflicting answers".to_owned();
    } else if !unit_question_match
        && ((max_cov < gates.no_memory_cov && !has_recency_answer)
            || (below_floor(note_top.map(|note| note.score))
                && below_floor(unit_top.as_ref().map(|top| top.score))
                && !has_recency_answer))
    {
        reason = format!(
            "no memory above the answer threshold (max_cov={}, floor={})",
            fixed2_string(max_cov),
            gates.floor
        );
    } else {
        let must_match = match &notes_idx {
            Some(idx) => must_match_tokens(&qtokens, &idx.idf),
            None => Vec::new(),
        };
        let note_rival = note_resolved.as_ref().and_then(|resolved| {
            top_notes.iter().find(|hit| {
                hit.key != resolved.key
                    && hit.family_key() != resolved.family_key()
                    && text::token_jaccard(&resolved.text, &hit.text) < AGREE_JACCARD
            })
        });
        let note_margin = note_resolved.as_ref().map_or(0.0, |resolved| {
            note_rival.map_or(f64::INFINITY, |hit| resolved.score / hit.score)
        });
        let note_winner = note_resolved.as_ref().is_some_and(|resolved| {
            note_decisive
                && curated_kinds().contains(resolved.source_kind.as_deref().unwrap_or(""))
                && (note_cov_r >= gates.note_cov
                    || (fam_relevant && note_superseded.as_ref().is_some_and(|s| !s.is_empty())))
                && resolved.score >= gates.note_score
                && (note_margin >= gates.note_margin
                    || note_superseded.as_ref().is_some_and(|s| !s.is_empty()))
                && note_covers_must_match(resolved, note_superseded.as_ref(), &must_match)
        });
        let unit_winner = unit_question_match
            || unit_top.as_ref().is_some_and(|top| {
                let literal_capture = top.kind == crate::ingest::CAPTURE_KIND
                    && question_match::Question::parse(&req.query).is_none()
                    && !req.query.trim().is_empty()
                    && text::collapse_ws_lower(&top.text)
                        .contains(&text::collapse_ws_lower(&req.query));
                unit_cov >= gates.unit_cov
                    && top.score >= gates.unit_score
                    && !is_boilerplate_unit_user(top)
                    && (literal_capture || raw_unit_margin >= gates.unit_margin)
            });
        let capture_outranks_note = unit_question_match
            || unit_winner
                && unit_top
                    .as_ref()
                    .is_some_and(|top| top.kind == crate::ingest::CAPTURE_KIND)
                && unit_cov
                    > phrased_coverage(
                        &qtokens,
                        note_resolved
                            .as_ref()
                            .map(|resolved| resolved.text.as_str()),
                    );

        if note_winner && !capture_outranks_note {
            let resolved = note_resolved
                .as_ref()
                .expect("winner keeps a resolved note");
            let margin = note_margin;
            let high = margin >= gates.high_margin
                && note_superseded.as_ref().is_none_or(|s| s.is_empty());
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
                host: resolved.source_host.clone(),
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
                supporting_keys: Vec::new(),
            });
            verdict = "answered".to_string();
            confidence = if high { "high" } else { "medium" }.to_string();
            outcome = Outcome::Supported;
            reason_code = Reason::NotesAnswer;
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
                host: top.host.clone(),
                score: text::to_fixed2(top.score),
                margin: None,
                superseded: None,
                supporting_keys: capture_selection.supporting_keys.clone(),
            });
            verdict = "answered".to_string();
            confidence = "medium".to_string();
            outcome = Outcome::Supported;
            reason_code = if top.kind == crate::ingest::CAPTURE_KIND {
                Reason::CaptureAnswer
            } else {
                Reason::TranscriptAnswer
            };
            reason = if top.kind == crate::ingest::CAPTURE_KIND {
                "units layer answer (agent capture), confidence capped medium".to_string()
            } else if top.kind == crate::ingest::ASSISTANT_KIND {
                "units layer answer (assistant reply), confidence capped medium".to_string()
            } else {
                "units layer answer (user transcript), confidence capped medium".to_string()
            };
        } else {
            verdict = "candidates".to_string();
            confidence = "low".to_string();
            outcome = Outcome::Ambiguous;
            reason_code = Reason::NoDecisiveAnswer;
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

    let mut recalled: Vec<Recalled> = top_notes
        .iter()
        .map(|note| Recalled {
            key: note.key.clone(),
            cls: note.cls.clone(),
            score: text::to_fixed2(note.score),
            source_kind: note.source_kind.clone(),
            source_ts: note.source_ts.clone(),
            host: note.source_host.clone(),
            layer: Some("note".to_string()),
        })
        .collect();
    recalled.extend(
        ranked_all
            .iter()
            .filter(|hit| crate::store::is_claim_unit_kind(&hit.kind))
            .take(RECALLED_UNIT_LIMIT)
            .map(|hit| Recalled {
                key: hit.key.clone(),
                cls: hit.kind.clone(),
                score: text::to_fixed2(hit.score),
                source_kind: Some(hit.kind.clone()),
                source_ts: hit.ts.clone(),
                host: hit.host.clone(),
                layer: Some("unit".to_string()),
            }),
    );
    recalled.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    recalled.truncate(RECALLED_LIMIT);

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
                    host: note.source_host.clone(),
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
                    host: note.source_host.clone(),
                    score: note.score,
                }
            }
        })
        .collect();

    Ok(AskOutput {
        verification: None,
        query: req.query.clone(),
        agent_home: caller,
        host: crate::host::current().to_string(),
        verdict,
        confidence,
        reason,
        outcome,
        reason_code,
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
            unit_question_match,
            matching_captures: capture_selection.matching,
            conflicting_captures: capture_selection.conflicts,
            max_token_coverage: text::to_fixed2(f64::max(note_cov, unit_cov)),
            notes_run_ts: notes_layer.run.clone(),
            snapshot_run_ts: units.run.clone(),
            live_units,
            stale_layer,
            recency_resolved: note_superseded.as_ref().map(|list| !list.is_empty()),
            has_multi_intent,
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

pub(crate) fn visible_notes(
    notes: &[crate::store::Note],
    units: &[Unit],
    caller: &str,
    registry: &qol_agent_homes::Registry,
) -> Vec<crate::store::Note> {
    let unit_keys: HashSet<&str> = units.iter().map(|unit| unit.key.as_str()).collect();
    let visible_unit_keys: HashSet<&str> = units
        .iter()
        .filter(|unit| agent_home::visible(unit, caller, registry))
        .map(|unit| unit.key.as_str())
        .collect();
    notes
        .iter()
        .filter(|note| match note.source_key.as_deref() {
            Some(key) if unit_keys.contains(key) => visible_unit_keys.contains(key),
            _ => true,
        })
        .cloned()
        .collect()
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

fn note_hit(note: &crate::store::Note, score: f64) -> NoteHit {
    NoteHit {
        key: note.key.clone(),
        cls: note.cls.clone(),
        text: note.text.clone(),
        source_key: note.source_key.clone(),
        source_ts: note.source_ts.clone(),
        source_kind: note.source_kind.clone(),
        source_host: note.source_host.clone(),
        score,
    }
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
    run_and_log_with_layers(store, aliases, req, log, &units, &notes, None)
}

pub fn run_and_log_with_layers(
    store: &Store,
    aliases: &AliasMap,
    req: &AskRequest,
    log: &LogOptions,
    units: &UnitsLayer,
    notes: &NotesLayer,
    warm: Option<&WarmIndexes<'_>>,
) -> Result<AskOutput> {
    let started = Instant::now();
    let out = run_with_warm(store, aliases, req, units, notes, warm)?;
    let latency_ms = started.elapsed().as_millis() as u64;
    log_output(store, req, log, &out, latency_ms);
    Ok(out)
}

pub(crate) fn log_output(
    store: &Store,
    req: &AskRequest,
    log: &LogOptions,
    out: &AskOutput,
    latency_ms: u64,
) {
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
            agent_home: out.agent_home.clone(),
            host: crate::host::current().to_string(),
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
}

pub fn render_text(out: &AskOutput) -> String {
    let host_suffix = |host: &Option<String>| match host {
        Some(h) if h != crate::host::current() => format!(" [from {h}]"),
        _ => String::new(),
    };
    let mut lines = Vec::new();
    lines.push(format!("verdict: {} ({})", out.verdict, out.confidence));
    lines.push(format!("reason: {}", out.reason));
    match out.verification.as_ref() {
        Some(crate::verification::service::Status::Pending) => lines.push("verification: checking the recorded answers in the background; repeat this question for the result".into()),
        Some(crate::verification::service::Status::Unavailable) => lines.push("verification: local answer checking is unavailable; related memories are still shown".into()),
        _ => {}
    }
    if out.verdict == "answered" {
        if let Some(answer) = &out.answer {
            let cls = match &answer.cls {
                Some(cls) => cls.clone(),
                None => "-".to_string(),
            };
            lines.push(format!(
                "answer [{}/{}]: {}{}",
                answer.layer,
                cls,
                answer.text,
                host_suffix(&answer.host)
            ));
        }
    }
    lines.push("recalled:".to_string());
    for recall in &out.recalled {
        lines.push(format!(
            "  {}  {}  {}{}",
            recall.key,
            recall.cls,
            recall.score,
            host_suffix(&recall.host)
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

    let registry = qol_agent_homes::Registry::load();
    let caller = registry.resolve_caller(None);
    let slug = agent_home::cache_slug(&caller);

    let user_units_input: Vec<Unit> = units
        .items
        .iter()
        .filter(|unit| {
            crate::store::in_answer_pool(&unit.kind)
                && agent_home::visible(unit, &caller, &registry)
        })
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
    let note_items = visible_notes(&notes.items, &units.items, &caller, &registry);
    let note_refs = notes_refs(&note_items);

    let cache_label = |state: cache::CacheState| match state {
        cache::CacheState::Fresh => "fresh",
        cache::CacheState::Stale => "stale",
        cache::CacheState::Missing => "missing",
    };
    let pool_state = cache_label(cache::cache_state(
        store.root(),
        &format!("pool-{slug}"),
        &pool_refs,
        Some(&units.path),
    ));
    let user_state = cache_label(cache::cache_state(
        store.root(),
        &format!("user-{slug}"),
        &user_refs,
        Some(&units.path),
    ));
    let notes_state = cache_label(cache::cache_state(
        store.root(),
        &format!("notes-{slug}"),
        &note_refs,
        None,
    ));

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Supported,
    Qualified,
    Ambiguous,
    Conflicting,
    Unsupported,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Supported => "supported",
            Outcome::Qualified => "qualified",
            Outcome::Ambiguous => "ambiguous",
            Outcome::Conflicting => "conflicting",
            Outcome::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ConflictingCaptures,
    BelowThreshold,
    NotesAnswer,
    CaptureAnswer,
    TranscriptAnswer,
    NoDecisiveAnswer,
    VerifiedAnswer,
}

impl Reason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reason::ConflictingCaptures => "conflicting_captures",
            Reason::BelowThreshold => "below_threshold",
            Reason::NotesAnswer => "notes_answer",
            Reason::CaptureAnswer => "capture_answer",
            Reason::TranscriptAnswer => "transcript_answer",
            Reason::NoDecisiveAnswer => "no_decisive_answer",
            Reason::VerifiedAnswer => "verified_answer",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AskOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<crate::verification::service::Status>,
    pub query: String,
    #[serde(default)]
    pub agent_home: String,
    #[serde(default)]
    pub host: String,
    pub verdict: String,
    pub confidence: String,
    pub reason: String,
    pub outcome: Outcome,
    pub reason_code: Reason,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub score: f64,
    pub margin: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded: Option<Option<Vec<Superseded>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
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
    #[serde(default)]
    pub unit_question_match: bool,
    #[serde(default)]
    pub matching_captures: usize,
    #[serde(default)]
    pub conflicting_captures: usize,
    pub max_token_coverage: f64,
    pub notes_run_ts: Option<String>,
    pub snapshot_run_ts: String,
    pub live_units: bool,
    pub stale_layer: bool,
    pub recency_resolved: Option<bool>,
    #[serde(default)]
    pub has_multi_intent: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
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

    fn margin_fillers() -> Vec<String> {
        [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
            "piano violin cello flute clarinet trumpet saxophone banjo ukulele harmony rhythm tempo octave",
            "summit valley glacier tundra prairie savanna wetland estuary delta basin plateau canyon ridge",
            "cedar birch maple willow aspen spruce fir hickory walnut chestnut sycamore mangrove bamboo",
            "sonnet haiku ballad elegy ode lyric verse stanza rhythm meter couplet refrain epic prose",
            "harbor jetty quay wharf pier dock berth marina lighthouse breakwater anchor mooring tugboat",
            "saffron paprika cumin turmeric ginger cinnamon nutmeg clove anise fennel thyme sage basil",
            "nova quasar pulsar nebula galaxy comet asteroid meteor planet dwarf orbit eclipse zenith",
            "tundra pampa llanos outback bushveld pampas steppe veld taiga chaparral maquis caatinga",
        ]
        .iter()
        .enumerate()
        .map(|(index, filler)| {
            json!({
                "key": format!("filler-{index:02}"),
                "kind": "user",
                "ts": "2026-08-01T08:00:00.000Z",
                "text": filler
            })
            .to_string()
        })
        .collect()
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
        rows.push("{\"key\":\"n-count-new\",\"cls\":\"decision\",\"source_kind\":\"decision-deter\",\"source_ts\":\"2026-08-06T08:00:00.000Z\",\"text\":\"count 4101 user units in the corpus\"}".to_string());
        rows.push("{\"key\":\"n-count-old\",\"cls\":\"decision\",\"source_kind\":\"decision-deter\",\"source_ts\":\"2026-08-02T08:00:00.000Z\",\"text\":\"count 3922 user units in the corpus\"}".to_string());
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
                agent_home: None,
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
            source_host: None,
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
    fn weighted_note_cov_uses_idf_weights_and_zero_coverage_without_matches() {
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
    fn weighted_note_cov_charges_terms_absent_from_the_idf_map_the_max_idf_floor() {
        let idf = HashMap::from([
            ("common".to_string(), 0.5),
            ("rare".to_string(), 4.0),
            ("lorem".to_string(), 2.0),
        ]);
        let qt = vec![
            "common".to_string(),
            "rare".to_string(),
            "offvocab".to_string(),
        ];
        let cov = weighted_note_cov(&qt, "has Common tokens and the RARE one", &idf);
        assert!(cov < 1.0);
        assert!((cov - (0.5 + 4.0) / (0.5 + 4.0 + 4.0)).abs() < 1e-12);
    }

    #[test]
    fn must_match_tokens_keeps_informative_terms_present_in_the_idf_map() {
        let idf = HashMap::from([
            ("common".to_string(), 0.5),
            ("rare".to_string(), 4.0),
            ("mid".to_string(), 2.6),
        ]);
        let qt = vec![
            "common".to_string(),
            "rare".to_string(),
            "mid".to_string(),
            "offvocab".to_string(),
        ];
        assert_eq!(
            must_match_tokens(&qt, &idf),
            vec!["rare".to_string(), "mid".to_string()]
        );
        assert_eq!(must_match_tokens(&[], &idf), Vec::<String>::new());
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
    fn the_note_margin_gate_is_configurable_and_the_curated_note_clears_it() {
        let root = temp_root("note-margin");
        build_fixture(&root);
        let store = Store::resolve(Some(&root)).expect("store resolves");

        let gates =
            Gates::from_lookup(|name| (name == "MEM_NOTE_MARGIN").then(|| "99".to_string()));

        assert_eq!(gates.note_margin, 99.0);
        assert!(!gates.is_default());

        let out = run_ask(
            &store,
            "does the clipboard history ring survive tray restarts",
            false,
        );

        assert_eq!(out.verdict, "answered");
        assert!(
            out.answer.expect("note answer").margin.unwrap() >= Gates::DEFAULTS.note_margin,
            "an answering note must outscore its nearest disagreeing rival"
        );

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
            agent_home: None,
        };
        let excluded_out = run(&store, &AliasMap::default(), &excluded).expect("excluded ask runs");
        assert_eq!(excluded_out.counts.units, 4);
        let registry = qol_agent_homes::Registry::load();
        let caller = registry.resolve_caller(None);
        let slug = agent_home::cache_slug(&caller);
        assert!(root
            .join(format!("idx-pool-x-sess-liv-{slug}.json"))
            .exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn warm_ask_with_excluded_session_never_answers_from_that_session() {
        let root = temp_root("warm-exclude");
        let tail = "quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz";
        let mut lines = vec![
            json!({
                "key": "u-a1",
                "session": "sess-live-aaa1",
                "kind": "user",
                "ts": "2026-08-01T09:00:00.000Z",
                "text": "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
            })
            .to_string(),
            json!({
                "key": "u-a2",
                "session": "sess-live-aaa1",
                "kind": "user",
                "ts": "2026-08-01T10:00:00.000Z",
                "text": "bravo charlie delta echo foxtrot golf hotel india juliet kilo lima alpha"
            })
            .to_string(),
            json!({
                "key": "u-b",
                "session": "sess-live-bbb2",
                "kind": "user",
                "ts": "2026-08-01T11:00:00.000Z",
                "text": "lima kilo juliet india hotel golf foxtrot echo delta charlie bravo alpha"
            })
            .to_string(),
        ];
        for (index, first) in [
            "ember", "saddle", "tunnel", "violin", "window", "yogurt", "copper", "silver", "velvet",
        ]
        .iter()
        .enumerate()
        {
            lines.push(
                json!({
                    "key": format!("f-{index}"),
                    "session": "sess-live-ccc3",
                    "kind": "user",
                    "ts": format!("2026-08-02T09:{index:02}:00.000Z"),
                    "text": format!("{first} {tail}"),
                })
                .to_string(),
            );
        }
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let units_layer = store.read_units().expect("units read");
        let notes_layer = NotesLayer {
            run: None,
            items: Vec::new(),
        };
        let registry = qol_agent_homes::Registry::load();
        let caller = registry.resolve_caller(None);
        let user_units: Vec<Unit> = units_layer
            .items
            .iter()
            .filter(|unit| {
                crate::store::in_answer_pool(&unit.kind)
                    && agent_home::visible(unit, &caller, &registry)
            })
            .cloned()
            .collect();
        let pool: Vec<Unit> = user_units
            .iter()
            .filter(|unit| !is_boilerplate_unit(unit))
            .cloned()
            .collect();
        let answer_index = build_index(&doc_refs(&pool));
        let all_index = build_index(&doc_refs(&user_units));
        let notes_index = build_index(&[]);
        let by_key: HashMap<String, usize> = user_units
            .iter()
            .enumerate()
            .map(|(position, unit)| (unit.key.clone(), position))
            .collect();
        let warm = WarmIndexes {
            answer: &answer_index,
            all: &all_index,
            notes: &notes_index,
            user_units: &user_units,
            answer_pool: &pool,
            visible_notes: &[],
            by_key: &by_key,
        };
        let req = AskRequest {
            query: "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
                .to_string(),
            k: 2,
            brief: false,
            exclude_session: Some("sess-live-aaa1".to_string()),
            agent_home: None,
        };
        let log = LogOptions {
            source: "test".to_string(),
            cwd: None,
            fact: None,
            no_log: true,
        };
        let out = run_and_log_with_layers(
            &store,
            &AliasMap::default(),
            &req,
            &log,
            &units_layer,
            &notes_layer,
            Some(&warm),
        )
        .expect("warm ask runs");
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("unit answer resolves");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.key, "u-b");
        assert_eq!(answer.session.as_deref(), Some("sess-live-bbb2"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_scopes_units_to_the_caller_home_and_shared_homes() {
        let root = temp_root("home-scope");
        let registry = qol_agent_homes::Registry::load();
        let caller = "/tmp/qol-home-mine";
        let units = [
            json!({
                "key": "u-mine",
                "agent_home": caller,
                "kind": "user",
                "ts": "2026-08-01T09:00:00.000Z",
                "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz"
            }),
            json!({
                "key": "u-theirs",
                "agent_home": "/tmp/qol-home-theirs",
                "kind": "user",
                "ts": "2026-08-01T10:00:00.000Z",
                "text": "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz"
            }),
            json!({
                "key": "u-shared",
                "agent_home": registry.default_for(qol_agent_homes::Harness::Pi).id,
                "kind": "user",
                "ts": "2026-08-01T11:00:00.000Z",
                "text": "aurora quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz"
            }),
        ];
        let body = units
            .iter()
            .map(|unit| unit.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        write_units(&root, &body);
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run(
            &store,
            &AliasMap::default(),
            &AskRequest {
                query: "ember zephyr aurora quartz".to_string(),
                k: 5,
                brief: false,
                exclude_session: None,
                agent_home: Some(caller.to_string()),
            },
        )
        .expect("ask runs");
        assert_eq!(out.agent_home, caller);
        let keys: Vec<&str> = out
            .units
            .as_ref()
            .expect("full ask keeps units")
            .iter()
            .map(|unit| unit.key.as_str())
            .collect();
        assert!(keys.contains(&"u-mine"));
        assert!(keys.contains(&"u-shared"));
        assert!(!keys.contains(&"u-theirs"));
        let slug = agent_home::cache_slug(caller);
        assert!(root.join(format!("idx-user-{slug}.json")).exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn status_reports_the_default_caller_cache_layers() {
        let root = temp_root("status-layers");
        build_fixture(&root);
        let foreign = json!({
            "key": "u-foreign",
            "agent_home": "/tmp/qol-home-private",
            "kind": "user",
            "ts": "2026-08-01T12:00:00.000Z",
            "text": "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz"
        });
        write_units(&root, &format!("{}\n{}", fixture_units(), foreign));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "does the clipboard history ring survive tray restarts",
            false,
        );
        assert_eq!(out.verdict, "answered");
        assert_eq!(out.counts.units, 4);
        let units = store.read_units().expect("units layer");
        let notes = store.read_notes().expect("notes layer");
        let value = status_with_layers(&store, &units, &notes).expect("status runs");
        assert_eq!(value["index_caches"]["pool"], "fresh");
        assert_eq!(value["index_caches"]["user"], "fresh");
        assert_eq!(value["index_caches"]["notes"], "fresh");
        crate::app::warm::reindex(&store).expect("reindex runs");
        run_ask(
            &store,
            "does the clipboard history ring survive tray restarts",
            false,
        );
        let units = store.read_units().expect("units layer");
        let notes = store.read_notes().expect("notes layer");
        let value = status_with_layers(&store, &units, &notes).expect("status runs");
        assert_eq!(value["index_caches"]["pool"], "fresh");
        assert_eq!(value["index_caches"]["user"], "fresh");
        assert_eq!(value["index_caches"]["notes"], "fresh");
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
            "notes layer decision answer, margin 2.74x, recency-resolved (superseded a stale fact)"
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
    fn ask_charges_unknown_terms_so_a_partial_claim_no_longer_answers() {
        let root = temp_root("vague-claim");
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines: Vec<String> = Vec::new();
        lines.push(
            json!({
                "key": "u-qol",
                "kind": "user",
                "ts": "2026-08-01T09:00:00.000Z",
                "text": "worked inside the qol monorepo worktree all morning"
            })
            .to_string(),
        );
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
        let claim = json!({
            "key": "n-qol-path",
            "cls": "decision",
            "source_kind": "decision",
            "source_ts": "2026-08-04T09:00:00.000Z",
            "text": "qol monorepo lives at /Users/kaho/repos/private/qol-monorepo"
        });
        let mut digests: Vec<String> = Vec::new();
        for i in 0..10usize {
            digests.push(
                json!({
                    "key": format!("n-qolfill-{i:02}"),
                    "cls": "observation",
                    "source_kind": "count",
                    "source_ts": format!("2026-08-03T08:{i:02}:00.000Z"),
                    "text": format!("qol monorepo weekly digest {i:02} qqfill{i:02}")
                })
                .to_string(),
            );
        }
        write_newest_run_notes(
            &root,
            &format!("{}\n{}\n{}", fixture_notes(), claim, digests.join("\n")),
        );
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, "what language is the qol monorepo", false);
        assert_eq!(out.verdict, "no-memory");
        assert_eq!(out.answer, None);
        let claim_recalled = out
            .recalled
            .iter()
            .find(|recall| recall.key == "n-qol-path")
            .expect("claim still recalled");
        assert_eq!(claim_recalled.cls, "decision");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_rejects_a_claim_missing_an_informative_term_from_the_notes_vocabulary() {
        let root = temp_root("must-match");
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines: Vec<String> = Vec::new();
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
        let claim = json!({
            "key": "n-claim",
            "cls": "decision",
            "source_kind": "decision",
            "source_ts": "2026-08-04T09:00:00.000Z",
            "text": "Decision: ember quartz flint are tracked in the ledger"
        });
        let unrelated = json!({
            "key": "n-unrelated",
            "cls": "observation",
            "source_kind": "count",
            "source_ts": "2026-08-03T09:00:00.000Z",
            "text": "zephyr aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise"
        });
        write_newest_run_notes(
            &root,
            &format!("{}\n{}\n{}", fixture_notes(), claim, unrelated),
        );
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, "ember quartz flint zephyr", false);
        assert_eq!(out.verdict, "candidates");
        assert_eq!(out.answer, None);
        assert!(out.recalled.iter().any(|recall| recall.key == "n-claim"));
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

    #[test]
    fn ask_answers_from_a_capture_shadowed_by_its_source_transcript() {
        let root = temp_root("capture-shadow");
        let capture = crate::ingest::capture_unit(
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            "/tmp/proj",
            "2026-08-01T09:00:00.000Z",
        );
        let shadow = json!({
            "key": "u-source",
            "kind": "user",
            "ts": "2026-08-01T08:30:00.000Z",
            "text": "the ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz crystals were catalogued during the last survey"
        });
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines = vec![capture.to_string(), shadow.to_string()];
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
            answer.key,
            capture["key"].as_str().expect("capture key string")
        );
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(
            text::token_jaccard(
                capture["text"].as_str().expect("capture text"),
                shadow["text"].as_str().expect("shadow text"),
            ) >= AGREE_JACCARD
        );
        assert!(margin >= Gates::DEFAULTS.unit_margin);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_keeps_the_margin_gate_for_user_units() {
        let root = temp_root("user-margin");
        let top_user = json!({
            "key": "u-a",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz were catalogued during the spring mineralogy survey"
        });
        let rival_user = json!({
            "key": "u-b",
            "kind": "user",
            "ts": "2026-08-01T08:00:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz got auctioned at our autumn gemstone exchange yesterday"
        });
        assert!(
            text::token_jaccard(
                top_user["text"].as_str().expect("top text"),
                rival_user["text"].as_str().expect("rival text")
            ) < AGREE_JACCARD
        );
        let mut lines = vec![top_user.to_string(), rival_user.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "candidates");
        assert_eq!(out.answer, None);
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(margin < Gates::DEFAULTS.unit_margin);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn capture_answers_respect_the_unit_margin_gate() {
        let root = temp_root("capture-margin");
        let shared = "ember quartz flint cobalt onyx basalt garnet pyrite";
        let reordered = "quartz ember flint cobalt onyx basalt garnet pyrite";
        let short = crate::ingest::capture_unit(reordered, "/tmp/proj", "2026-08-01T09:00:00.000Z");
        let long = json!({
            "key": "u-long",
            "kind": "capture",
            "ts": "2026-08-01T09:30:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite were catalogued during the spring survey at the canyon station"
        });
        let mut lines = vec![short.to_string(), long.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, shared, false);
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(
            margin < Gates::DEFAULTS.unit_margin,
            "the short capture must face its longer rival below the margin gate, got {margin}"
        );
        assert_eq!(out.verdict, "candidates");
        assert_eq!(out.answer, None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn capture_restating_the_query_answers_without_a_margin() {
        let root = temp_root("capture-restates");
        let shared = "ember quartz flint cobalt onyx basalt garnet pyrite";
        let short = crate::ingest::capture_unit(shared, "/tmp/proj", "2026-08-01T09:00:00.000Z");
        let long = json!({
            "key": "u-long",
            "kind": "capture",
            "ts": "2026-08-01T09:30:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite were catalogued during the spring survey at the canyon station"
        });
        let mut lines = vec![short.to_string(), long.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, shared, false);
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(
            margin < Gates::DEFAULTS.unit_margin,
            "the short capture must still face its longer rival below the margin gate, got {margin}"
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.expect("restating capture answers");
        assert_eq!(answer.key, short["key"].as_str().expect("short key"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn launch_question_paraphrases_produce_an_answer_row() {
        let root = temp_root("launch-paraphrase");
        let captures = [
            "Run ./quartz-forge dev or ./quartz-forge -d. How to run Quartz in debug mode: quartz-forge dev launches the dev build with debug logging and arms the overlay.",
            "./quartz-forge -d. How to start Quartz in debug mode: ./quartz-forge -d launches the dev build with debug logging and arms the overlay in the background.",
        ];
        let mut lines: Vec<String> = captures
            .iter()
            .enumerate()
            .map(|(i, text)| {
                json!({"key": format!("capture-{i}"), "kind": "capture", "text": text}).to_string()
            })
            .collect();
        for i in 0..200 {
            lines.push(json!({"key": format!("filler-{i}"), "kind": "user", "text": format!("mineral survey field report {i} about garnet pyrite mica slate beryl opal")}).to_string());
        }
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).unwrap();
        for query in [
            "how to open quartz debug",
            "how do I launch quartz debug?",
            "how to run quartz in debug mode",
        ] {
            let out = run_ask(&store, query, false);
            assert_eq!(
                out.verdict,
                "answered",
                "{query}: {} {}",
                out.reason,
                serde_json::to_string(&out.signals).unwrap()
            );
            assert!(out.signals.unit_question_match, "{query}");
            let rows = rows::from_output(
                &out,
                &store.read_units().unwrap(),
                &store.read_notes().unwrap(),
            );
            assert_eq!(rows[0].kind, "answer", "{query}");
            assert!(rows[0].copy.contains("quartz-forge"), "{query}");
        }
        for query in [
            "what is quartz forge",
            "how to stop quartz debug",
            "how to open quartz debug remotely",
            "how to open quartz release",
        ] {
            let out = run_ask(&store, query, false);
            assert!(out.answer.is_none(), "{query}: {}", out.reason);
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_demotes_a_downvoted_unit_from_answering() {
        let root = temp_root("feedback-demotion");
        let query =
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz";
        let answer = json!({
            "key": "u-aaa",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": query
        });
        let alternate = json!({
            "key": "u-zzz",
            "kind": "user",
            "ts": "2026-08-01T09:30:00.000Z",
            "text": "topaz jade opal beryl slate mica pyrite garnet basalt onyx cobalt flint quartz ember"
        });
        let mut lines = vec![answer.to_string(), alternate.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(&store, query, false);
        assert_eq!(out.verdict, "answered");
        assert_eq!(out.answer.as_ref().expect("unit answer").key, "u-aaa");

        let norm = crate::retrieval_log::normalize_query(query);
        crate::feedback::append_vote(store.root(), &norm, "u-aaa", -1);
        let out = run_ask(&store, query, false);
        assert_eq!(out.verdict, "answered");
        assert_eq!(
            out.answer.as_ref().expect("unit answer").key,
            "u-zzz",
            "the downvoted unit must not answer; its rival takes over"
        );
        assert!(
            out.units
                .as_ref()
                .expect("full ask keeps units")
                .iter()
                .any(|unit| unit.key == "u-aaa"),
            "a downvoted unit still shows up in the recalled rows"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_prefers_a_full_coverage_capture_over_a_half_coverage_note() {
        let root = temp_root("capture-half-note");
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
        let extra = json!({
            "key": "n-half",
            "cls": "decision",
            "source_kind": "decision",
            "source_ts": "2026-08-04T09:00:00.000Z",
            "text": "Decision: ember quartz flint cobalt onyx basalt garnet were catalogued in the survey ledger"
        });
        write_newest_run_notes(&root, &format!("{}\n{}", fixture_notes(), extra));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("unit answer");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.source_kind, "capture");
        assert_eq!(answer.key, capture["key"].as_str().unwrap());
        assert_eq!(
            out.reason,
            "units layer answer (agent capture), confidence capped medium"
        );
        assert_eq!(out.signals.note_token_coverage, 0.5);
        assert!(out.signals.top_note_score.expect("note score") >= Gates::DEFAULTS.note_score);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_keeps_a_full_coverage_note_ahead_of_a_capture() {
        let root = temp_root("note-full-capture");
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
        let extra = json!({
            "key": "n-full",
            "cls": "decision",
            "source_kind": "decision",
            "source_ts": "2026-08-04T09:00:00.000Z",
            "text": "Decision: ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz are catalogued"
        });
        write_newest_run_notes(&root, &format!("{}\n{}", fixture_notes(), extra));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("note answer");
        assert_eq!(answer.layer, "note");
        assert_eq!(answer.key, "n-full");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_notes_stay_out_of_recalled_and_answer_and_surface_as_related() {
        let root = temp_root("path-related");
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines: Vec<String> = Vec::new();
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
        let decision = json!({
            "key": "n-ledger",
            "cls": "decision",
            "source_kind": "decision",
            "source_ts": "2026-08-04T09:00:00.000Z",
            "text": "Decision: ember quartz flint cobalt onyx basalt garnet are catalogued in the survey ledger"
        });
        let path_one = json!({
            "key": "n-path-1",
            "cls": "path",
            "source_kind": "count",
            "source_ts": "2026-08-03T09:00:00.000Z",
            "text": "path src/harvest.rs ember ember quartz quartz flint flint cobalt cobalt onyx onyx basalt basalt garnet garnet pyrite pyrite mentions"
        });
        let path_two = json!({
            "key": "n-path-2",
            "cls": "path",
            "source_kind": "count",
            "source_ts": "2026-08-03T10:00:00.000Z",
            "text": "path src/render.rs ember ember quartz quartz flint flint cobalt cobalt onyx onyx basalt basalt garnet garnet pyrite pyrite mentions"
        });
        write_newest_run_notes(
            &root,
            &format!(
                "{}\n{}\n{}\n{}",
                fixture_notes(),
                decision,
                path_one,
                path_two
            ),
        );
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet",
            false,
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("note answer");
        assert_eq!(answer.layer, "note");
        assert_eq!(answer.key, "n-ledger");
        assert!(!out
            .recalled
            .iter()
            .any(|recall| recall.cls == "path" || recall.key.starts_with("n-path")));
        assert!(out
            .related
            .iter()
            .any(|rel| rel.cls == "path" && rel.text.contains("src/harvest.rs")));
        assert_eq!(out.related.len(), RELATED_LIMIT);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_answers_from_an_assistant_unit_and_recalls_it_as_a_claim() {
        let root = temp_root("assistant-unit");
        let reply = json!({
            "key": "a-1",
            "kind": "assistant",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz"
        });
        let fillers = [
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            "anchor beacon current estuary fathom gulf harbor inlet jetty keel lagoon mooring narrows",
            "binder canvas dowel easel fillet gauge hinge jamb knob latch miter notch overlay paste",
            "aurora basin canyon dell escarpment fen gorge hollow karst ledge mesa outcrop plateau rise",
        ];
        let mut lines = vec![reply.to_string()];
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
        let answer = out.answer.as_ref().expect("unit answer");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.source_kind, "assistant");
        assert_eq!(
            out.reason,
            "units layer answer (assistant reply), confidence capped medium"
        );
        let recalled = out
            .recalled
            .iter()
            .find(|recall| recall.key == "a-1")
            .expect("assistant unit recalled");
        assert_eq!(recalled.layer.as_deref(), Some("unit"));
        assert_eq!(recalled.cls, "assistant");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_keeps_the_margin_gate_for_assistant_units() {
        let root = temp_root("assistant-margin");
        let top_reply = json!({
            "key": "a-1",
            "kind": "assistant",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz were catalogued during the spring mineralogy survey"
        });
        let rival_reply = json!({
            "key": "a-2",
            "kind": "assistant",
            "ts": "2026-08-01T08:00:00.000Z",
            "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz got auctioned at our autumn gemstone exchange yesterday"
        });
        assert!(
            text::token_jaccard(
                top_reply["text"].as_str().expect("top text"),
                rival_reply["text"].as_str().expect("rival text")
            ) < AGREE_JACCARD
        );
        let mut lines = vec![top_reply.to_string(), rival_reply.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "candidates");
        assert_eq!(out.answer, None);
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(margin < Gates::DEFAULTS.unit_margin);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_answers_three_agreeing_paraphrase_captures_with_one_clear_top() {
        let root = temp_root("paraphrase-trio");
        let top_text =
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz";
        let second_text = format!("the {top_text} crystals were catalogued");
        let third_text = format!("{top_text} got catalogued today");
        assert!(text::token_jaccard(top_text, &second_text) >= AGREE_JACCARD);
        assert!(text::token_jaccard(top_text, &third_text) >= AGREE_JACCARD);
        assert!(text::token_jaccard(&second_text, &third_text) >= AGREE_JACCARD);
        let top = json!({
            "key": "u-top",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": top_text
        });
        let second = json!({
            "key": "u-p1",
            "kind": "user",
            "ts": "2026-08-01T08:30:00.000Z",
            "text": second_text
        });
        let third = json!({
            "key": "u-p2",
            "kind": "user",
            "ts": "2026-08-01T08:00:00.000Z",
            "text": third_text
        });
        let mut lines = vec![top.to_string(), second.to_string(), third.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("unit answer");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.key, "u-top");
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(margin >= Gates::DEFAULTS.unit_margin);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ask_answers_when_a_weak_unrelated_unit_is_the_only_competitor() {
        let root = temp_root("agree-weak");
        let top_text =
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz";
        let agree_text = format!("the {top_text} crystals were catalogued");
        let weak_text = "ember lantern drift cobalt";
        assert!(text::token_jaccard(top_text, &agree_text) >= AGREE_JACCARD);
        assert!(text::token_jaccard(top_text, weak_text) < AGREE_JACCARD);
        let top = json!({
            "key": "u-top",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": top_text
        });
        let agree = json!({
            "key": "u-agree",
            "kind": "user",
            "ts": "2026-08-01T08:30:00.000Z",
            "text": agree_text
        });
        let weak = json!({
            "key": "u-weak",
            "kind": "user",
            "ts": "2026-08-01T08:00:00.000Z",
            "text": weak_text
        });
        let mut lines = vec![top.to_string(), agree.to_string(), weak.to_string()];
        lines.extend(margin_fillers());
        write_units(&root, &lines.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.verdict, "answered");
        let answer = out.answer.as_ref().expect("unit answer");
        assert_eq!(answer.layer, "unit");
        assert_eq!(answer.key, "u-top");
        let margin = out.signals.unit_margin.expect("unit margin signal");
        assert!(margin.is_finite());
        assert!(margin >= Gates::DEFAULTS.unit_margin);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recalled_merges_claim_notes_and_units_score_ordered_and_caps_at_eight() {
        let root = temp_root("recalled-merge");
        let mut lines: Vec<String> = Vec::new();
        for index in 0..6usize {
            lines.push(
                json!({
                    "key": format!("a-{index:02}"),
                    "kind": "assistant",
                    "ts": "2026-08-01T09:00:00.000Z",
                    "text": format!(
                        "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz reply {index:02}"
                    )
                })
                .to_string(),
            );
        }
        lines.push(
            json!({
                "key": "a-off",
                "kind": "assistant",
                "ts": "2026-08-01T09:00:00.000Z",
                "text": "nothing remotely relevant lives here"
            })
            .to_string(),
        );
        write_units(&root, &lines.join("\n"));
        let mut rows: Vec<String> = Vec::new();
        for index in 0..6usize {
            rows.push(
                json!({
                    "key": format!("n-d-{index}"),
                    "cls": "decision",
                    "source_kind": "decision",
                    "source_ts": "2026-08-04T09:00:00.000Z",
                    "text": format!(
                        "Decision: ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz ledger {index}"
                    )
                })
                .to_string(),
            );
        }
        write_newest_run_notes(&root, &rows.join("\n"));
        let store = Store::resolve(Some(&root)).expect("store resolves");
        let out = run_ask(
            &store,
            "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            false,
        );
        assert_eq!(out.recalled.len(), RECALLED_LIMIT);
        let mut previous = f64::INFINITY;
        for recall in &out.recalled {
            assert!(recall.score <= previous);
            previous = recall.score;
            assert!(matches!(
                recall.layer.as_deref(),
                Some("note") | Some("unit")
            ));
        }
        assert!(out
            .recalled
            .iter()
            .any(|recall| recall.layer.as_deref() == Some("note")));
        assert!(out
            .recalled
            .iter()
            .any(|recall| recall.layer.as_deref() == Some("unit")));
        assert!(!out.recalled.iter().any(|recall| recall.key == "a-off"));
        fs::remove_dir_all(&root).ok();
    }

    fn render_fixture(answer_host: Value, recalled_host: Value) -> AskOutput {
        serde_json::from_value(json!({
            "query": "does the clipboard ring survive restarts",
            "verdict": "answered",
            "confidence": "high",
            "reason": "notes layer decision answer, margin 9x",
            "outcome": "supported",
            "reason_code": "notes_answer",
            "gates": {
                "NO_MEMORY_COV": 0.5,
                "FLOOR": 6.0,
                "NOTE_COV": 0.5,
                "NOTE_SCORE": 6.0,
                "NOTE_MARGIN": 1.25,
                "UNIT_COV": 1.0,
                "UNIT_SCORE": 8.0,
                "UNIT_MARGIN": 1.5,
                "HIGH_MARGIN": 1.8
            },
            "non_default_gates": false,
            "answer": {
                "text": "the clipboard ring survives tray restarts",
                "layer": "note",
                "key": "n-1",
                "cls": "decision",
                "source_kind": "decision",
                "source_ts": "2026-08-04T08:00:00.000Z",
                "score": 14.0,
                "margin": 9.0,
                "host": answer_host
            },
            "recalled": [
                {
                    "key": "n-1",
                    "cls": "decision",
                    "score": 14.0,
                    "host": recalled_host
                }
            ],
            "related": [],
            "signals": {
                "top_note_score": 14.0,
                "top_unit_score": null,
                "unit_margin": null,
                "note_token_coverage": 0.8,
                "unit_token_coverage": 0.0,
                "max_token_coverage": 0.8,
                "notes_run_ts": "2026-08-05T10-00-00-000Z",
                "snapshot_run_ts": "live",
                "live_units": true,
                "stale_layer": false,
                "recency_resolved": null
            },
            "counts": { "units": 1, "notes": 1 },
            "skills": { "status": "served", "hits": [] },
            "notes": []
        }))
        .expect("render fixture parses into AskOutput")
    }

    #[test]
    fn render_text_labels_an_answer_from_a_foreign_host() {
        let foreign = "remote-box/macos".to_string();
        let out = render_fixture(json!(foreign), json!(foreign));
        let text = render_text(&out);
        let expected = format!(" [from {foreign}]");
        assert!(text.contains(&format!(
            "answer [note/decision]: the clipboard ring survives tray restarts{expected}"
        )));
        assert!(text.contains(&format!("  n-1  decision  14{expected}")));
    }

    #[test]
    fn render_text_omits_the_suffix_when_the_host_is_local_or_absent() {
        let local = crate::host::current().to_string();
        let out = render_fixture(json!(local), Value::Null);
        let text = render_text(&out);
        assert!(!text.contains(" [from "));
        assert!(
            text.contains("answer [note/decision]: the clipboard ring survives tray restarts\n")
        );
        assert!(text.ends_with("  n-1  decision  14"));
    }
}
