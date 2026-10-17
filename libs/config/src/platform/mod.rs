use qol_platform::Os;

use crate::normalized::ResolvedConfig;
use crate::validation::ValidationError;

/// Whether a `platforms` list includes the running OS. Omitted means every OS.
pub fn supports_current_platform(platforms: Option<&[String]>) -> bool {
    platforms.is_none_or(|platforms| {
        Os::current().is_some_and(|os| platforms.iter().any(|platform| platform == os.name()))
    })
}

impl ResolvedConfig {
    /// The config as the settings panel shows it on the running OS, without
    /// the fields and sections whose `platforms` leave this OS out.
    pub fn for_this_platform(&self) -> ResolvedConfig {
        let mut config = self.clone();
        config
            .fields
            .retain(|field| supports_current_platform(field.platforms.as_deref()));
        config
            .sections
            .retain(|section| supports_current_platform(section.platforms.as_deref()));
        for section in &mut config.sections {
            section
                .fields
                .retain(|field| supports_current_platform(field.platforms.as_deref()));
        }
        config
    }
}

pub(crate) fn validate_platforms(
    path: &str,
    platforms: Option<&[String]>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(platforms) = platforms else {
        return;
    };
    if platforms.is_empty() {
        errors.push(ValidationError::new(
            format!("{path}.platforms"),
            "must name at least one platform",
        ));
    }
    for (index, platform) in platforms.iter().enumerate() {
        if Os::from_name(platform).is_some() {
            continue;
        }
        errors.push(ValidationError::new(
            format!("{path}.platforms[{index}]"),
            format!("unknown platform {platform}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_list_gates_on_the_running_os() {
        let here = vec![current_os().name().to_string()];
        let elsewhere: Vec<String> = Os::ALL
            .into_iter()
            .filter(|os| *os != current_os())
            .map(|os| os.name().to_string())
            .collect();
        assert!(supports_current_platform(None));
        assert!(supports_current_platform(Some(&here)));
        assert!(!supports_current_platform(Some(&elsewhere)));
    }

    fn current_os() -> Os {
        Os::current().expect("tests run on a supported OS")
    }
}
