use std::time::SystemTime;

use crate::cleanup::{Leftover, Remove, Switch};
use crate::platform::Refusal;

const UNSUPPORTED: &str = "Update Manager reads what updates leave behind on Linux only so far";

pub(crate) fn leftovers() -> &'static [&'static Leftover] {
    &[]
}

pub(crate) fn switches() -> &'static [Switch] {
    &[]
}

pub(crate) fn found(_program: &str) -> bool {
    false
}

pub(crate) fn output(_argv: &[&str]) -> Result<String, String> {
    Err(UNSUPPORTED.into())
}

pub(crate) fn stamp() -> Vec<Option<SystemTime>> {
    Vec::new()
}

pub(crate) fn run(_remove: &Remove, _items: &[String], _admin: bool) -> Result<(), Refusal> {
    Err(Refusal::Failed(UNSUPPORTED.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_platform_has_no_kinds_and_no_switches_yet() {
        assert!(leftovers().is_empty());
        assert!(switches().is_empty());
    }
}
