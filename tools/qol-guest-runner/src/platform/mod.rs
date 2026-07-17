#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(crate) use linux::run;

#[cfg(not(target_os = "linux"))]
pub(crate) fn run() -> anyhow::Result<()> {
    anyhow::bail!("qol-guest-runner is only supported inside Linux guests")
}
