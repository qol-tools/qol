use super::*;
use crate::enrollment::EnrollmentRejection;
use crate::service::enrollment::wire::Confirmation;
use serde::{de::DeserializeOwned, Serialize};

fn strict_object<T: DeserializeOwned + Serialize>(fixture: &str) {
    let parsed: T = serde_json::from_str(fixture).unwrap();
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    for extra in [r#""extra":true"#, r#""secret":"hidden""#] {
        let invalid = format!("{{{extra},{}", &fixture[1..]);
        assert!(serde_json::from_str::<T>(&invalid).is_err(), "{extra}");
    }
    for (field, value) in value.as_object().unwrap() {
        for duplicate in [
            serde_json::to_string(field).unwrap(),
            format!(r#""\u{:04x}{}""#, field.as_bytes()[0], &field[1..]),
        ] {
            let invalid = format!("{{{duplicate}:{value},{}", &fixture[1..]);
            assert!(serde_json::from_str::<T>(&invalid).is_err(), "{field}");
        }
    }
}

#[test]
fn every_request_variant_rejects_extra_fields_and_duplicate_keys() {
    let inviter = session("inviter");
    let invitation = inviter.create_invitation(vec![]).unwrap();
    let transaction = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    for operation in [
        request(&invitation, transaction, AuthorityLifetime::Session).operation,
        RequestOperation::Recover {},
    ] {
        let operation = serde_json::to_string(&operation).unwrap();
        strict_object::<RequestOperation>(&operation);
        let fixture = format!(
            r#"{{"version":1,"invitation":"{}","transaction":"{transaction}","operation":{operation}}}"#,
            invitation.id()
        );
        strict_object::<Request>(&fixture);
        for version in ["0", "2", "1.0", "\"1\"", "null"] {
            let invalid = fixture.replacen("\"version\":1", &format!("\"version\":{version}"), 1);
            assert!(
                serde_json::from_str::<Request>(&invalid).is_err(),
                "{version}"
            );
        }
    }
    for fixture in [r#"{"kind":"invoke"}"#, r#"{}"#, r#"{"kind":null}"#] {
        assert!(serde_json::from_str::<RequestOperation>(fixture).is_err());
    }
}

#[test]
fn every_response_variant_and_confirmation_reject_extra_fields_and_duplicate_keys() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let receipt = EnrollmentReceipt {
        invitation: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
        transaction: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
        inviter: inviter.local_pin().unwrap().peer_id(),
        joiner: joiner.local_pin().unwrap().peer_id(),
        inviter_name: "inviter".into(),
        joiner_name: "joiner".into(),
        inviter_lifetime: AuthorityLifetime::Session,
        joiner_lifetime: AuthorityLifetime::Session,
    };
    for (kind, outcome) in [
        ("pending", ResponseOutcome::Pending {}),
        ("unknown", ResponseOutcome::Unknown {}),
        (
            "committed",
            ResponseOutcome::Committed {
                receipt: receipt.clone(),
            },
        ),
        (
            "confirmed",
            ResponseOutcome::Confirmed {
                receipt: receipt.clone(),
            },
        ),
        (
            "rejected",
            ResponseOutcome::Rejected {
                reason: EnrollmentRejection::Conflict,
            },
        ),
    ] {
        let fixture = serde_json::to_string(&outcome).unwrap();
        strict_object::<ResponseOutcome>(&fixture);
        if matches!(
            outcome,
            ResponseOutcome::Pending {} | ResponseOutcome::Unknown {}
        ) {
            assert_eq!(fixture, format!(r#"{{"kind":"{kind}"}}"#));
            for extra in [r#""receipt":null"#, r#""reason":"conflict""#] {
                let invalid = format!("{{{extra},{}", &fixture[1..]);
                assert!(
                    serde_json::from_str::<ResponseOutcome>(&invalid).is_err(),
                    "{kind}"
                );
            }
        }
        let response = Response {
            version: EnrollmentVersion::V1,
            invitation: receipt.invitation,
            transaction: receipt.transaction,
            outcome,
        };
        strict_object::<Response>(&serde_json::to_string(&response).unwrap());
    }
    strict_object::<EnrollmentReceipt>(&serde_json::to_string(&receipt).unwrap());
    let confirmation = Confirmation {
        version: EnrollmentVersion::V1,
        receipt,
    };
    strict_object::<Confirmation>(&serde_json::to_string(&confirmation).unwrap());
    for fixture in [r#"{"kind":"completed"}"#, r#"{}"#, r#"{"kind":null}"#] {
        assert!(serde_json::from_str::<ResponseOutcome>(fixture).is_err());
    }
}

#[test]
fn outbound_state_schema_requires_one_strict_tag_and_only_committed_carries_receipt() {
    let inviter = session("inviter");
    let joiner = session("joiner");
    let receipt = EnrollmentReceipt {
        invitation: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
        transaction: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
        inviter: inviter.local_pin().unwrap().peer_id(),
        joiner: joiner.local_pin().unwrap().peer_id(),
        inviter_name: "inviter".into(),
        joiner_name: "joiner".into(),
        inviter_lifetime: AuthorityLifetime::Session,
        joiner_lifetime: AuthorityLifetime::Session,
    };
    for (kind, state) in [
        ("pending", OutboundEnrollmentState::Pending {}),
        ("abandoned", OutboundEnrollmentState::Abandoned {}),
        (
            "committed",
            OutboundEnrollmentState::Committed {
                receipt: receipt.clone(),
            },
        ),
    ] {
        let fixture = serde_json::to_string(&state).unwrap();
        strict_object::<OutboundEnrollmentState>(&fixture);
        if kind == "committed" {
            assert_eq!(
                serde_json::to_value(&state).unwrap(),
                serde_json::json!({"kind":"committed", "receipt":receipt})
            );
        } else {
            assert_eq!(fixture, format!(r#"{{"kind":"{kind}"}}"#));
            for receipt in ["null".to_string(), serde_json::to_string(&receipt).unwrap()] {
                let invalid = format!(r#"{{"kind":"{kind}","receipt":{receipt}}}"#);
                assert!(
                    serde_json::from_str::<OutboundEnrollmentState>(&invalid).is_err(),
                    "{kind}: unexpected receipt"
                );
            }
        }
    }
    for invalid in [
        r#"{}"#,
        r#"null"#,
        r#"{"kind":null}"#,
        r#"{"kind":"cancelled"}"#,
        r#"{"kind":"committed"}"#,
        r#"{"kind":"committed","receipt":null}"#,
        r#"{"kind":"pending","kind":"abandoned"}"#,
        r#"{"kind":"abandoned","\u006bind":"pending"}"#,
    ] {
        assert!(
            serde_json::from_str::<OutboundEnrollmentState>(invalid).is_err(),
            "{invalid}"
        );
    }
}
