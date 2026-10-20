use std::collections::HashSet;

use super::question_match::{self, Evidence, Question};
use crate::lifecycle::Lifecycle;
use crate::store::Unit;

#[derive(Clone)]
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
    pub covered: Vec<Candidate<'a>>,
}

pub(super) fn select<'a>(
    query: &str,
    units: &'a [Unit],
    excluded_session: Option<&str>,
    disliked: Option<&HashSet<String>>,
    lifecycle: &Lifecycle,
) -> Selection<'a> {
    let found = relevant(query, units, excluded_session, disliked, lifecycle);
    let present = found
        .iter()
        .map(|candidate| candidate.unit.key.clone())
        .collect::<HashSet<_>>();
    decide(found, lifecycle, &present)
}

pub(super) fn relevant<'a>(
    query: &str,
    units: &'a [Unit],
    excluded_session: Option<&str>,
    disliked: Option<&HashSet<String>>,
    lifecycle: &Lifecycle,
) -> Vec<Candidate<'a>> {
    let Some(query) = Question::parse(query).or_else(|| Question::shorthand(query)) else {
        return Vec::new();
    };
    let candidates = units
        .iter()
        .filter(|unit| unit.kind == crate::ingest::CAPTURE_KIND)
        .filter(|unit| {
            excluded_session.is_none_or(|session| unit.session.as_deref() != Some(session))
        })
        .filter(|unit| disliked.is_none_or(|keys| !keys.contains(&unit.key)))
        .filter(|unit| !lifecycle.is_forgotten(&unit.key))
        .filter_map(|unit| {
            question_match::evidence(&unit.text).map(|evidence| Candidate { unit, evidence })
        })
        .collect::<Vec<_>>();
    let matched = candidates
        .iter()
        .filter(|candidate| candidate.evidence.question.covers(&query))
        .collect::<Vec<_>>();
    let keep = candidates
        .iter()
        .map(|candidate| {
            matched.iter().any(|seed| {
                candidate
                    .evidence
                    .question
                    .same_subject(&seed.evidence.question)
            })
        })
        .collect::<Vec<_>>();
    candidates
        .into_iter()
        .zip(keep)
        .filter_map(|(candidate, keep)| keep.then_some(candidate))
        .collect()
}

pub(super) fn decide<'a>(
    relevant: Vec<Candidate<'a>>,
    lifecycle: &Lifecycle,
    present: &HashSet<String>,
) -> Selection<'a> {
    let (covered, rest): (Vec<_>, Vec<_>) = relevant.into_iter().partition(|candidate| {
        lifecycle
            .live_replacement(&candidate.unit.key)
            .is_some_and(|by| present.contains(by))
    });
    let any_live = rest
        .iter()
        .any(|candidate| lifecycle.is_live(&candidate.unit.key));
    let relevant = rest
        .into_iter()
        .filter(|candidate| !any_live || lifecycle.is_live(&candidate.unit.key))
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return Selection {
            covered,
            ..Selection::default()
        };
    }
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
            covered,
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
        .cloned();
    Selection {
        winner,
        matching: relevant.len(),
        conflicts: 0,
        supporting_keys,
        covered,
    }
}
