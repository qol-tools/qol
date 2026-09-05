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
        let evidence = visible
            .iter()
            .filter_map(|unit| {
                question_match::evidence(&unit.text).map(|evidence| (unit, evidence))
            })
            .collect::<Vec<_>>();
        let docs = evidence
            .iter()
            .map(|(unit, evidence)| DocRef {
                key: &unit.key,
                text: &evidence.recorded_question,
            })
            .collect::<Vec<_>>();
        let ranks = bm25_ranks(&terms.join(" "), &build_index(&docs), CANDIDATE_LIMIT);
        let mut facts = BTreeMap::new();
        let mut answers = HashMap::new();
        for ranked in ranks {
            let (_, record) = evidence.iter().find(|(unit, _)| unit.key == ranked.key)?;
            let group = selection::select(&record.recorded_question, &visible, None, None);
            if group.conflicts > 0 {
                return None;
            }
            let winner = group.winner?;
            let key = winner.unit.key.clone();
            facts.insert(
                key.clone(),
                Fact {
                    id: key.clone(),
                    question: winner.evidence.recorded_question,
                    answer: winner.evidence.recorded_answer,
                },
            );
            answers.insert(
                key,
                answer(winner.unit, winner.evidence.display, group.supporting_keys),
            );
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
                output.reason =
                    "recorded answer verified against the question; confidence capped medium"
                        .to_owned();
            }
        }
        output.verification = Some(status);
    }
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
