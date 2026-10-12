use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use crate::plugins::manifest::PluginManifest;
use std::path::PathBuf;

const ID: &str = "socket_paths";

pub(super) struct SocketPathsCheck;

impl DoctorCheck for SocketPathsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Local socket paths", CheckCategory::Runtime)
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let mut sockets = vec![(
            "qol-tray".to_string(),
            crate::dev_generation::state_socket_path(),
        )];
        if let Ok(registry) = ctx.registry() {
            sockets.extend(registry.entries.iter().filter_map(|entry| {
                let manifest = PluginManifest::read_from_dir(&entry.active.path).ok()?;
                let socket = manifest.daemon?.socket?;
                Some((
                    entry.id.clone(),
                    crate::dev_generation::daemon_socket_path(&socket),
                ))
            }));
        }
        report(&sockets, qol_runtime::local_ipc::MAX_SOCKET_PATH_BYTES)
    }
}

fn report(sockets: &[(String, PathBuf)], limit: usize) -> CheckReport {
    let issues: Vec<DoctorIssue> = sockets
        .iter()
        .filter(|(_, path)| path.as_os_str().len() > limit)
        .map(|(owner, path)| DoctorIssue {
            code: ID,
            severity: Severity::Error,
            message: format!(
                "{owner} cannot listen at {}: the path is {} bytes and local sockets allow {limit}",
                path.display(),
                path.as_os_str().len()
            ),
            evidence: vec![owner.clone()],
        })
        .collect();
    if issues.is_empty() {
        return CheckReport::ok(format!(
            "{} local socket path(s) fit the {limit}-byte limit",
            sockets.len()
        ));
    }
    CheckReport {
        summary: format!("{} local socket path(s) are too long", issues.len()),
        issues,
        advice: vec![
            "the qol data folder sits under a user profile path too long for local sockets"
                .to_string(),
        ],
        fixes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockets(paths: &[(&str, &str)]) -> Vec<(String, PathBuf)> {
        paths
            .iter()
            .map(|(owner, path)| (owner.to_string(), PathBuf::from(path)))
            .collect()
    }

    #[test]
    fn paths_within_the_limit_report_ok() {
        let report = report(
            &sockets(&[("qol-tray", "/run/s.sock"), ("a", "/run/a.sock")]),
            11,
        );
        assert!(report.issues.is_empty());
        assert_eq!(
            report.summary,
            "2 local socket path(s) fit the 11-byte limit"
        );
    }

    #[test]
    fn a_path_over_the_limit_is_an_error_naming_its_owner() {
        let report = report(
            &sockets(&[
                ("qol-tray", "/run/s.sock"),
                ("monitor", "/run/monitor.sock"),
            ]),
            11,
        );
        assert_eq!(report.summary, "1 local socket path(s) are too long");
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Error);
        assert_eq!(report.issues[0].evidence, vec!["monitor".to_string()]);
        assert!(
            report.issues[0].message.contains("17 bytes"),
            "{}",
            report.issues[0].message
        );
        assert!(report.fixes.is_empty());
        assert!(!report.advice.is_empty());
    }
}
