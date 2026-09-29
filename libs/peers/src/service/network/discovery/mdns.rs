use mdns_sd::{DaemonEvent, IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::{mpsc, watch};

use super::cleanup::close;
use super::{
    cancelled, Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture,
    NearbyAdvertisement, MAX_ADVERTISED_NAME,
};
use crate::network::NetworkFailure;

const SERVICE: &str = "_qol-peer._tcp.local.";

pub struct MdnsDiscovery;

impl DiscoveryFactory for MdnsDiscovery {
    fn run(
        &self,
        advertisement: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        Box::pin(async move {
            let daemon = match ServiceDaemon::new() {
                Ok(daemon) => daemon,
                Err(_) => {
                    let _ = events.send(DiscoveryEvent::Failed).await;
                    return Ok(());
                }
            };
            let fullname = format!("{}.{}", advertisement.peer, SERVICE);
            {
                let running = serve(&daemon, advertisement, &events);
                tokio::pin!(running);
                tokio::select! {
                    biased;
                    () = cancelled(&mut stop) => {},
                    result = &mut running => {
                        if result.is_err() {
                            tokio::select! {
                                () = cancelled(&mut stop) => {},
                                _ = events.send(DiscoveryEvent::Failed) => {},
                            }
                        }
                    },
                }
            }
            close(&daemon, &fullname).await
        })
    }
}

async fn serve(
    daemon: &ServiceDaemon,
    advertisement: Advertisement,
    events: &mpsc::Sender<DiscoveryEvent>,
) -> Result<(), NetworkFailure> {
    let monitor = daemon.monitor().map_err(|_| NetworkFailure::Discovery)?;
    daemon
        .disable_interface(IfKind::IPv6)
        .map_err(|_| NetworkFailure::Discovery)?;
    daemon
        .disable_interface(IfKind::LoopbackV4)
        .map_err(|_| NetworkFailure::Discovery)?;
    daemon
        .disable_interface(IfKind::LoopbackV6)
        .map_err(|_| NetworkFailure::Discovery)?;
    let peer = advertisement.peer.to_string();
    let digest: String = advertisement
        .peer
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let hostname = format!("qol-{}.{}.local.", &digest[..32], &digest[32..]);
    let link = advertisement.link.map(|port| port.to_string());
    let mut properties = vec![("version", "1"), ("peer_id", peer.as_str())];
    if let Some(link) = &link {
        properties.push(("name", advertisement.name.as_str()));
        properties.push(("link", link.as_str()));
    }
    let mut info = ServiceInfo::new(
        SERVICE,
        &peer,
        &hostname,
        "",
        advertisement.port,
        &properties[..],
    )
    .map_err(|_| NetworkFailure::Discovery)?
    .enable_addr_auto();
    let interface = if advertisement.bind.is_unspecified() {
        IfKind::IPv4
    } else {
        IfKind::Addr(advertisement.bind.into())
    };
    info.set_interfaces(vec![interface]);
    daemon
        .register(info)
        .map_err(|_| NetworkFailure::Discovery)?;
    let browse = daemon
        .browse(SERVICE)
        .map_err(|_| NetworkFailure::Discovery)?;
    loop {
        let event = tokio::select! {
            event = monitor.recv_async() => match event.map_err(|_| NetworkFailure::Discovery)? {
                DaemonEvent::Announce(..) => Some(DiscoveryEvent::Ready),
                DaemonEvent::Error(_) => return Err(NetworkFailure::Discovery),
                _ => None,
            },
            event = browse.recv_async() => match event.map_err(|_| NetworkFailure::Discovery)? {
                ServiceEvent::ServiceResolved(info) => info
                    .get_property_val_str("version")
                    .filter(|version| *version == "1")
                    .and_then(|_| info.get_property_val_str("peer_id"))
                    .and_then(|peer| peer.parse().ok())
                    .map(|peer| DiscoveryEvent::Resolved {
                        source: info.get_fullname().to_owned(),
                        peer,
                        endpoints: info.get_addresses_v4().into_iter().take(8)
                            .map(|address| std::net::SocketAddrV4::new(std::net::Ipv4Addr::from(address.octets()), info.get_port()))
                            .collect(),
                        claim: nearby(&info),
                    }),
                ServiceEvent::ServiceRemoved(_, source) => Some(DiscoveryEvent::Removed { source }),
                _ => None,
            },
        };
        if let Some(event) = event {
            events
                .send(event)
                .await
                .map_err(|_| NetworkFailure::Discovery)?;
        }
    }
}

fn nearby(info: &ResolvedService) -> Option<NearbyAdvertisement> {
    let name = info.get_property_val_str("name")?;
    let link = info.get_property_val_str("link")?.parse().ok()?;
    let valid = name.len() <= MAX_ADVERTISED_NAME && crate::is_valid_name(name);
    (valid && link != 0).then(|| NearbyAdvertisement {
        name: name.to_owned(),
        link,
    })
}
