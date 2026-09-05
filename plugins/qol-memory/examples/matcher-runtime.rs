use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_memory::app::warm::WarmState;
use qol_memory::verification::{ollama::Ollama, Fact};
use qol_memory::verification::{service::Verifier, Prediction};
use serde::Deserialize;
use serde_json::json;

mod support;

struct Recorded {
    provider: Ollama,
    observations: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Verifier for Recorded {
    fn identity(&self) -> &str {
        self.provider.identity()
    }
    fn verify(&mut self, query: &str, facts: &[Fact]) -> Result<Prediction> {
        let prediction = self.provider.verify(query, facts)?;
        self.observations
            .lock()
            .unwrap()
            .push(json!({"query":query,"facts":facts,"prediction":prediction}));
        Ok(prediction)
    }
}

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
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().context("new fixture directory required")?);
    let endpoint = args.next().context("loopback model endpoint required")?;
    let input: Input = serde_json::from_reader(std::io::stdin())?;
    if input.facts.is_empty() || input.queries.is_empty() || !(1..=3).contains(&input.repeats) {
        bail!("nonempty fixtures and 1..=3 repeats are required");
    }
    std::fs::create_dir(&root)?;
    let caller = qol_agent_homes::Registry::load().resolve_caller(None);
    let mut rounds = Vec::new();
    let observations = Arc::new(Mutex::new(Vec::new()));
    for repeat in 0..input.repeats {
        let store = support::fixture_store(&root.join(repeat.to_string()), &input.facts, &caller)?;
        let provider = Ollama::new(store.root().join("provider"), &endpoint)?;
        let mut warm = WarmState::open(store, qol_memory::aliases::embedded())?;
        warm.enable_verification(Recorded {
            provider,
            observations: Arc::clone(&observations),
        })?;
        let mut state = Arc::new(Mutex::new(warm));
        let mut results = Vec::new();
        for query in &input.queries {
            let start = Instant::now();
            let mut rows = support::call(&mut state, "rows", &query.query, &caller)?;
            let initial_ms = start.elapsed().as_secs_f64() * 1000.0;
            let pending = rows["verification"]["status"] == "pending";
            while rows["verification"]["status"] == "pending" {
                if start.elapsed() > Duration::from_secs(60) {
                    bail!("background verification timed out for {}", query.id);
                }
                std::thread::sleep(Duration::from_millis(25));
                rows = support::call(&mut state, "rows", &query.query, &caller)?;
            }
            if rows["verification"]["status"] == "unavailable" {
                bail!("runtime verification unavailable for {}", query.id);
            }
            let completion_ms = start.elapsed().as_secs_f64() * 1000.0;
            let answer = rows["rows"]
                .as_array()
                .context("rows array")?
                .iter()
                .find(|row| row["kind"] == "answer")
                .and_then(|row| row["key"].as_str())
                .map(str::to_owned);
            let mut samples = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                let ask = support::call(&mut state, "ask", &query.query, &caller)?;
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
                if ask["answer"]["key"].as_str() != answer.as_deref()
                    || ask["verification"]["status"] == "pending"
                {
                    bail!("cached ask and launcher rows disagree for {}", query.id);
                }
            }
            results.push(json!({"id":query.id,"answer":answer,"samples_ms":samples,"initial_ms":initial_ms,"completion_ms":completion_ms,"pending":pending}));
            qol_fs::atomic_write(
                &root.join(format!("round-{repeat}.json")),
                &serde_json::to_vec(
                    &json!({"results":results,"observations":*observations.lock().unwrap()}),
                )?,
            )?;
        }
        rounds.push(results);
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"rounds":rounds,"observations":*observations.lock().unwrap()})
        )?
    );
    Ok(())
}
