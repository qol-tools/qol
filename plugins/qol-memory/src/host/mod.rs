use std::sync::OnceLock;

pub fn compose(override_env: Option<&str>, hostname: &str, os: &str) -> String {
    if let Some(value) = override_env {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let name = hostname.trim();
    let name = if name.is_empty() { "unknown" } else { name };
    format!("{name}/{os}")
}

pub fn current() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        let override_env = std::env::var("QOL_MEMORY_HOST").ok();
        let hostname = gethostname::gethostname().to_string_lossy().into_owned();
        compose(override_env.as_deref(), &hostname, std::env::consts::OS)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_when_non_empty_after_trim() {
        assert_eq!(compose(Some(" box-b "), "box-a", "linux"), "box-b");
    }

    #[test]
    fn empty_override_is_ignored() {
        assert_eq!(compose(Some("   "), "box-a", "linux"), "box-a/linux");
    }

    #[test]
    fn empty_hostname_becomes_unknown() {
        assert_eq!(compose(None, "   ", "linux"), "unknown/linux");
    }

    #[test]
    fn normal_case_yields_name_os() {
        assert_eq!(compose(None, "box-a", "linux"), "box-a/linux");
    }
}
