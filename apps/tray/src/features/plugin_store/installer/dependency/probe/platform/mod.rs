#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod stub;

#[cfg(target_os = "linux")]
pub(super) use linux::verify_candidate_runs;
#[cfg(not(target_os = "linux"))]
pub(super) use stub::verify_candidate_runs;
