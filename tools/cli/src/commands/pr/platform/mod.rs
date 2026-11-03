#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub(super) use fallback::{prepare_plugin, prepare_tray, tray_binary};
#[cfg(target_os = "macos")]
pub(super) use macos::{prepare_plugin, prepare_tray, tray_binary};

use anyhow::{bail, Result};
use qol_platform::Os;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Build {
    os: Os,
    arch: &'static str,
    pub(super) target: &'static str,
    pub(super) artifact: &'static str,
    pub(super) job: &'static str,
}

const BUILDS: [Build; 2] = [
    Build {
        os: Os::Linux,
        arch: "x86_64",
        target: "x86_64-unknown-linux-gnu",
        artifact: "live-test-build-linux-x86_64",
        job: "Build for the live test (linux)",
    },
    Build {
        os: Os::Macos,
        arch: "aarch64",
        target: "aarch64-apple-darwin",
        artifact: "live-test-build-macos-aarch64",
        job: "Build for the live test (macos)",
    },
];

impl Build {
    pub(super) fn label(&self) -> String {
        format!("{} {}", self.os.label(), self.arch)
    }
}

pub(super) fn host_build() -> Result<&'static Build> {
    build_for(Os::current(), std::env::consts::ARCH)
}

fn build_for(os: Option<Os>, arch: &str) -> Result<&'static Build> {
    if let Some(build) = BUILDS
        .iter()
        .find(|build| Some(build.os) == os && build.arch == arch)
    {
        return Ok(build);
    }
    let made = BUILDS
        .iter()
        .map(Build::label)
        .collect::<Vec<_>>()
        .join(" and ");
    let here = os.map_or("this OS", Os::label);
    bail!("pull request builds are made for {made} only, not {here} {arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_supported_host_gets_its_own_build() {
        let linux = build_for(Some(Os::Linux), "x86_64").unwrap();
        assert_eq!(linux.target, "x86_64-unknown-linux-gnu");
        assert_eq!(linux.artifact, "live-test-build-linux-x86_64");
        assert_eq!(linux.label(), "Linux x86_64");
        let mac = build_for(Some(Os::Macos), "aarch64").unwrap();
        assert_eq!(mac.target, "aarch64-apple-darwin");
        assert_eq!(mac.artifact, "live-test-build-macos-aarch64");
        assert_eq!(mac.label(), "macOS aarch64");
        assert!(mac.job.ends_with("(macos)"), "{}", mac.job);
    }

    #[test]
    fn other_hosts_are_refused_with_the_supported_ones_named() {
        let refused = [
            (Some(Os::Macos), "x86_64"),
            (Some(Os::Linux), "aarch64"),
            (Some(Os::Windows), "x86_64"),
            (None, "x86_64"),
        ];
        for (os, arch) in refused {
            let error = build_for(os, arch).unwrap_err().to_string();
            assert!(
                error.contains("Linux x86_64 and macOS aarch64 only"),
                "{error}"
            );
            assert!(error.contains(arch), "{error}");
        }
    }
}
