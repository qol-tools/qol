use std::collections::HashSet;

use super::question_match::{self, Evidence, Question};
use crate::store::Unit;

pub(super) struct Candidate<'a> {
    pub unit: &'a Unit,
    pub evidence: Evidence,
}

#[derive(Default)]
pub(super) struct Selection<'a> {
    pub winner: Option<Candidate<'a>>,
    pub supporting_keys: Vec<String>,
    pub matching: usize,
    pub conflicts: usize,
}

pub(super) fn select<'a>(
    query: &str,
    units: &'a [Unit],
    excluded_session: Option<&str>,
    disliked: Option<&HashSet<String>>,
) -> Selection<'a> {
    let Some(query) = Question::parse(query).or_else(|| Question::shorthand(query)) else {
        return Selection::default();
    };
    let candidates = units
        .iter()
        .filter(|unit| unit.kind == crate::ingest::CAPTURE_KIND)
        .filter(|unit| {
            excluded_session.is_none_or(|session| unit.session.as_deref() != Some(session))
        })
        .filter(|unit| disliked.is_none_or(|keys| !keys.contains(&unit.key)))
        .filter_map(|unit| {
            question_match::evidence(&unit.text).map(|evidence| Candidate { unit, evidence })
        })
        .collect::<Vec<_>>();
    let matched = candidates
        .iter()
        .filter(|candidate| candidate.evidence.question.covers(&query))
        .collect::<Vec<_>>();
    if matched.is_empty() {
        return Selection::default();
    }
    let relevant = candidates
        .iter()
        .filter(|candidate| {
            matched.iter().any(|seed| {
                candidate
                    .evidence
                    .question
                    .same_subject(&seed.evidence.question)
            })
        })
        .collect::<Vec<_>>();
    let has_conflict = relevant.iter().enumerate().any(|(i, left)| {
        relevant
            .iter()
            .skip(i + 1)
            .any(|right| !question_match::agree(&left.evidence.answer, &right.evidence.answer))
    });
    if has_conflict {
        return Selection {
            matching: relevant.len(),
            conflicts: relevant.len(),
            ..Selection::default()
        };
    }
    let mut supporting_keys = relevant
        .iter()
        .map(|candidate| candidate.unit.key.clone())
        .collect::<Vec<_>>();
    supporting_keys.sort();
    supporting_keys.dedup();
    let winner = relevant
        .iter()
        .min_by_key(|candidate| &candidate.unit.key)
        .unwrap();
    Selection {
        winner: Some(Candidate {
            unit: winner.unit,
            evidence: winner.evidence.clone(),
        }),
        matching: relevant.len(),
        conflicts: 0,
        supporting_keys,
    }
}
