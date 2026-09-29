use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{wire::Token, EnrollmentError, Invitation};
use crate::admin::LinkCode;
use crate::enrollment::{EnrollmentRejection, EnrollmentVersion, ExportedInvitation};
use crate::service::{
    framing::{read_json, write_json, FrameLimit},
    PeerAuthority, PeerConnection, PeerPin, SessionKind,
};
use crate::AuthorityLifetime;

const COMMIT_LABEL: &[u8] = b"qol-nearby/1 commit";
const CODE_LABEL: &[u8] = b"qol-nearby/1 code";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Offer {
    version: EnrollmentVersion,
    commitment: Token,
    #[serde(deserialize_with = "crate::enrollment::deserialize_name")]
    name: String,
    lifetime: AuthorityLifetime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    version: EnrollmentVersion,
    outcome: AnswerOutcome,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AnswerOutcome {
    Invitation {
        invitation: ExportedInvitation,
        nonce: Token,
    },
    Rejected {
        reason: EnrollmentRejection,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reveal {
    version: EnrollmentVersion,
    nonce: Token,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bound {
    version: EnrollmentVersion,
}

pub struct NearbyOffer {
    pub invitation: Invitation,
    pub code: LinkCode,
}

impl std::fmt::Debug for NearbyOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearbyOffer")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

impl PeerAuthority {
    pub async fn serve_nearby<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut connection: PeerConnection<S>,
        local: SocketAddr,
    ) -> Result<(), EnrollmentError> {
        let joiner = authenticated_pin(&connection)?;
        let offer: Offer = read_json(&mut connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let invitation = match self.offer_nearby(&joiner, offer.name, offer.lifetime, local) {
            Ok(invitation) => invitation,
            Err(EnrollmentError::Rejected(reason)) => {
                let answer = Answer {
                    version: EnrollmentVersion::V1,
                    outcome: AnswerOutcome::Rejected { reason },
                };
                write_json(&mut connection, &answer, FrameLimit::Enrollment)
                    .await
                    .map_err(|_| EnrollmentError::Framing)?;
                return Err(EnrollmentError::Rejected(reason));
            }
            Err(error) => return Err(error),
        };
        let id = invitation.id();
        let result = self
            .finish_nearby(&mut connection, &joiner, invitation, offer.commitment)
            .await;
        if result.is_err() {
            self.drop_nearby(id);
        }
        result
    }

    async fn finish_nearby<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        connection: &mut PeerConnection<S>,
        joiner: &PeerPin,
        invitation: Invitation,
        commitment: Token,
    ) -> Result<(), EnrollmentError> {
        let nonce = Token::random()?;
        let answer = Answer {
            version: EnrollmentVersion::V1,
            outcome: AnswerOutcome::Invitation {
                invitation: invitation.export()?,
                nonce: nonce.clone(),
            },
        };
        write_json(connection, &answer, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        drop(answer);
        let reveal: Reveal = read_json(connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        if !commit(&reveal.nonce).matches(&commitment) {
            return Err(EnrollmentError::Protocol);
        }
        let code = code(invitation.inviter_pin(), joiner, &nonce, &reveal.nonce);
        self.bind_nearby(invitation.id(), joiner, code)?;
        let bound = Bound {
            version: EnrollmentVersion::V1,
        };
        write_json(connection, &bound, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)
    }

    pub async fn request_nearby<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut connection: PeerConnection<S>,
    ) -> Result<NearbyOffer, EnrollmentError> {
        let inviter = authenticated_pin(&connection)?;
        self.check_nearby_remote(&inviter)?;
        let (local, name, lifetime) = self.nearby_identity()?;
        let nonce = Token::random()?;
        let offer = Offer {
            version: EnrollmentVersion::V1,
            commitment: commit(&nonce),
            name,
            lifetime,
        };
        write_json(&mut connection, &offer, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let answer: Answer = read_json(&mut connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let (document, theirs) = match answer.outcome {
            AnswerOutcome::Invitation { invitation, nonce } => (invitation, nonce),
            AnswerOutcome::Rejected { reason } => return Err(EnrollmentError::Rejected(reason)),
        };
        let invitation = Invitation::import(document.expose())?;
        if *invitation.inviter_pin() != inviter {
            return Err(EnrollmentError::Protocol);
        }
        let reveal = Reveal {
            version: EnrollmentVersion::V1,
            nonce: nonce.clone(),
        };
        write_json(&mut connection, &reveal, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        let _: Bound = read_json(&mut connection, FrameLimit::Enrollment)
            .await
            .map_err(|_| EnrollmentError::Framing)?;
        Ok(NearbyOffer {
            code: code(&inviter, &local, &theirs, &nonce),
            invitation,
        })
    }
}

fn authenticated_pin<S: AsyncRead + AsyncWrite + Unpin>(
    connection: &PeerConnection<S>,
) -> Result<PeerPin, EnrollmentError> {
    if connection.session_kind() != SessionKind::Nearby {
        return Err(EnrollmentError::Protocol);
    }
    Ok(connection.remote_identity().pin().clone())
}

fn commit(nonce: &Token) -> Token {
    Token::from_bytes(
        Sha256::new()
            .chain_update(COMMIT_LABEL)
            .chain_update(nonce.expose())
            .finalize()
            .into(),
    )
}

fn code(
    inviter: &PeerPin,
    joiner: &PeerPin,
    inviter_nonce: &Token,
    joiner_nonce: &Token,
) -> LinkCode {
    let digest = Sha256::new()
        .chain_update(CODE_LABEL)
        .chain_update(inviter.spki_der())
        .chain_update(joiner.spki_der())
        .chain_update(inviter_nonce.expose())
        .chain_update(joiner_nonce.expose())
        .finalize();
    LinkCode::reduce(u32::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3],
    ]))
}
