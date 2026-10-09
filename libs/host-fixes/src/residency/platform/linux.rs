use anyhow::Result;

pub(crate) fn device_id() -> Result<String> {
    let owner = crate::policy::stable_host_owner("residency")?;
    Ok(owner.as_str().to_string())
}
