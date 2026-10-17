use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

const BUILD_TARGET_OS: &str = "CARGO_CFG_TARGET_OS";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Linux, Os::Macos, Os::Windows];

    pub fn current() -> Option<Os> {
        Os::from_name(std::env::consts::OS)
    }

    pub fn build_target() -> Option<Os> {
        std::env::var(BUILD_TARGET_OS)
            .ok()
            .as_deref()
            .and_then(Os::from_name)
    }

    pub fn from_name(name: &str) -> Option<Os> {
        Os::ALL.into_iter().find(|os| os.name() == name)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Os::Linux => "Linux",
            Os::Macos => "macOS",
            Os::Windows => "Windows",
        }
    }

    pub fn is_current(self) -> bool {
        Os::current() == Some(self)
    }

    pub fn includes_current(platforms: Option<&[Os]>) -> bool {
        match platforms {
            None => true,
            Some(platforms) => Os::current().is_some_and(|os| platforms.contains(&os)),
        }
    }
}

pub fn current_name() -> &'static str {
    Os::current().map_or(std::env::consts::OS, Os::name)
}

pub fn current_label() -> &'static str {
    Os::current().map_or(std::env::consts::OS, Os::label)
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOs(pub String);

impl fmt::Display for UnknownOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let known = Os::ALL.map(Os::name).join(", ");
        write!(f, "unknown OS `{}`; expected one of {known}", self.0)
    }
}

impl std::error::Error for UnknownOs {}

impl FromStr for Os {
    type Err = UnknownOs;

    fn from_str(name: &str) -> Result<Os, UnknownOs> {
        Os::from_name(name).ok_or_else(|| UnknownOs(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for os in Os::ALL {
            assert_eq!(Os::from_name(os.name()), Some(os));
            assert_eq!(os.name().parse::<Os>(), Ok(os));
            assert_eq!(os.to_string(), os.name());
        }
    }

    #[test]
    fn serde_uses_the_name() {
        for os in Os::ALL {
            let json = serde_json::to_string(&os).unwrap();
            assert_eq!(json, format!("\"{}\"", os.name()));
            assert_eq!(serde_json::from_str::<Os>(&json).unwrap(), os);
        }
    }

    #[test]
    fn unknown_names_are_refused() {
        assert_eq!(Os::from_name("freebsd"), None);
        assert!("Linux".parse::<Os>().is_err());
    }

    #[test]
    fn no_list_means_every_os() {
        assert!(Os::includes_current(None));
        assert!(!Os::includes_current(Some(&[])));
    }

    #[test]
    fn the_current_os_is_in_the_full_list() {
        assert_eq!(
            Os::includes_current(Some(&Os::ALL)),
            Os::current().is_some()
        );
        assert_eq!(current_name(), std::env::consts::OS);
    }
}
