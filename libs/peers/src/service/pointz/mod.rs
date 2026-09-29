mod envelope;
mod legacy;
mod pairing;
mod replay;
mod seed;
mod transport;

#[cfg(test)]
mod tests;

pub use legacy::LegacyPointz;
pub(crate) use seed::Seed;
pub use transport::{PointzAdapter, PointzOptions, PointzSink, COMMAND_PORT, DISCOVERY_PORT};

pub(crate) const MAX_DEVICES: usize = 64;
