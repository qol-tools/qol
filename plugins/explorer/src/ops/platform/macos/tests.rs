use std::fs;

use super::*;
use crate::ops::bin::{BinError, Held};
use crate::ops::platform::name_problem;
use crate::ops::print::Print;
use crate::ops::NameProblem;

#[test]
fn name_rules_forbid_the_colon_slash_and_dots() {
    assert_eq!(name_problem(""), Some(NameProblem::Empty));
    assert!(matches!(
        name_problem("a:b"),
        Some(NameProblem::Forbidden(_))
    ));
    assert!(matches!(
        name_problem("a/b"),
        Some(NameProblem::Forbidden(_))
    ));
    assert!(matches!(name_problem("."), Some(NameProblem::Forbidden(_))));
    assert_eq!(name_problem(&"x".repeat(256)), Some(NameProblem::TooLong));
    assert_eq!(name_problem("CON"), None);
}

#[test]
fn restore_says_plainly_that_it_cannot() {
    let dir = tempfile::tempdir().unwrap();
    let item = dir.path().join("a");
    fs::write(&item, "").unwrap();
    let held = Held {
        before: Print::read(&item).unwrap(),
        item,
        sidecar: None,
    };
    let result = system_bin().restore(&held, &dir.path().join("b"));
    assert_eq!(result, Err(BinError::Failed(NO_RESTORE.into())));
}
