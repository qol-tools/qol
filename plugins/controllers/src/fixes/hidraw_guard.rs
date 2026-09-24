use anyhow::{bail, Result};

pub const RULE_CONTENT: &str = "\
# qol controllers: keep apps off the raw HID node of controllers the nintendo driver owns
SUBSYSTEM==\"hidraw\", DRIVERS==\"nintendo\", TAG-=\"uaccess\"
";

pub fn installed(on_disk: Option<&str>) -> bool {
    on_disk == Some(RULE_CONTENT)
}

pub fn ensure() -> Result<()> {
    let Some(path) = super::platform::hidraw_guard_path() else {
        bail!("guarding controllers from other apps is only supported on Linux");
    };
    if installed(std::fs::read_to_string(&path).ok().as_deref()) {
        return Ok(());
    }
    super::platform::install_hidraw_guard(&path, RULE_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_counts_only_the_exact_rule_as_installed() {
        let cases = [
            ("missing file", None, false),
            ("exact rule", Some(RULE_CONTENT), true),
            ("empty file", Some(""), false),
            (
                "edited rule",
                Some("SUBSYSTEM==\"hidraw\", DRIVERS==\"nintendo\"\n"),
                false,
            ),
        ];
        for (label, on_disk, expected) in cases {
            assert_eq!(installed(on_disk), expected, "case: {label}");
        }
    }

    #[test]
    fn guard_rule_drops_uaccess_for_nintendo_hidraw_nodes() {
        let rule = RULE_CONTENT
            .lines()
            .find(|line| !line.starts_with('#'))
            .expect("rule line");
        assert_eq!(
            rule,
            "SUBSYSTEM==\"hidraw\", DRIVERS==\"nintendo\", TAG-=\"uaccess\""
        );
    }
}
