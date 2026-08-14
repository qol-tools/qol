use std::path::PathBuf;

use anyhow::{Context, Result};
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;
use rayon::prelude::*;

const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";
const MAX_SEQ: usize = 512;

#[derive(Deserialize)]
struct Question {
    id: String,
    category: String,
    query: String,
    target_key: Option<String>,
    covered: bool,
}

#[derive(Deserialize)]
struct QuestionsDoc {
    run_pin: Option<String>,
    questions: Vec<Question>,
}

#[derive(Deserialize)]
struct Unit {
    key: String,
    source: String,
    kind: String,
    text: String,
}

#[derive(Serialize)]
struct ResultRow {
    id: String,
    category: String,
    covered: bool,
    hit1: bool,
    hit5: bool,
    target_rank: Option<usize>,
    top5: Vec<String>,
}

#[derive(Serialize)]
struct Stats {
    questions: usize,
    covered: usize,
    coverage_share: f64,
    hit1: usize,
    hit5: usize,
    hit1_share: f64,
    hit5_share: f64,
    by_category: std::collections::BTreeMap<String, CategoryStats>,
    units_indexed: usize,
    embed_ms: u128,
}

#[derive(Serialize)]
struct CategoryStats {
    n: usize,
    covered: usize,
    hit1: usize,
    hit5: usize,
}

#[derive(Serialize)]
struct Report {
    name: &'static str,
    schema_version: u8,
    started_at: String,
    finished_at: String,
    status: &'static str,
    inputs: serde_json::Value,
    artifacts: serde_json::Value,
    commands: Vec<String>,
    stats: Stats,
    results: Vec<ResultRow>,
    next: Vec<String>,
}

fn embed_text(
    model: &BertModel,
    tokenizer: &Tokenizer,
    text: &str,
    device: &Device,
) -> Result<Tensor> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
    let ids: Vec<u32> = encoding
        .get_ids()
        .iter()
        .take(MAX_SEQ)
        .map(|v| *v as u32)
        .collect();
    if ids.is_empty() {
        anyhow::bail!("empty token ids");
    }
    let input = Tensor::new(ids.as_slice(), device)?.unsqueeze(0)?;
    let token_type_ids = input.zeros_like()?;
    let output = model.forward(&input, &token_type_ids, None)?;
    let cls = output.narrow(1, 0, 1)?.squeeze(1)?;
    let norm: f32 = cls.sqr()?.sum_all()?.sqrt()?.to_scalar()?;
    Ok((cls / (norm as f64))?.squeeze(0)?)
}

fn dot(a: &Tensor, b: &Tensor) -> Result<f32> {
    let d = a.broadcast_mul(b)?.sum_all()?.to_scalar::<f32>()?;
    Ok(d)
}

