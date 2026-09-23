use std::collections::{BTreeMap, HashMap};

use sha2::{Digest, Sha256};

use super::{question_match, selection, Answer, AskOutput, AskRequest};
use crate::retrieval::{bm25_ranks, build_index, DocRef};
use crate::store::{Store, Unit, UnitsLayer};
use crate::verification::service::{Job, Service, Status};
use crate::verification::{Decision, Fact};

const CANDIDATE_LIMIT: usize = 8;

pub struct Snapshot {
    job: Job,
    answers: HashMap<String, Answer>,
}

impl Snapshot {
    pub fn prepare(
        store: &Store,
        request: &AskRequest,
        units: &UnitsLayer,
        source: &str,
    ) -> Option<Self> {
        let terms = crate::text::tokens(&request.query)
            .into_iter()
            .filter(|term| !super::stopword_set().contains(term.as_str()))
            .collect::<Vec<_>>();
        if terms.len() < 2 {
            return None;
        }
        let registry = qol_agent_homes::Registry::load();
        let caller = registry.resolve_caller(request.agent_home.as_deref());
        let disliked = crate::feedback::disliked_by_norm(store.root());
        let norm = crate::retrieval_log::normalize_query(&request.query);
        let excluded = disliked.get(&norm);
        let visible = units
            .items
            .iter()
            .filter(|unit| {
                unit.kind == crate::ingest::CAPTURE_KIND
                    && !crate::store::is_boilerplate_unit(unit)
                    && crate::agent_home::visible(unit, &caller, &registry)
                    && request
                        .exclude_session
                        .as_deref()
                        .is_none_or(|session| unit.session.as_deref() != Some(session))
                    && excluded.is_none_or(|keys| !keys.contains(&unit.key))
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut evidence = Vec::new();
        let mut declarative = Vec::new();
        for unit in &visible {
            match question_match::evidence(&unit.text) {
                Some(found) => evidence.push((unit, found)),
                None if unit.text.chars().count() <= 700 => declarative.push(unit),
                None => {}
            }
        }
        let mut docs = evidence
            .iter()
            .map(|(unit, found)| {
                (
                    unit.key.as_str(),
                    format!("{} {}", found.recorded_question, found.recorded_answer),
                )
            })
            .collect::<Vec<_>>();
        docs.extend(
            declarative
                .iter()
                .map(|unit| (unit.key.as_str(), unit.text.clone())),
        );
        let doc_refs = docs
            .iter()
            .map(|(key, text)| DocRef { key, text })
            .collect::<Vec<_>>();
        let ranks = bm25_ranks(&terms.join(" "), &build_index(&doc_refs), CANDIDATE_LIMIT);
        let mut facts = BTreeMap::new();
        let mut kept = Vec::new();
        let mut answers = HashMap::new();
        for ranked in ranks {
            let (key, fact, entry) = if let Some((_, found)) =
                evidence.iter().find(|(unit, _)| unit.key == ranked.key)
            {
                let group = selection::select(&found.recorded_question, &visible, None, None);
                if group.conflicts > 0 {
                    continue;
                }
                let Some(winner) = group.winner else {
                    continue;
                };
                let key = winner.unit.key.clone();
                let fact = Fact {
                    id: key.clone(),
                    question: winner.evidence.recorded_question,
                    answer: winner.evidence.recorded_answer,
                };
                let entry = answer(winner.unit, winner.evidence.display, group.supporting_keys);
                (key, fact, entry)
            } else {
                let unit = declarative.iter().find(|unit| unit.key == ranked.key)?;
                let key = unit.key.clone();
                let fact = Fact {
                    id: key.clone(),
                    question: String::new(),
                    answer: unit.text.trim().to_owned(),
                };
                let entry = answer(unit, unit.text.to_owned(), Vec::new());
                (key, fact, entry)
            };
            kept.push(fact.clone());
            if prompt_bytes(&request.query, &kept)
                > crate::verification::profile().context_byte_limit
            {
                kept.pop();
                if kept.is_empty() {
                    return None;
                }
                break;
            }
            facts.insert(key.clone(), fact);
            answers.insert(key, entry);
        }
        if facts.is_empty() {
            return None;
        }
        let mut revision = visible
            .iter()
            .map(|unit| {
                (
                    &unit.key,
                    &unit.text,
                    &unit.agent_home,
                    &unit.cwd,
                    &unit.session,
                    &unit.ts,
                )
            })
            .collect::<Vec<_>>();
        revision.sort_by(|left, right| left.0.cmp(right.0));
        let context = serde_json::json!([caller, request.exclude_session, revision]).to_string();
        Some(Self {
            job: Job {
                query: request.query.clone(),
                facts: facts.into_values().collect(),
                context: format!("{:x}", Sha256::digest(context.as_bytes())),
                lane: (source == "launcher").then(|| format!("launcher:{caller}")),
            },
            answers,
        })
    }

    pub fn apply(self, service: &Service, output: &mut AskOutput) {
        let status = service.query(self.job);
        if let Status::Ready(Decision::Accepted(key)) = &status {
            if let Some(answer) = self.answers.get(key) {
                output.answer = Some(answer.clone());
                output.verdict = "answered".to_owned();
                output.confidence = "medium".to_owned();
                output.outcome = super::Outcome::Supported;
                output.reason_code = super::Reason::VerifiedAnswer;
                output.reason =
                    "recorded answer verified against the question; confidence capped medium"
                        .to_owned();
            }
        }
        output.verification = Some(status);
    }
}

fn prompt_bytes(query: &str, facts: &[Fact]) -> usize {
    crate::verification::request(&crate::verification::profile().model, query, facts)["prompt"]
        .as_str()
        .map_or(0, str::len)
}

fn answer(unit: &Unit, text: String, supporting_keys: Vec<String>) -> Answer {
    Answer {
        text,
        key: unit.key.clone(),
        layer: "unit".into(),
        cls: None,
        source_kind: unit.kind.clone(),
        source_ts: unit.ts.clone(),
        session: unit.session.clone(),
        host: unit.host.clone(),
        score: 0.0,
        margin: None,
        superseded: None,
        supporting_keys,
    }
}
