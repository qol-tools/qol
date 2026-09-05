use anyhow::{bail, Result};
use qol_memory::verification::{self, Fact, Prediction};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    model: String,
    facts: Vec<Fact>,
    queries: Vec<Query>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    query: String,
    prediction: Option<Prediction>,
}

fn main() -> Result<()> {
    let input: Input = serde_json::from_reader(std::io::stdin())?;
    if input.facts.is_empty() || input.queries.is_empty() {
        bail!("nonempty facts and queries are required");
    }
    let results = input.queries.iter().map(|query| {
        let decision = query.prediction.as_ref().map(|prediction| verification::check(&query.query, &input.facts, prediction));
        json!({"id": query.id, "request": verification::request(&input.model, &query.query, &input.facts), "decision": decision})
    }).collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&results)?);
    Ok(())
}
