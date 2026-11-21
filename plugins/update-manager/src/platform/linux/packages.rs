pub(crate) fn base(package: &str) -> &str {
    package.split(':').next().unwrap_or(package)
}

pub(crate) fn kernel_abi(package: &str) -> Option<String> {
    let rest = base(package).strip_prefix("linux-image-")?;
    let mut parts = rest.splitn(3, '-');
    let version = parts.next()?;
    let build = parts.next()?;
    (version.split('.').all(|part| part.parse::<u32>().is_ok()) && build.parse::<u32>().is_ok())
        .then(|| format!("{version}-{build}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_packages_name_their_abi() {
        assert_eq!(
            kernel_abi("linux-image-6.8.0-147-lowlatency:amd64").as_deref(),
            Some("6.8.0-147")
        );
        assert_eq!(kernel_abi("linux-image-lowlatency"), None);
        assert_eq!(base("bluez:amd64"), "bluez");
    }
}
