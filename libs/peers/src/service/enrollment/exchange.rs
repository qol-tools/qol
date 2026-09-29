use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::time::{Instant, MissedTickBehavior};

use super::{
    wire::{Confirmation, Request, RequestOperation, Response, ResponseOutcome},
    EnrollmentError, EnrollmentOutcome, Invitation,
};
use crate::enrollment::{
    EnrollmentReceipt, EnrollmentRejection, EnrollmentRequestKey, EnrollmentVersion, TransactionId,
};
use crate::service::{
    framing::{read_json, write_json, FrameLimit},
    PeerAuthority, PeerConnection, PeerPin, SessionKind,
};

impl PeerAuthority {
    pub async fn serve_enrollment<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut connection: PeerConnection<S>,
    ) -> Result<EnrollmentOutcome, EnrollmentError> {
        let pin = authenticated_pin(&connection)?;
        let request: Request = read_json(&mut connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let transaction = request.transaction;
        let key = EnrollmentRequestKey {
            invitation: request.invitation,
            transaction,
            peer: pin.peer_id(),
        };
        let decision = match self.reserve_enrollment(&pin, &request) {
            Ok(()) => self.wait_and_signal(&mut connection, &pin, key).await,
            Err(error) => Err(error),
        };
        drop(request);
        let receipt = match decision {
            Ok(receipt) => receipt,
            Err(EnrollmentError::Rejected(EnrollmentRejection::UnknownTransaction)) => {
                let response = Response {
                    version: EnrollmentVersion::V1,
                    invitation: key.invitation,
                    transaction,
                    outcome: ResponseOutcome::Unknown {},
                };
                let reason = match write_json(&mut connection, &response, FrameLimit::Enrollment)
                    .await
                {
                    Ok(()) => EnrollmentError::Rejected(EnrollmentRejection::UnknownTransaction),
                    Err(_) => EnrollmentError::Framing,
                };
                return Ok(unknown(transaction, reason));
            }
            Err(EnrollmentError::Rejected(reason)) => {
                let response = Response {
                    version: EnrollmentVersion::V1,
                    invitation: key.invitation,
                    transaction,
                    outcome: ResponseOutcome::Rejected { reason },
                };
                write_json(&mut connection, &response, FrameLimit::Enrollment)
                    .await
                    .map_err(|_| EnrollmentError::Framing)?;
                return Ok(EnrollmentOutcome::Rejected(reason));
            }
            Err(error) => return Ok(unknown(transaction, error)),
        };
        let result = self.send_commit(&mut connection, &pin, key, &receipt).await;
        Ok(match result {
            Ok(()) => EnrollmentOutcome::Completed(receipt),
            Err(error) => unknown(transaction, error),
        })
    }

    pub async fn redeem_enrollment<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut connection: PeerConnection<S>,
        invitation: &Invitation,
        transaction: TransactionId,
    ) -> Result<EnrollmentOutcome, EnrollmentError> {
        let pin = authenticated_pin(&connection)?;
        if pin != *invitation.inviter_pin() {
            return Err(EnrollmentError::Protocol);
        }
        let pending = self.outbound_join(transaction, &pin)?;
        if pending.invitation != invitation.id() || pending.remote_lifetime != invitation.lifetime()
        {
            return Err(EnrollmentError::Protocol);
        }
        let request = Request {
            version: EnrollmentVersion::V1,
            invitation: pending.invitation,
            transaction,
            operation: RequestOperation::Redeem {
                secret: invitation.document.secret.clone(),
                name: pending.name,
                lifetime: pending.local_lifetime,
            },
        };
        Ok(self.exchange_join(&mut connection, &pin, request).await)
    }

    pub async fn recover_enrollment<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut connection: PeerConnection<S>,
        transaction: TransactionId,
    ) -> Result<EnrollmentOutcome, EnrollmentError> {
        let pin = authenticated_pin(&connection)?;
        let pending = self.outbound_join(transaction, &pin)?;
        let request = Request {
            version: EnrollmentVersion::V1,
            invitation: pending.invitation,
            transaction,
            operation: RequestOperation::Recover {},
        };
        Ok(self.exchange_join(&mut connection, &pin, request).await)
    }

    async fn exchange_join<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut PeerConnection<S>,
        pin: &PeerPin,
        request: Request,
    ) -> EnrollmentOutcome {
        let transaction = request.transaction;
        let key = EnrollmentRequestKey {
            invitation: request.invitation,
            transaction,
            peer: pin.peer_id(),
        };
        if write_json(connection, &request, FrameLimit::Enrollment)
            .await
            .is_err()
        {
            return unknown(transaction, EnrollmentError::Framing);
        }
        drop(request);
        let deadline = Instant::now() + Duration::from_secs(120);
        let result = match tokio::time::timeout_at(
            deadline,
            self.receive_commit(connection, pin, key),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => return unknown(transaction, EnrollmentError::Framing),
        };
        match result {
            Ok(outcome) => outcome,
            Err(error) => unknown(transaction, error),
        }
    }

    async fn receive_commit<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut S,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
    ) -> Result<EnrollmentOutcome, EnrollmentError> {
        let response = self.read_decision(connection, pin, key).await?;
        let receipt = match response.outcome {
            ResponseOutcome::Committed { receipt } => receipt,
            ResponseOutcome::Rejected { reason } => return Ok(EnrollmentOutcome::Rejected(reason)),
            ResponseOutcome::Unknown {} => {
                return Ok(unknown(
                    key.transaction,
                    EnrollmentError::Rejected(EnrollmentRejection::UnknownTransaction),
                ))
            }
            ResponseOutcome::Pending {} | ResponseOutcome::Confirmed { .. } => {
                return Err(EnrollmentError::Protocol)
            }
        };
        if receipt.transaction != key.transaction || receipt.invitation != key.invitation {
            return Err(EnrollmentError::Protocol);
        }
        self.commit_join(pin, &receipt)?;
        let confirmation = Confirmation {
            version: EnrollmentVersion::V1,
            receipt: receipt.clone(),
        };
        write_json(connection, &confirmation, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let response: Response = read_json(connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        check_response(&response, key)?;
        let ResponseOutcome::Confirmed { receipt: confirmed } = response.outcome else {
            return Err(EnrollmentError::Protocol);
        };
        if confirmed != receipt {
            return Err(EnrollmentError::Protocol);
        }
        self.commit_join(pin, &receipt)?;
        Ok(EnrollmentOutcome::Completed(receipt))
    }

    async fn read_decision<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut S,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
    ) -> Result<Response, EnrollmentError> {
        for _ in 0..32 {
            let response: Response = read_json(connection, FrameLimit::Enrollment)
                .await
                .map_err(|_| EnrollmentError::Framing)?;
            check_response(&response, key)?;
            self.outbound_join(key.transaction, pin)?;
            if !matches!(response.outcome, ResponseOutcome::Pending {}) {
                return Ok(response);
            }
        }
        Err(EnrollmentError::Protocol)
    }

    async fn wait_and_signal<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut S,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
    ) -> Result<EnrollmentReceipt, EnrollmentError> {
        let approval = self.wait_enrollment(pin, key);
        tokio::pin!(approval);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(4));
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let pending = Response {
            version: EnrollmentVersion::V1,
            invitation: key.invitation,
            transaction: key.transaction,
            outcome: ResponseOutcome::Pending {},
        };
        let mut probe = [0_u8; 1];
        loop {
            tokio::select! {
                biased;
                result = &mut approval => return result,
                _ = heartbeat.tick() => {
                    if write_json(connection, &pending, FrameLimit::Enrollment).await.is_err() {
                        break;
                    }
                }
                _ = connection.read(&mut probe) => break,
            }
        }
        self.abandon_nearby(key);
        Err(EnrollmentError::Framing)
    }

    async fn send_commit<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut PeerConnection<S>,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
        receipt: &EnrollmentReceipt,
    ) -> Result<(), EnrollmentError> {
        if self.checked_inbound_receipt(pin, key)? != *receipt {
            return Err(EnrollmentError::Protocol);
        }
        let mut response = Response {
            version: EnrollmentVersion::V1,
            invitation: key.invitation,
            transaction: key.transaction,
            outcome: ResponseOutcome::Committed {
                receipt: receipt.clone(),
            },
        };
        write_json(connection, &response, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let confirmation: Confirmation = read_json(connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        if confirmation.receipt != *receipt || self.checked_inbound_receipt(pin, key)? != *receipt {
            return Err(EnrollmentError::Protocol);
        }
        response.outcome = ResponseOutcome::Confirmed {
            receipt: receipt.clone(),
        };
        write_json(connection, &response, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        self.checked_inbound_receipt(pin, key)?;
        Ok(())
    }
}

fn authenticated_pin<S: AsyncRead + AsyncWrite + Unpin>(
    connection: &PeerConnection<S>,
) -> Result<PeerPin, EnrollmentError> {
    if connection.session_kind() != SessionKind::Enrollment {
        return Err(EnrollmentError::Protocol);
    }
    Ok(connection.remote_identity().pin().clone())
}

fn check_response(response: &Response, key: EnrollmentRequestKey) -> Result<(), EnrollmentError> {
    if response.invitation != key.invitation || response.transaction != key.transaction {
        return Err(EnrollmentError::Protocol);
    }
    Ok(())
}

fn unknown(transaction: TransactionId, reason: EnrollmentError) -> EnrollmentOutcome {
    EnrollmentOutcome::Unknown {
        transaction,
        reason,
    }
}
