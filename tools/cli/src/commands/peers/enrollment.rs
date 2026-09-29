use std::{
    io::Read,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use anyhow::{anyhow, bail, Result};
use qol_peers::admin::{EnrollmentRequest, ExpectedAuthority, Request};
use qol_peers::enrollment::{
    EnrollmentRequestKey, ExportedInvitation, InvitationId, TransactionId, MAX_INVITATION_BYTES,
};
use zeroize::Zeroizing;

use super::input::{Action, Mutation};

#[derive(Debug, PartialEq)]
pub(super) enum EnrollmentAction {
    Invite(Vec<IpAddr>),
    Cancel(InvitationId),
    Approve(EnrollmentRequestKey),
    Reject(EnrollmentRequestKey),
    Prepare(ExportedInvitation),
    Redeem(TransactionId, ExportedInvitation),
    Recover(TransactionId, Vec<SocketAddr>),
    Abandon(TransactionId),
    Resume(TransactionId),
    Attempt(TransactionId),
}

impl EnrollmentAction {
    pub(super) fn request(self, expected: ExpectedAuthority) -> Request {
        let request = match self {
            Self::Invite(addresses) => EnrollmentRequest::CreateInvitation {
                expected,
                addresses,
            },
            Self::Cancel(invitation) => EnrollmentRequest::CancelInvitation {
                expected,
                invitation,
            },
            Self::Approve(key) => EnrollmentRequest::Approve { expected, key },
            Self::Reject(key) => EnrollmentRequest::Reject { expected, key },
            Self::Prepare(document) => EnrollmentRequest::Prepare { expected, document },
            Self::Redeem(transaction, document) => EnrollmentRequest::Redeem {
                expected,
                transaction,
                document,
            },
            Self::Recover(transaction, endpoints) => EnrollmentRequest::Recover {
                expected,
                transaction,
                endpoints,
            },
            Self::Abandon(transaction) => EnrollmentRequest::Abandon {
                expected,
                transaction,
            },
            Self::Resume(transaction) => EnrollmentRequest::Resume {
                expected,
                transaction,
            },
            Self::Attempt(transaction) => EnrollmentRequest::Attempt {
                expected,
                transaction,
            },
        };
        Request::Enrollment { request }
    }
}

pub(super) fn parse(args: &[&str], stdin: &mut impl Read) -> Result<Action> {
    let action = match args {
        ["network"] => return Ok(Action::Direct(Request::Network)),
        ["pending"] => {
            return Ok(Action::Direct(Request::Enrollment {
                request: EnrollmentRequest::Pending {},
            }))
        }
        ["outbound"] => return Ok(Action::Pages(super::pagination::Pages::Outbound)),
        ["invite", addresses @ ..] if !addresses.is_empty() && addresses.len() <= 8 => {
            EnrollmentAction::Invite(parse_all(addresses)?)
        }
        ["cancel-invitation", invitation] => EnrollmentAction::Cancel(identifier(invitation)?),
        ["approve", invitation, transaction, peer] => {
            EnrollmentAction::Approve(key(invitation, transaction, peer)?)
        }
        ["reject", invitation, transaction, peer] => {
            EnrollmentAction::Reject(key(invitation, transaction, peer)?)
        }
        ["prepare"] => EnrollmentAction::Prepare(document(stdin)?),
        ["redeem", transaction] => {
            EnrollmentAction::Redeem(identifier(transaction)?, document(stdin)?)
        }
        ["recover", transaction, endpoints @ ..]
            if !endpoints.is_empty() && endpoints.len() <= 8 =>
        {
            EnrollmentAction::Recover(identifier(transaction)?, parse_all(endpoints)?)
        }
        ["abandon", transaction] => EnrollmentAction::Abandon(identifier(transaction)?),
        ["resume", transaction] => EnrollmentAction::Resume(identifier(transaction)?),
        ["attempt", transaction] => EnrollmentAction::Attempt(identifier(transaction)?),
        _ => bail!("invalid peers arguments; run qol peers help"),
    };
    Ok(Action::Mutation(Mutation::Enrollment(action)))
}

fn identifier<T: FromStr>(text: &str) -> Result<T> {
    text.parse()
        .map_err(|_| anyhow!("invalid canonical enrollment identifier"))
}

fn parse_all<T: FromStr>(values: &[&str]) -> Result<Vec<T>> {
    values
        .iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| anyhow!("invalid literal enrollment address"))
        })
        .collect()
}

fn key(invitation: &str, transaction: &str, peer: &str) -> Result<EnrollmentRequestKey> {
    Ok(EnrollmentRequestKey {
        invitation: identifier(invitation)?,
        transaction: identifier(transaction)?,
        peer: identifier(peer)?,
    })
}

fn document(stdin: &mut impl Read) -> Result<ExportedInvitation> {
    let mut bytes = Zeroizing::new(vec![0; MAX_INVITATION_BYTES + 3]);
    let mut length = 0;
    loop {
        let count = match stdin.read(&mut bytes[length..]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| anyhow!("could not read invitation stdin"))?,
        };
        length += count;
        if length == bytes.len() {
            bail!("invitation stdin exceeds the document boundary");
        }
        if count == 0 {
            break;
        }
    }
    let value =
        std::str::from_utf8(&bytes[..length]).map_err(|_| anyhow!("invalid invitation stdin"))?;
    let value = value.strip_suffix('\n').unwrap_or(value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    ExportedInvitation::from_owned(Zeroizing::new(value.to_owned()))
        .map_err(|_| anyhow!("invalid invitation stdin"))
}
