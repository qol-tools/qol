//! Structural invariants for the pull-API capability gate.
//!
//! The broker enforces field-level access on every `GET /panes/<id>`
//! call: a plugin sees only the fields its manifest declared, with
//! `foreground.exe` and `foreground.pid` always allowed without
//! declaration. These tests pin the wire vocabulary (PaneField rename
//! strings), the closed-set property of the enum (unknown fields fail
//! to parse), and the always-allowed list.
//!
//! qol-runtime owns the canonical `PaneField` definition because
//! qol-plugin-api already depends on qol-runtime (the reverse
//! direction would cycle), so the enum lives in `src/pane_field.rs`
//! and qol-plugin-api re-exports it for plugin manifest parsing. This
//! test pins the wire vocabulary at the source of truth; qol-plugin-api
//! has a companion test that confirms the re-export is visible to
//! plugins.
//!
//! Refs: workspace/docs/superpowers/plans/2026-05-12-terminal-workspace-restore-security-plan.md (card 07)
//! Closes: RUNTIME-1.2, RUNTIME-1.3.

#![cfg(unix)]

use qol_runtime::broker::{check_field_access, FieldCheckError, PaneField};

#[test]
fn pane_field_enum_is_closed() {
    // Unknown variants must fail to parse instead of silently becoming
    // a wildcard. The capability gate relies on this property: every
    // request maps to exactly one enum variant or is rejected.
    let unknown: Result<PaneField, _> =
        serde_json::from_value(serde_json::Value::String("foreground.uid".into()));
    assert!(
        unknown.is_err(),
        "PaneField parsed an unknown variant `foreground.uid`. \
         Add it to the enum if intentional; otherwise the closed-set \
         guarantee is broken."
    );
}

#[test]
fn pane_field_wire_vocabulary_matches_capability_decls() {
    // The string names must be identical to the qol-plugin-api
    // capability::PaneField rename strings, because plugins declare
    // them in plugin.toml and the broker consumes the declarations
    // by string match. This test pins each pairing.
    let cases = [
        (PaneField::ForegroundExe, "foreground.exe"),
        (PaneField::ForegroundPid, "foreground.pid"),
        (PaneField::ForegroundCwd, "foreground.cwd"),
        (PaneField::ForegroundArgv, "foreground.argv"),
        (PaneField::ForegroundArgvFull, "foreground.argv-full"),
        (PaneField::Title, "title"),
        (PaneField::Cwd, "cwd"),
    ];
    for (variant, wire) in cases {
        let v = serde_json::to_value(variant).expect("serialize PaneField");
        assert_eq!(
            v,
            serde_json::Value::String(wire.to_string()),
            "PaneField::{variant:?} did not serialize to `{wire}`; \
             the wire vocabulary diverged from the qol-plugin-api capability decl"
        );
    }
}

#[test]
fn always_allowed_fields_pass_check_with_empty_declarations() {
    // foreground.exe and foreground.pid are spec'd as always allowed,
    // regardless of what the plugin declared. The check must pass even
    // when `declared` is empty.
    let declared: &[PaneField] = &[];
    assert!(
        check_field_access(declared, PaneField::ForegroundExe).is_ok(),
        "foreground.exe was denied with empty declared fields; \
         the always-allowed contract requires it to pass"
    );
    assert!(
        check_field_access(declared, PaneField::ForegroundPid).is_ok(),
        "foreground.pid was denied with empty declared fields; \
         the always-allowed contract requires it to pass"
    );
}

#[test]
fn gated_fields_require_explicit_declaration() {
    // foreground.cwd, title, cwd, and foreground.argv all require
    // their corresponding declaration. An empty declaration list
    // must produce NotDeclared with the requested field echoed back
    // so a higher layer can map it to an HTTP 403 body.
    let declared: &[PaneField] = &[];
    for f in [
        PaneField::ForegroundCwd,
        PaneField::Title,
        PaneField::Cwd,
        PaneField::ForegroundArgv,
    ] {
        match check_field_access(declared, f) {
            Err(FieldCheckError::NotDeclared { field }) => {
                assert_eq!(
                    field, f,
                    "FieldCheckError::NotDeclared echoed the wrong field: {field:?} vs {f:?}"
                );
            }
            other => panic!(
                "check_field_access({f:?}) with empty declared returned {other:?}; \
                 expected NotDeclared"
            ),
        }
    }
}

#[test]
fn declared_field_passes_check() {
    // The happy path: a plugin that declared a field can read it.
    let declared = [PaneField::ForegroundCwd];
    assert!(
        check_field_access(&declared, PaneField::ForegroundCwd).is_ok(),
        "check_field_access denied a field the plugin had declared; \
         the capability gate is rejecting legitimate access"
    );
    // A field that the plugin did NOT declare still gets denied even
    // when other (unrelated) fields are declared.
    assert!(
        matches!(
            check_field_access(&declared, PaneField::Title),
            Err(FieldCheckError::NotDeclared { .. })
        ),
        "declaring foreground.cwd implicitly granted title; \
         the gate must remain field-scoped, not bundle-scoped"
    );
}
