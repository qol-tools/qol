use qol_peers::admin::{
    AttemptState, EnrollmentFailure, EnrollmentRequest, Error, ExpectedAuthority, Response,
};
use qol_peers::service::{enrollment::Invitation, network::NetworkControl};
use qol_peers::StoreRevision;

use super::ActiveAuthority;

pub(super) fn preflight_addresses(
    request: &mut qol_peers::admin::Request,
    resolve: impl FnOnce() -> Result<Vec<std::net::IpAddr>, Error>,
) -> Result<(), Error> {
    if let qol_peers::admin::Request::Enrollment {
        request: EnrollmentRequest::CreateInvitation { addresses, .. },
    } = request
    {
        if addresses.is_empty() {
            let resolved = resolve()?;
            if resolved.is_empty() {
                return Err(qol_peers::admin::Error::Enrollment {
                    error: EnrollmentFailure::Unavailable,
                });
            }
            *addresses = resolved;
        }
    }
    Ok(())
}

pub(super) fn dispatch(
    active: &ActiveAuthority,
    request: EnrollmentRequest,
) -> Result<Response, Error> {
    match request {
        EnrollmentRequest::CreateInvitation {
            expected,
            addresses,
        } => {
            let authority = active.check_expected(expected)?;
            let endpoints = network(active)?.invitation_endpoints(addresses)?;
            let invitation = authority.create_invitation_checked(expected.revision, endpoints)?;
            Ok(Response::Invitation {
                authority: stamp(active)?,
                invitation: invitation.id(),
                document: invitation.export()?,
            })
        }
        EnrollmentRequest::CancelInvitation {
            expected,
            invitation,
        } => {
            active
                .check_expected(expected)?
                .cancel_invitation_checked(expected.revision, invitation)?;
            changed(active)
        }
        EnrollmentRequest::Pending {} => {
            let (revision, items) = active.authority.pending_enrollments_snapshot()?;
            Ok(Response::PendingEnrollments {
                authority: stamp_at(active, revision)?,
                items,
            })
        }
        EnrollmentRequest::Approve { expected, key } => {
            active
                .check_expected(expected)?
                .approve_enrollment(expected.revision, key)?;
            changed(active)
        }
        EnrollmentRequest::Reject { expected, key } => {
            active
                .check_expected(expected)?
                .reject_enrollment(expected.revision, key)?;
            changed(active)
        }
        EnrollmentRequest::Prepare { expected, document } => {
            let authority = active.check_expected(expected)?;
            let invitation = Invitation::import(document.expose())?;
            network(active)?.validate_invitation(&invitation)?;
            let transaction = authority.prepare_join(expected.revision, &invitation)?;
            Ok(Response::JoinPrepared {
                authority: stamp(active)?,
                transaction,
            })
        }
        EnrollmentRequest::Redeem {
            expected,
            transaction,
            document,
        } => {
            let authority = active.check_expected(expected)?;
            let state = network(active)?.admit_enrollment(
                authority,
                expected.revision,
                transaction,
                Some(document),
                Vec::new(),
            )?;
            Ok(Response::EnrollmentAttempt {
                authority: expected,
                transaction,
                state,
            })
        }
        EnrollmentRequest::Recover {
            expected,
            transaction,
            endpoints,
        } => {
            let authority = active.check_expected(expected)?;
            let state = network(active)?.admit_enrollment(
                authority,
                expected.revision,
                transaction,
                None,
                endpoints,
            )?;
            Ok(Response::EnrollmentAttempt {
                authority: expected,
                transaction,
                state,
            })
        }
        EnrollmentRequest::Abandon {
            expected,
            transaction,
        } => {
            active
                .check_expected(expected)?
                .abandon_join(expected.revision, transaction)?;
            if let Some(network) = &active.network {
                network.cancel_enrollment(transaction);
            }
            changed(active)
        }
        EnrollmentRequest::Resume {
            expected,
            transaction,
        } => {
            active
                .check_expected(expected)?
                .resume_join(expected.revision, transaction)?;
            changed(active)
        }
        EnrollmentRequest::Outbound { cursor } => {
            if active.activation_id != cursor.activation_id {
                return Err(Error::StaleCursor);
            }
            Ok(Response::OutboundEnrollments {
                page: active.authority.outbound_enrollment_page(cursor)?,
            })
        }
        EnrollmentRequest::Attempt {
            expected,
            transaction,
        } => {
            active
                .check_expected(expected)?
                .check_enrollment_transaction(transaction)?;
            let state = match &active.network {
                Some(network) => network.enrollment_attempt(transaction)?,
                None => AttemptState::Unavailable {},
            };
            Ok(Response::EnrollmentAttempt {
                authority: stamp(active)?,
                transaction,
                state,
            })
        }
    }
}

pub(super) fn network(active: &ActiveAuthority) -> Result<&NetworkControl, Error> {
    active.network.as_ref().ok_or(Error::Enrollment {
        error: EnrollmentFailure::Unavailable,
    })
}

fn stamp_at(active: &ActiveAuthority, revision: StoreRevision) -> Result<ExpectedAuthority, Error> {
    Ok(ExpectedAuthority {
        authority_id: active.authority.local_pin()?.peer_id(),
        activation_id: active.activation_id,
        revision,
    })
}

pub(super) fn stamp(active: &ActiveAuthority) -> Result<ExpectedAuthority, Error> {
    stamp_at(active, active.authority.projection()?.revision)
}

pub(super) fn changed(active: &ActiveAuthority) -> Result<Response, Error> {
    let stamp = stamp(active)?;
    Ok(Response::Changed {
        authority_id: stamp.authority_id,
        activation_id: stamp.activation_id,
        revision: stamp.revision,
    })
}

pub(crate) fn attempt_name(state: &AttemptState) -> &'static str {
    match state {
        AttemptState::Unavailable {} => "unavailable",
        AttemptState::Queued {} => "queued",
        AttemptState::Running {} => "running",
        AttemptState::Completed { .. } => "completed",
        AttemptState::Rejected { .. } => "rejected",
        AttemptState::Unknown { .. } => "unknown",
    }
}

pub(super) fn trace_requested(request: &EnrollmentRequest) {
    match request {
        EnrollmentRequest::Approve { expected, key }
        | EnrollmentRequest::Reject { expected, key } => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=enrollment_requested action={} activation={} transaction={} peer={}",
                request.action_name(),
                expected.activation_id,
                key.transaction,
                key.peer
            );
        }
        EnrollmentRequest::Redeem {
            expected,
            transaction,
            ..
        }
        | EnrollmentRequest::Recover {
            expected,
            transaction,
            ..
        }
        | EnrollmentRequest::Abandon {
            expected,
            transaction,
        }
        | EnrollmentRequest::Resume {
            expected,
            transaction,
        }
        | EnrollmentRequest::Attempt {
            expected,
            transaction,
        } => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=enrollment_requested action={} activation={} transaction={}",
                request.action_name(),
                expected.activation_id,
                transaction
            );
        }
        EnrollmentRequest::CreateInvitation { .. }
        | EnrollmentRequest::CancelInvitation { .. }
        | EnrollmentRequest::Pending {}
        | EnrollmentRequest::Prepare { .. }
        | EnrollmentRequest::Outbound { .. } => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=enrollment_requested action={}",
                request.action_name()
            );
        }
    }
}
