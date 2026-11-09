mod envelope;
mod legacy;
mod pairing;
mod replay;
mod seed;
mod spake2;
mod transport;

#[cfg(test)]
mod phone;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vector;

use zeroize::Zeroizing;

pub use legacy::LegacyPointz;
pub(crate) use seed::Seed;
pub use transport::{PointzAdapter, PointzOptions, PointzSink, COMMAND_PORT, DISCOVERY_PORT};

pub(crate) const MAX_DEVICES: usize = 64;

pub(crate) struct DeviceKey {
    pub(crate) key: Zeroizing<[u8; 32]>,
    pub(crate) sealed_only: bool,
}
