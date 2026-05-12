//! Structural invariant for the ForegroundArgvFull double-gate.
//!
//! A plugin that wants the full, untruncated argv (every element)
//! must declare BOTH `ForegroundArgv` and `ForegroundArgvFull`. The
//! split is deliberate: a plugin that only needs the program path
//! ("argv[0] basename") gets argv truncation by default, so secrets
//! that may appear in later argv elements (`--token=...`) never
//! reach a plugin that did not opt in to see them.
//!
//! The single capability gate from Round 3 enforces "declared in the
//! manifest". This file pins the COMBINATION rule: declaring
//! `ForegroundArgvFull` alone is not enough. It is a separate test
//! file so the invariant is visible on its own.
//!
//! Refs: workspace/docs/superpowers/plans/2026-05-12-terminal-workspace-restore-security-plan.md (card 07, "Defense in depth")
//! Closes: RUNTIME-1.2 (argv-truncation sub-invariant).

#![cfg(unix)]

use qol_runtime::broker::{check_field_access, FieldCheckError, PaneField};

#[test]
fn argv_full_alone_is_rejected_with_missing_companion() {
    // A plugin that declares ONLY foreground.argv-full but not
    // foreground.argv must be denied with a distinct error variant.
    // This is the structural difference from "field not declared":
    // the field IS declared, but its companion requirement is missing.
    let declared = [PaneField::ForegroundArgvFull];
    let result = check_field_access(&declared, PaneField::ForegroundArgvFull);
    match result {
        Err(FieldCheckError::MissingCompanion { field, requires }) => {
            assert_eq!(field, PaneField::ForegroundArgvFull);
            assert_eq!(
                requires,
                PaneField::ForegroundArgv,
                "ForegroundArgvFull's companion requirement is ForegroundArgv; \
                 the gate is enforcing the wrong companion"
            );
        }
        other => panic!(
            "check_field_access(argv-full alone) returned {other:?}; \
             expected MissingCompanion {{ field: ForegroundArgvFull, requires: ForegroundArgv }}"
        ),
    }
}

#[test]
fn argv_full_with_argv_companion_passes() {
    // The happy path: both ForegroundArgv and ForegroundArgvFull declared.
    let declared = [PaneField::ForegroundArgv, PaneField::ForegroundArgvFull];
    assert!(
        check_field_access(&declared, PaneField::ForegroundArgvFull).is_ok(),
        "check_field_access denied argv-full when both companion fields \
         were declared; the double-gate must permit this combination"
    );
    // And ForegroundArgv on its own (the more restrictive sibling) is
    // also accessible when both are declared.
    assert!(
        check_field_access(&declared, PaneField::ForegroundArgv).is_ok(),
        "check_field_access denied argv when argv-full was also declared; \
         the double-gate must not break the simpler ForegroundArgv path"
    );
}

#[test]
fn argv_without_argv_full_does_not_grant_full_argv() {
    // Declaring `ForegroundArgv` alone gives access to ForegroundArgv
    // (truncated form) but NOT to ForegroundArgvFull. The structural
    // split is what protects later argv elements from a
    // program-path-only plugin.
    let declared = [PaneField::ForegroundArgv];
    assert!(
        check_field_access(&declared, PaneField::ForegroundArgv).is_ok(),
        "check_field_access denied argv even though it was declared"
    );
    match check_field_access(&declared, PaneField::ForegroundArgvFull) {
        Err(FieldCheckError::NotDeclared { field }) => {
            assert_eq!(field, PaneField::ForegroundArgvFull);
        }
        other => panic!(
            "check_field_access(argv-full) with argv-only declared returned {other:?}; \
             expected NotDeclared {{ field: ForegroundArgvFull }}"
        ),
    }
}
