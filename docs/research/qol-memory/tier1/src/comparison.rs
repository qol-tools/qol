use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{bert, xlm_roberta};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

use crate::embedding::{dot, embed_text};

const SHORTLIST: usize = 3;
const RRF_OFFSET: f64 = 60.0;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    facts: Vec<Fact>,
    queries: Vec<Query>,
    repeats: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fact {
    id: String,
    question: String,
    answer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    query: String,
    lexical: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Score {
    key: String,
    score: f64,
}

#[derive(Serialize)]
pub struct Output {
    setup_ms: f64,
    index_ms: f64,
    embedding: Vec<Row>,
    hybrid_equivalence: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    id: String,
    scores: Vec<Score>,
    samples_ms: Vec<f64>,
    retrieved: Vec<String>,
}

struct Models {
    embedding: bert::BertModel,
    embedding_tokens: Tokenizer,
    equivalence: xlm_roberta::XLMRobertaForSequenceClassification,
    equivalence_tokens: Tokenizer,
}

impl Models {
    fn load(embedding: &Path, equivalence: &Path) -> Result<Self> {
        let bert_config = serde_json::from_slice(&std::fs::read(embedding.join("config.json"))?)?;
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(equivalence.join("config.json"))?)?;
        if config["model_type"] != "roberta"
            || config["architectures"][0] != "RobertaForSequenceClassification"
        {
            bail!("expected the pinned RoBERTa question-equivalence classifier");
        }
        config["position_embedding_type"] = serde_json::json!("absolute");
        Ok(Self {
            embedding: bert::BertModel::load(weights(embedding)?, &bert_config)?,
            embedding_tokens: tokenizer(embedding)?,
            equivalence: xlm_roberta::XLMRobertaForSequenceClassification::new(
                1,
                &serde_json::from_value(config)?,
                weights(equivalence)?,
            )?,
            equivalence_tokens: tokenizer(equivalence)?,
        })
    }

    fn embed(&self, text: &str) -> Result<Tensor> {
        embed_text(&self.embedding, &self.embedding_tokens, text, &Device::Cpu)
    }

    fn equivalent(&self, left: &str, right: &str) -> Result<f64> {
        let encoded = self
            .equivalence_tokens
            .encode((left, right), true)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        if encoded.len() > 512 {
            bail!("question pair exceeds the classifier context");
        }
        let ids = Tensor::new(encoded.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
        let mask = Tensor::new(encoded.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
        let logit = self
            .equivalence
            .forward(&ids, &mask, &ids.zeros_like()?)?
            .flatten_all()?
            .to_vec1::<f32>()?[0];
        Ok(f64::from(1.0 / (1.0 + (-logit).exp())))
    }
}

pub fn run(input: Input, embedding: &Path, equivalence: &Path) -> Result<Output> {
    if input.facts.is_empty() || input.queries.is_empty() || !(1..=10).contains(&input.repeats) {
        bail!("nonempty fixtures and 1..=10 repeats are required");
    }
    if input.facts.iter().any(|fact| fact.answer.is_empty()) {
        bail!("facts must carry recorded answers");
    }
    let start = Instant::now();
    let models = Models::load(embedding, equivalence)?;
    models.embed("model warmup")?;
    models.equivalent("How to start the service?", "How to launch the service?")?;
    let setup_ms = elapsed_ms(start);
    let start = Instant::now();
    let vectors = input
        .facts
        .iter()
        .map(|fact| models.embed(&fact.question))
        .collect::<Result<Vec<_>>>()?;
    let index_ms = elapsed_ms(start);
    let mut output = Output {
        setup_ms,
        index_ms,
        embedding: Vec::new(),
        hybrid_equivalence: Vec::new(),
    };
    for query in &input.queries {
        let mut dense = Row {
            id: query.id.clone(),
            scores: Vec::new(),
            samples_ms: Vec::new(),
            retrieved: Vec::new(),
        };
        let mut paired = Row {
            id: query.id.clone(),
            scores: Vec::new(),
            samples_ms: Vec::new(),
            retrieved: Vec::new(),
        };
        for _ in 0..input.repeats {
            let start = Instant::now();
            let vector = models.embed(&query.query)?;
            let scores = input
                .facts
                .iter()
                .zip(&vectors)
                .map(|(fact, other)| {
                    Ok(Score {
                        key: fact.id.clone(),
                        score: f64::from(dot(&vector, other)?),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            dense.samples_ms.push(elapsed_ms(start));
            let keys = shortlist(&scores, &query.lexical);
            let mut equivalent = Vec::new();
            for key in &keys {
                let fact = input
                    .facts
                    .iter()
                    .find(|fact| fact.id == *key)
                    .context("unknown shortlisted fact")?;
                equivalent.push(Score {
                    key: key.clone(),
                    score: models.equivalent(&query.query, &fact.question)?,
                });
            }
            paired.samples_ms.push(elapsed_ms(start));
            dense.retrieved = ranked_keys(&scores).into_iter().take(SHORTLIST).collect();
            dense.scores = scores;
            paired.retrieved = keys;
            paired.scores = equivalent;
        }
        output.embedding.push(dense);
        output.hybrid_equivalence.push(paired);
    }
    Ok(output)
}

fn ranked_keys(scores: &[Score]) -> Vec<String> {
    let mut ranked = scores.to_vec();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.key.cmp(&right.key))
    });
    ranked.into_iter().map(|score| score.key).collect()
}

fn shortlist(scores: &[Score], lexical: &[String]) -> Vec<String> {
    let mut combined: HashMap<String, f64> = HashMap::new();
    for ranking in [ranked_keys(scores), lexical.to_vec()] {
        for (rank, key) in ranking.into_iter().enumerate() {
            *combined.entry(key).or_default() += 1.0 / (RRF_OFFSET + rank as f64 + 1.0);
        }
    }
    let scores = combined
        .into_iter()
        .map(|(key, score)| Score { key, score })
        .collect::<Vec<_>>();
    ranked_keys(&scores).into_iter().take(SHORTLIST).collect()
}

fn weights(path: &Path) -> Result<VarBuilder<'static>> {
    Ok(VarBuilder::from_buffered_safetensors(
        std::fs::read(path.join("model.safetensors"))?,
        DType::F32,
        &Device::Cpu,
    )?)
}

fn tokenizer(path: &Path) -> Result<Tokenizer> {
    Tokenizer::from_file(path.join("tokenizer.json"))
        .map_err(|error| anyhow::anyhow!("tokenizer: {error}"))
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fused_retrieval_retains_complementary_candidates_with_stable_ties() {
        let scores = vec![
            Score {
                key: "a".into(),
                score: 0.9,
            },
            Score {
                key: "b".into(),
                score: 0.8,
            },
            Score {
                key: "c".into(),
                score: 0.7,
            },
        ];
        assert_eq!(
            shortlist(&scores, &["c".into(), "b".into(), "a".into()]),
            vec!["a", "c", "b"]
        );
    }

    #[test]
    fn labels_are_rejected_by_the_model_worker() {
        let input = serde_json::json!({"facts":[],"queries":[{"id":"q","query":"where is x","lexical":[],"expected":"x"}],"repeats":3});
        assert!(serde_json::from_value::<Input>(input).is_err());
    }
}