fn main() -> Result<()> {
    let started = std::time::Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let pick = |flag: &str, def: &str| -> String {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| def.to_string())
    };
    let cwd = std::env::current_dir()?;
    let base = cwd.join("docs/research/qol-memory");
    let snapshot = PathBuf::from(pick("--snapshot", ""));
    let snapshot = if snapshot.as_os_str().is_empty() {
        let home = std::env::var("HOME").unwrap_or_default();
        let data_home = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty());
        let store_root = data_home
            .map(|d| std::path::PathBuf::from(d).join("qol-tray/plugins/qol-memory"))
            .unwrap_or_else(|| std::path::PathBuf::from(home).join(".local/share/qol-tray/plugins/qol-memory"));
        let root = store_root.join("snapshot");
        let mut runs: Vec<PathBuf> = std::fs::read_dir(&root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        runs.sort();
        runs.last().cloned().context("no snapshot runs")?
    } else {
        snapshot
    };
    let snapshot_jsonl = snapshot.join("snapshot.jsonl");
    let questions_path = base.join("eval/questions.json");
    let dump_dense = pick("--dump-dense", "");
    let kinds: Vec<String> = pick("--kinds", "user")
        .split(',')
        .map(|k| k.to_string())
        .collect();
    let threads: usize = pick("--threads", "4")
        .parse()
        .unwrap_or(4)
        .clamp(1, 8);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();
    let out_dir = PathBuf::from(pick("--out", ""));
    let out_dir = if out_dir.as_os_str().is_empty() {
        cwd.join("reports/qol-memory/eval")
            .join(chrono_like_run_id())
    } else {
        out_dir
    };
    let model_dir = PathBuf::from(pick(
        "--model-dir",
        &format!("{}/.cache/qol-memory/bge-small-en-v1.5", std::env::var("HOME")?),
    ));

    let questions_doc: QuestionsDoc =
        serde_json::from_str(&std::fs::read_to_string(&questions_path)?)?;
    let units: Vec<Unit> = std::fs::read_to_string(&snapshot_jsonl)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| anyhow::anyhow!("unit parse: {e}")))
        .collect::<Result<_>>()?;
    let user_units: Vec<&Unit> = units.iter().filter(|u| kinds.contains(&u.kind)).collect();

    let device = Device::Cpu;
    let config: Config = serde_json::from_str(&std::fs::read_to_string(model_dir.join("config.json"))?)?;
    let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json").to_str().unwrap())
        .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[model_dir.join("model.safetensors")], DTYPE, &device)?
    };
    let model = BertModel::load(vb, &config)?;

    let unit_vecs: Vec<(String, Tensor)> = user_units
        .par_iter()
        .map(|u| {
            let v = embed_text(&model, &tokenizer, &u.text, &device)?;
            Ok((u.key.clone(), v))
        })
        .collect::<Result<_>>()?;

    let mut dense_dump: std::collections::BTreeMap<String, Vec<(String, f32)>> =
        std::collections::BTreeMap::new();
    let mut results = Vec::with_capacity(questions_doc.questions.len());
    let mut hit1 = 0usize;
    let mut hit5 = 0usize;
    let mut covered = 0usize;
    let mut by_category: std::collections::BTreeMap<String, CategoryStats> =
        std::collections::BTreeMap::new();
    for q in &questions_doc.questions {
        let query = format!("{QUERY_INSTRUCTION}{}", q.query);
        let qv = embed_text(&model, &tokenizer, &query, &device)?;
        let mut scored: Vec<(String, f32)> = Vec::with_capacity(unit_vecs.len());
        for (key, uv) in &unit_vecs {
            scored.push((key.clone(), dot(&qv, uv)?));
        }
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        dense_dump.insert(q.id.clone(), scored.clone());
        let cat = by_category.entry(q.category.clone()).or_insert(CategoryStats {
            n: 0,
            covered: 0,
            hit1: 0,
            hit5: 0,
        });
        cat.n += 1;
        let rank = q
            .target_key
            .as_ref()
            .and_then(|tk| scored.iter().position(|(k, _)| k == tk));
        let h1 = q.covered && rank == Some(0);
        let h5 = q.covered && rank.map(|r| r < 5).unwrap_or(false);
        if q.covered {
            covered += 1;
            cat.covered += 1;
        }
        if h1 {
            hit1 += 1;
            cat.hit1 += 1;
        }
        if h5 {
            hit5 += 1;
            cat.hit5 += 1;
        }
        results.push(ResultRow {
            id: q.id.clone(),
            category: q.category.clone(),
            covered: q.covered,
            hit1: h1,
            hit5: h5,
            target_rank: rank,
            top5: scored
                .iter()
                .take(5)
                .map(|(k, s)| format!("{k}:{s:.3}"))
                .collect(),
        });
    }
    let stats = Stats {
        questions: questions_doc.questions.len(),
        covered,
        coverage_share: covered as f64 / questions_doc.questions.len() as f64,
        hit1,
        hit5,
        hit1_share: if covered > 0 {
            hit1 as f64 / covered as f64
        } else {
            0.0
        },
        hit5_share: if covered > 0 {
            hit5 as f64 / covered as f64
        } else {
            0.0
        },
        by_category,
        units_indexed: user_units.len(),
        embed_ms: started.elapsed().as_millis(),
    };
    if !dump_dense.is_empty() {
        let dump: std::collections::BTreeMap<String, Vec<(String, f32)>> = dense_dump
            .iter()
            .map(|(qid, rows)| (qid.clone(), rows.iter().map(|(k, v)| (k.clone(), *v)).collect()))
            .collect();
        std::fs::write(
            &dump_dense,
            serde_json::to_string(&dump)?,
        )?;
        println!("dense dump written: {dump_dense}");
    }
    std::fs::create_dir_all(&out_dir)?;
    let report_path = out_dir.join("report.json");
    let report = Report {
        name: "qol-memory eval (candle bge-small-en-v1.5)",
        schema_version: 1,
        started_at: started.elapsed().as_secs().to_string(),
        finished_at: chrono_like_run_id(),
        status: "pass",
        inputs: serde_json::json!({
            "snapshotRun": snapshot.file_name().unwrap_or_default().to_string_lossy(),
            "questions": questions_path.to_string_lossy(),
            "model": "BAAI/bge-small-en-v1.5",
            "queryInstruction": QUERY_INSTRUCTION,
            "pooling": "CLS",
            "indexContent": "user units only",
            "device": "cpu"
        }),
        artifacts: serde_json::json!({ "report": report_path.to_string_lossy() }),
        commands: vec![
            "cargo run --release --manifest-path docs/research/qol-memory/tier1/Cargo.toml"
                .to_string(),
        ],
        stats,
        results,
        next: vec![
            "Compare against BM25 baseline: hit@1 40%, hit@5 55% on the same frozen questions".to_string(),
            "If dense wins, decide: index assistant text or summaries next (coverage gap)".to_string(),
        ],
    };
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    println!(
        "units indexed {} | covered {}/{} | hit@1 {}/{} ({:.0}%) | hit@5 {}/{} ({:.0}%) | embed+score {}ms",
        report.stats.units_indexed,
        report.stats.covered,
        report.stats.questions,
        report.stats.hit1,
        report.stats.covered,
        report.stats.hit1_share * 100.0,
        report.stats.hit5,
        report.stats.covered,
        report.stats.hit5_share * 100.0,
        report.stats.embed_ms
    );
    for (cat, c) in &report.stats.by_category {
        println!(
            "  {cat:8} hit1 {}/{} hit5 {}/{}",
            c.hit1, c.covered, c.hit5, c.covered
        );
    }
    println!("report: {}", report_path.display());
    Ok(())
}

fn chrono_like_run_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    format!("tier1-{}", now.as_millis())
}
