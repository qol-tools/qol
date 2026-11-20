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
    let rest = base(package).strip_prefix("linux-image-")?;
    let rest = rest.strip_prefix("unsigned-").unwrap_or(rest);
    let mut parts = rest.splitn(3, '-');
    let version = parts.next()?;
    let build = parts.next()?;
    (version.split('.').all(|part| part.parse::<u32>().is_ok()) && build.parse::<u32>().is_ok())
        .then(|| format!("{version}-{build}"))
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
    images(&command::run("dpkg-query", &IMAGES, command::READ)?)
}

fn images(listed: &Done) -> Result<Vec<String>, String> {
    match listed.code {
        Some(0) => Ok(listed
            .stdout
            .lines()
            .filter_map(|line| line.strip_prefix("installed\t"))
            .map(String::from)
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
        assert_eq!(base("bluez:amd64"), "bluez");
        assert!(version_key("6.8.0-147") > version_key("6.8.0-146"));
        assert!(version_key("6.10.0-1") > version_key("6.8.0-147"));
    }

    #[test]
    fn installed_images_read_dpkg_query_exits() {
        let listing = "installed\tlinux-image-6.8.0-149-lowlatency\n\
                       config-files\tlinux-image-6.8.0-111-generic\n\
                       not-installed\tlinux-image-6.17.0-1008-nvidia\n";
        assert_eq!(
            images(&done(Some(0), listing, "")),
            Ok(vec!["linux-image-6.8.0-149-lowlatency".to_string()])
        );
        assert_eq!(
            images(&done(
                Some(1),
                "",
                "dpkg-query: no packages found matching linux-image-*"
            )),
            Ok(Vec::new())
        );
        assert_eq!(
            images(&done(Some(2), "", "dpkg-query: error: database is locked")),
            Err("dpkg-query: error: database is locked".to_string())
        );
        assert!(images(&done(None, "", "")).is_err());
    }
}
