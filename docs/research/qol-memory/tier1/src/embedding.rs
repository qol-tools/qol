use anyhow::Result;
use candle_core::{Device, Tensor};
use candle_transformers::models::bert::BertModel;
use tokenizers::Tokenizer;

const MAX_SEQ: usize = 512;

pub fn embed_text(
    model: &BertModel,
    tokenizer: &Tokenizer,
    text: &str,
    device: &Device,
) -> Result<Tensor> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
    let ids: Vec<u32> = encoding.get_ids().iter().take(MAX_SEQ).copied().collect();
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

pub fn dot(a: &Tensor, b: &Tensor) -> Result<f32> {
    let d = a.broadcast_mul(b)?.sum_all()?.to_scalar::<f32>()?;
    Ok(d)
}
