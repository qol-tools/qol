use std::path::PathBuf;

pub fn open(start: PathBuf) -> anyhow::Result<()> {
    anyhow::bail!(
        "the Explorer window is not built yet, so {} cannot be opened",
        start.display()
    )
}
