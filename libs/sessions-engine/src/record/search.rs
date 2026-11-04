use std::path::PathBuf;

use qol_terminal_sessions::cli::search_transcripts;
use serde::Serialize;

use super::store::{Done, Record, RecordStore};

#[derive(Debug, Serialize)]
pub(crate) struct RecordPart {
    pub(crate) n: u32,
    pub(crate) agent_id: String,
    pub(crate) archive: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecordRow {
    pub(crate) key: String,
    pub(crate) friendly: String,
    pub(crate) cwd: String,
    pub(crate) created_ms: u64,
    pub(crate) parts: Vec<RecordPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) done: Option<Done>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stopped_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) excerpt: Option<String>,
}

fn matches_query(record: &Record, query: &str) -> bool {
    [&record.key, &record.friendly, &record.cwd]
        .iter()
        .any(|field| field.to_lowercase().contains(query))
}

fn archived(store: &RecordStore, record: &Record) -> Vec<(u32, PathBuf)> {
    record
        .parts
        .iter()
        .map(|part| (part.n, store.archive_path(&record.key, part.n)))
        .filter(|(_, path)| path.is_file())
        .collect()
}

fn excerpt(store: &RecordStore, record: &Record, terms: &[String]) -> Option<String> {
    let files = archived(store, record);
    let mut found = search_transcripts(files.into_iter().map(|(_, path)| path).collect(), terms);
    found.sort_by(|left, right| left.external_id.cmp(&right.external_id));
    found.into_iter().next().map(|found| found.excerpt)
}

fn row(store: &RecordStore, record: Record, excerpt: Option<String>) -> RecordRow {
    let parts = record
        .parts
        .iter()
        .map(|part| {
            let path = store.archive_path(&record.key, part.n);
            RecordPart {
                n: part.n,
                agent_id: part.agent_id.clone(),
                archive: path.is_file().then(|| path.display().to_string()),
            }
        })
        .collect();
    RecordRow {
        key: record.key,
        friendly: record.friendly,
        cwd: record.cwd,
        created_ms: record.created_ms,
        parts,
        done: record.done,
        stopped_ms: record.stopped_ms,
        excerpt,
    }
}

pub(crate) fn rows(
    store: &RecordStore,
    query: Option<&str>,
    terms: Option<&[String]>,
    limit: usize,
) -> Result<Vec<RecordRow>, String> {
    let records = store
        .list()
        .map_err(|error| format!("failed to read the session records: {error}"))?;
    let mut rows = Vec::new();
    for record in records {
        if rows.len() >= limit {
            break;
        }
        if query.is_some_and(|query| !matches_query(&record, query)) {
            continue;
        }
        let found = match terms {
            Some(terms) => match excerpt(store, &record, terms) {
                Some(found) => Some(found),
                None => continue,
            },
            None => None,
        };
        rows.push(row(store, record, found));
    }
    Ok(rows)
}
