use super::*;
use crate::ops::platform::name_problem;
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
    let failures = restore_trashed(&[PathBuf::from("/Users/me/.Trash/a")]);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].reason, NO_RESTORE);
}
