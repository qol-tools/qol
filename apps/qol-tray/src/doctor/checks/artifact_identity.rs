use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use qol_conventions::artifact::{BuildIdentity, BuildRole, RunningBuildInfo};

const ID: &str = "artifact_identity";

pub(super) struct ArtifactIdentityCheck;

impl DoctorCheck for ArtifactIdentityCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Artifact identity", CheckCategory::Install)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let running = match running_build_info() {
            Ok(running) => running,
            Err(error) => return CheckReport::error(error, ID),
        };
        let inspected = match qol_artifact::inspect_path(&running.executable) {
            Ok(inspected) => inspected,
            Err(error) => {
                return CheckReport::error(
                    format!(
                        "cannot inspect on-disk executable {}: {error}",
                        running.executable.display()
                    ),
                    ID,
                );
            }
        };
        diagnose(
            &running,
            inspected.slices.iter().map(|slice| &slice.identity),
        )
    }
}

fn running_build_info() -> Result<RunningBuildInfo, String> {
    let current = qol_conventions::artifact::current()
        .cloned()
        .ok_or_else(|| "running build identity is unavailable".to_string())?;
    if invoked_as_doctor(&current) {
        match running_host_build_info() {
            Ok(info) => return Ok(info),
            Err(RunningHostQueryError::Unavailable(error)) => {
                log::debug!("[artifact-identity] running host query unavailable: {error}");
            }
            Err(RunningHostQueryError::Invalid(error)) => return Err(error),
        }
    }
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot resolve executable: {error}"))?;
    Ok(RunningBuildInfo {
        identity: current,
        executable,
    })
}

fn invoked_as_doctor(current: &BuildIdentity) -> bool {
    current.role == BuildRole::Doctor || std::env::args().any(|argument| argument == "doctor")
}

#[derive(Debug)]
enum RunningHostQueryError {
    Unavailable(String),
    Invalid(String),
}

fn running_host_build_info() -> Result<RunningBuildInfo, RunningHostQueryError> {
    let (status, body) = crate::commands::local_http::get_from_daemon("/api/build-info")
        .map_err(|error| RunningHostQueryError::Unavailable(error.to_string()))?;
    parse_running_host_response(status, &body)
}

fn parse_running_host_response(
    status: u16,
    body: &str,
) -> Result<RunningBuildInfo, RunningHostQueryError> {
    if status != 200 {
        return Err(RunningHostQueryError::Invalid(format!(
            "running host /api/build-info failed with HTTP {status}"
        )));
    }
    serde_json::from_str(body).map_err(|error| {
        RunningHostQueryError::Invalid(format!("invalid /api/build-info: {error}"))
    })
}

fn diagnose<'a>(
    running: &RunningBuildInfo,
    on_disk: impl Iterator<Item = &'a BuildIdentity>,
) -> CheckReport {
    if on_disk
        .into_iter()
        .any(|identity| identity == &running.identity)
    {
        return CheckReport::ok(format!(
            "running {} {} matches on-disk executable ({})",
            running.identity.binary,
            running.identity.version,
            running.executable.display()
        ));
    }
    CheckReport::error(
        format!(
            "running {} {} differs from on-disk executable {}",
            running.identity.binary,
            running.identity.version,
            running.executable.display()
        ),
        ID,
    )
}

#[cfg(test)]
mod tests {
    use super::{diagnose, parse_running_host_response, RunningHostQueryError};
    use qol_conventions::artifact::{
        BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, CompilerFacts,
        RunningBuildInfo, SourceIdentity, SCHEMA_VERSION,
    };
    use std::path::PathBuf;

    fn identity(version: &str) -> BuildIdentity {
        BuildIdentity {
            schema: SCHEMA_VERSION,
            binary: "qol-tray".to_string(),
            role: BuildRole::Host,
            package: "qol-tray".to_string(),
            version: version.to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            intent: BuildIntent::Production,
            flavor: BuildFlavor {
                profile: BuildProfile::Release,
                dev_features: false,
            },
            compiler: CompilerFacts {
                cargo_profile: "release".to_string(),
                opt_level: "3".to_string(),
                debuginfo: false,
                debug_assertions: false,
                overflow_checks: None,
                test: false,
            },
            features: vec!["default".to_string()],
            source: SourceIdentity::Git {
                commit: "a".repeat(40),
                head_tree: "b".repeat(40),
                working_tree: "b".repeat(40),
            },
        }
    }

    #[test]
    fn running_identity_must_exist_in_the_on_disk_slices() {
        let running = RunningBuildInfo {
            identity: identity("1.0.0"),
            executable: PathBuf::from("/opt/qol-tray"),
        };
        let matching = identity("1.0.0");
        let stale = identity("0.9.0");

        assert!(diagnose(&running, [&matching].into_iter())
            .issues
            .is_empty());
        assert_eq!(diagnose(&running, [&stale].into_iter()).issues.len(), 1);
    }

    #[test]
    fn reachable_daemon_with_invalid_build_info_fails_closed() {
        assert!(matches!(
            parse_running_host_response(404, ""),
            Err(RunningHostQueryError::Invalid(_))
        ));
        assert!(matches!(
            parse_running_host_response(200, "{}"),
            Err(RunningHostQueryError::Invalid(_))
        ));

        let running = RunningBuildInfo {
            identity: identity("1.0.0"),
            executable: PathBuf::from("/opt/qol-tray"),
        };
        let body = serde_json::to_string(&running).unwrap();
        assert_eq!(parse_running_host_response(200, &body).unwrap(), running);
    }
}
