use std::io::Read;

use anyhow::{anyhow, bail, Result};
use qol_conventions::operations::OperationKey;
use qol_peers::admin::{ExpectedAuthority, Request};
use qol_peers::PeerId;
use qol_runtime::local_ipc::MAX_MESSAGE_BYTES;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use super::pagination::Pages;

#[derive(Debug, PartialEq)]
pub(super) enum Action {
    Direct(Request),
    Start(Request),
    Pages(Pages),
    Mutation(Mutation),
}

#[derive(Debug, PartialEq)]
pub(super) enum Mutation {
    Enrollment(super::enrollment::EnrollmentAction),
    Stop,
    Rename(String),
    Revoke(PeerId),
    SetGrants(PeerId, Vec<OperationKey>),
}

impl Mutation {
    pub(super) fn request(self, expected: ExpectedAuthority) -> Request {
        match self {
            Self::Enrollment(action) => action.request(expected),
            Self::Stop => Request::Stop { expected },
            Self::Rename(name) => Request::Rename { expected, name },
            Self::Revoke(peer_id) => Request::Revoke { expected, peer_id },
            Self::SetGrants(peer_id, grants) => Request::SetGrants {
                expected,
                peer_id,
                grants,
            },
        }
    }
}

pub(super) fn parse(args: &[&str], stdin: &mut impl Read) -> Result<Action> {
    Ok(match args {
        ["status"] => Action::Direct(Request::Status),
        ["list"] => Action::Pages(Pages::Peers),
        ["grants", peer] => Action::Pages(Pages::Grants(parse_peer(peer)?)),
        ["tombstones"] => Action::Pages(Pages::Tombstones),
        ["session", name] => Action::Start(Request::StartSession {
            name: name.to_string(),
        }),
        ["create", name] => Action::Start(Request::CreatePersistent {
            name: name.to_string(),
        }),
        ["open"] => Action::Start(Request::OpenPersistent),
        ["stop"] => Action::Mutation(Mutation::Stop),
        ["rename", name] => Action::Mutation(Mutation::Rename(name.to_string())),
        ["revoke", peer] => Action::Mutation(Mutation::Revoke(parse_peer(peer)?)),
        ["set-grants", peer] => {
            Action::Mutation(Mutation::SetGrants(parse_peer(peer)?, read_json(stdin)?))
        }
        ["request"] => Action::Direct(read_json(stdin)?),
        _ => return super::enrollment::parse(args, stdin),
    })
}

fn parse_peer(value: &str) -> Result<PeerId> {
    value
        .parse()
        .map_err(|_| anyhow!("invalid canonical peer identifier"))
}

pub(super) fn read_json<T: DeserializeOwned>(stdin: &mut impl Read) -> Result<T> {
    let mut bytes = Zeroizing::new(vec![0; MAX_MESSAGE_BYTES + 1]);
    let mut length = 0;
    loop {
        let count = match stdin.read(&mut bytes[length..]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|_| anyhow!("could not read peer administration stdin"))?,
        };
        length += count;
        if length > MAX_MESSAGE_BYTES {
            bail!("peer administration stdin exceeds the local message boundary");
        }
        if count == 0 {
            return serde_json::from_slice(&bytes[..length])
                .map_err(|_| anyhow!("invalid canonical peer administration JSON on stdin"));
        }
    }
}
