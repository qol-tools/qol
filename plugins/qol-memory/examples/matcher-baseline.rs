use std::path::Path;
mod support;
use qol_memory::verification::Fact;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use support::{call, fixture_store};

use anyhow::{bail, Context, Result};
use qol_memory::app::warm::WarmState;
use qol_memory::ask::{self, AskRequest};
use qol_memory::retrieval::{bm25_ranks, build_index, DocRef};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Input {
    facts: Vec<Fact>,
    queries: Vec<Query>,
    repeats: usize,
}

#[derive(Deserialize)]
struct Query {
    id: String,
    query: String,
}

fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .context("expected a new fixture store path")?;
    let input: Input = serde_json::from_reader(std::io::stdin())?;
    if input.facts.is_empty() || input.queries.is_empty() || !(1..=10).contains(&input.repeats) {
        bail!("nonempty fixtures and 1..=10 repeats are required");
    }
    let caller = qol_agent_homes::Registry::load().resolve_caller(None);
    let store = fixture_store(Path::new(&root), &input.facts, &caller)?;
    let started = Instant::now();
    let mut state = Arc::new(Mutex::new(WarmState::open(
        store.clone(),
        qol_memory::aliases::embedded(),
    )?));
    call(&mut state, "rows", &input.queries[0].query, &caller)?;
    let setup_ms = started.elapsed().as_secs_f64() * 1000.0;
    let docs = input
        .facts
        .iter()
        .map(|fact| DocRef {
            key: &fact.id,
            text: &fact.question,
        })
        .collect::<Vec<_>>();
    let index = build_index(&docs);
    let mut results = Vec::new();
    for query in &input.queries {
        let start = Instant::now();
        let lexical = bm25_ranks(&query.query, &index, input.facts.len())
            .into_iter()
            .map(|row| row.key)
            .collect::<Vec<_>>();
        let lexical_ms = start.elapsed().as_secs_f64() * 1000.0;
        let cold = ask::run(
            &store,
            &qol_memory::aliases::embedded(),
            &AskRequest {
                query: query.query.clone(),
                k: 5,
                brief: false,
                exclude_session: None,
                agent_home: Some(caller.clone()),
            },
        )?;
        let answer = cold.answer.as_ref().map(|answer| answer.key.as_str());
        let mut samples = Vec::new();
        for _ in 0..input.repeats {
            let warm = call(&mut state, "ask", &query.query, &caller)?;
            let start = Instant::now();
            let rows = call(&mut state, "rows", &query.query, &caller)?;
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            let shown = rows["rows"]
                .as_array()
                .context("missing rows")?
                .iter()
                .find(|row| row["kind"] == "answer")
                .and_then(|row| row["key"].as_str());
            if warm["answer"]["key"].as_str() != answer
                || shown != answer
                || rows["verdict"] != cold.verdict
            {
                bail!(
                    "cold ask, warm ask and launcher rows disagree for {}",
                    query.id
                );
            }
        }
        results.push(json!({"id":query.id,"answer":answer,"samples_ms":samples,"lexical":lexical,"lexical_ms":lexical_ms,"verdict":cold.verdict,"reason":cold.reason}));
    }
    println!(
        "{}",
        serde_json::to_string(&json!({"setup_ms":setup_ms,"results":results}))?
    );
    Ok(())
}
