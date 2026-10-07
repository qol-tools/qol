use crate::registry::{read_body, send};
use anyhow::{Context, Result};
use qol_plugin_index::IndexDocument;

pub fn fetch(agent: &ureq::Agent, url: &str, public_key: &str) -> Result<Option<IndexDocument>> {
    let response = send(agent.get(url), None, url)?;
    match response.status() {
        404 => return Ok(None),
        200 => {}
        status => anyhow::bail!("{url} answered {status}"),
    }
    let signed = read_body(response, url)?;
    let document = qol_plugin_index::verify_signed(&signed, public_key)
        .with_context(|| format!("the deployed index at {url} does not verify"))?;
    Ok(Some(document))
}
