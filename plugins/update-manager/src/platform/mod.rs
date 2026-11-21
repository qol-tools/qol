use std::fmt;

#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    Dismissed,
    Denied,
    TimedOut,
    Failed(String),
}

impl Refusal {
    pub(crate) fn stops(&self) -> bool {
        matches!(self, Self::Dismissed | Self::Denied | Self::TimedOut)
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dismissed => formatter.write_str("the password prompt was closed"),
            Self::Denied => formatter.write_str("the password was not accepted"),
            Self::TimedOut => formatter.write_str("the password prompt timed out"),
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}
