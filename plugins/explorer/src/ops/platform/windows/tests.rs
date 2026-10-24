use super::*;

fn forbidden(name: &str) -> String {
    match name_problem(name) {
        Some(NameProblem::Forbidden(reason)) => reason,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn name_rules_explain_each_forbidden_character() {
    assert_eq!(forbidden("a:b"), "Windows does not allow : in names");
    assert_eq!(forbidden("a?b"), "Windows does not allow ? in names");
    assert_eq!(forbidden("a\\b"), "Windows does not allow \\ in names");
}

#[test]
fn name_rules_reserve_device_names_with_or_without_an_extension() {
    assert_eq!(forbidden("CON"), "CON is reserved on Windows");
    assert_eq!(forbidden("nul.txt"), "NUL is reserved on Windows");
    assert_eq!(forbidden("com3"), "COM3 is reserved on Windows");
    assert_eq!(name_problem("COM10"), None);
    assert_eq!(name_problem("console"), None);
}

#[test]
fn name_rules_refuse_a_trailing_space_or_dot() {
    forbidden("a.");
    forbidden("a ");
    assert_eq!(name_problem(""), Some(NameProblem::Empty));
}

#[test]
fn name_rules_count_utf16_units() {
    assert_eq!(name_problem(&"x".repeat(255)), None);
    assert_eq!(name_problem(&"x".repeat(256)), Some(NameProblem::TooLong));
}

#[test]
fn free_bytes_and_volume_work_on_a_real_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert_eq!(
        volume_of(dir.path()).unwrap(),
        volume_of(&dir.path().join("not-yet")).unwrap()
    );
}
