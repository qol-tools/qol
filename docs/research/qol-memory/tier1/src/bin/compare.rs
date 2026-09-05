use anyhow::{Context, Result};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let embedding = args.next().context("expected embedding model directory")?;
    let equivalence = args
        .next()
        .context("expected equivalence model directory")?;
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build_global()?;
    let input = serde_json::from_reader(std::io::stdin())?;
    let output =
        qol_memory_tier1_probe::comparison::run(input, embedding.as_ref(), equivalence.as_ref())?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
