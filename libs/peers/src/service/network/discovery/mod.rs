mod cleanup;
mod mdns;

use std::{future::Future, net::SocketAddrV4, pin::Pin};

use tokio::sync::{mpsc, watch};

use crate::{network::NetworkFailure, PeerId};

pub use mdns::MdnsDiscovery;

pub type DiscoveryFuture = Pin<Box<dyn Future<Output = Result<(), NetworkFailure>> + Send>>;

pub(super) const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone)]
pub struct Advertisement {
    pub peer: PeerId,
    pub port: u16,
    pub bind: std::net::Ipv4Addr,
    pub name: String,
    pub link: Option<u16>,
}

#[derive(Clone, Debug)]
pub enum DiscoveryEvent {
    Ready,
    Resolved {
        source: String,
        peer: PeerId,
        endpoints: Vec<SocketAddrV4>,
        claim: Option<NearbyAdvertisement>,
    },
    Removed {
        source: String,
    },
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NearbyAdvertisement {
    pub name: String,
    pub link: u16,
}

pub(crate) const MAX_ADVERTISED_NAME: usize = 200;

pub(crate) fn advertised_name(name: &str) -> String {
    let mut bounded = String::new();
    for character in name.chars() {
        if bounded.len() + character.len_utf8() > MAX_ADVERTISED_NAME {
            break;
        }
        bounded.push(character);
    }
    bounded.trim_end().to_owned()
}

pub trait DiscoveryFactory: Send + Sync {
    fn run(
        &self,
        advertisement: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture;
}

pub(crate) async fn cancelled(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}
