use std::fs;

use super::command::{self, Done};

const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
const NONE_MATCHED: i32 = 1;
const IMAGES: [&str; 3] = [
    "-W",
    "-f=${db:Status-Status}\t${Package}\n",
    "linux-image-*",
];

pub(crate) fn base(package: &str) -> &str {
    package.split(':').next().unwrap_or(package)
}

pub(crate) fn kernel_abi(package: &str) -> Option<String> {
    base(package)
        .starts_with("linux-image-")
        .then(|| versioned_abi(package))
        .flatten()
}

pub(crate) fn versioned_abi(package: &str) -> Option<String> {
    let rest = base(package).strip_prefix("linux-")?;
    let parts: Vec<&str> = rest.split('-').collect();
    parts.windows(2).find_map(|pair| {
        let numbers: Vec<&str> = pair[0].split('.').collect();
        let dotted = numbers.len() == 3 && numbers.iter().all(|part| part.parse::<u32>().is_ok());
        (dotted && pair[1].parse::<u32>().is_ok()).then(|| format!("{}-{}", pair[0], pair[1]))
    })
}

pub(crate) fn version_key(abi: &str) -> Vec<u32> {
    abi.split(['.', '-'])
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

pub(crate) fn running_release() -> Option<String> {
    fs::read_to_string(OSRELEASE)
        .ok()
        .map(|release| release.trim().to_string())
}

pub(crate) fn running_kernel() -> Option<String> {
    kernel_abi(&format!("linux-image-{}", running_release()?))
}

pub(crate) fn installed_images() -> Result<Vec<String>, String> {
    images(
        &command::run("dpkg-query", &IMAGES, command::READ)?,
        installed,
    )
}

pub(crate) fn images_on_disk() -> Result<Vec<String>, String> {
    images(
        &command::run("dpkg-query", &IMAGES, command::READ)?,
        on_disk,
    )
}

fn installed(status: &str) -> bool {
    status == "installed"
}

fn on_disk(status: &str) -> bool {
    !matches!(status, "not-installed" | "config-files")
}

fn images(listed: &Done, keep: fn(&str) -> bool) -> Result<Vec<String>, String> {
    match listed.code {
        Some(0) => Ok(listed
            .stdout
            .lines()
            .filter_map(|line| {
                let (status, package) = line.split_once('\t')?;
                keep(status).then(|| package.to_string())
            })
            .collect()),
        Some(NONE_MATCHED) => Ok(Vec::new()),
        _ => Err(listed.reason()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(code: Option<i32>, stdout: &str, stderr: &str) -> Done {
        Done {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn only_versioned_kernel_packages_carry_an_abi() {
        assert_eq!(
            versioned_abi("linux-hwe-6.8-headers-6.8.0-111").as_deref(),
            Some("6.8.0-111")
        );
        assert_eq!(
            versioned_abi("linux-modules-extra-6.8.0-111-generic:amd64").as_deref(),
            Some("6.8.0-111")
        );
        assert_eq!(versioned_abi("linux-image-generic"), None);
        assert_eq!(versioned_abi("linux-firmware"), None);
        assert_eq!(versioned_abi("libfoo1"), None);
    }

    #[test]
    fn kernel_packages_name_their_abi() {
        assert_eq!(
            kernel_abi("linux-image-6.8.0-147-lowlatency:amd64").as_deref(),
            Some("6.8.0-147")
        );
        assert_eq!(
            kernel_abi("linux-image-unsigned-6.8.0-152-generic").as_deref(),
            Some("6.8.0-152")
        );
        assert_eq!(kernel_abi("linux-image-lowlatency"), None);
        assert_eq!(kernel_abi("linux-image-unsigned-generic"), None);
        assert_eq!(
            kernel_abi("linux-modules-6.8.0-152-generic"),
            None,
            "only an image names a kernel"
        );
        assert_eq!(base("bluez:amd64"), "bluez");
        assert!(version_key("6.8.0-147") > version_key("6.8.0-146"));
        assert!(version_key("6.10.0-1") > version_key("6.8.0-147"));
    }

    #[test]
    fn installed_images_read_dpkg_query_exits() {
        let listing = "installed\tlinux-image-6.8.0-149-lowlatency\n\
                       half-configured\tlinux-image-6.8.0-150-lowlatency\n\
                       config-files\tlinux-image-6.8.0-111-generic\n\
                       not-installed\tlinux-image-6.17.0-1008-nvidia\n";
        assert_eq!(
            images(&done(Some(0), listing, ""), installed),
            Ok(vec!["linux-image-6.8.0-149-lowlatency".to_string()])
        );
        assert_eq!(
            images(&done(Some(0), listing, ""), on_disk),
            Ok(vec![
                "linux-image-6.8.0-149-lowlatency".to_string(),
                "linux-image-6.8.0-150-lowlatency".to_string()
            ]),
            "a kernel whose setup failed is still on disk and counts as one of the newest"
        );
        assert_eq!(
            images(
                &done(
                    Some(1),
                    "",
                    "dpkg-query: no packages found matching linux-image-*"
                ),
                installed
            ),
            Ok(Vec::new())
        );
        assert_eq!(
            images(
                &done(Some(2), "", "dpkg-query: error: database is locked"),
                installed
            ),
            Err("dpkg-query: error: database is locked".to_string())
        );
        assert!(images(&done(None, "", ""), installed).is_err());
    }
}
