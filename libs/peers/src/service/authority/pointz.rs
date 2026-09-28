use std::time::{SystemTime, UNIX_EPOCH};

use zeroize::Zeroizing;

use super::state::{sanitize_name, PointzDeviceRecord, PointzState, State};
use super::{AuthorityError, PeerAuthority};
use crate::pointz::{PointzDevice, PointzDeviceId, PointzImport, PointzProjection};
use crate::service::pointz::{LegacyPointz, Seed, MAX_DEVICES};
use crate::StoreRevision;

const DEVICE_FALLBACK_NAME: &str = "Phone";

impl PeerAuthority {
    pub fn pointz(&self) -> Result<Option<PointzProjection>, AuthorityError> {
        let inner = self.lock()?;
        Ok(inner.state.pointz.as_ref().map(|pointz| PointzProjection {
            server_id: pointz.seed.server_id(),
            import: pointz.import,
            devices: pointz
                .devices
                .iter()
                .map(|device| PointzDevice {
                    device_id: device.id,
                    name: device.name.clone(),
                    paired_at_ms: device.paired_at_ms,
                })
                .collect(),
        }))
    }

    pub fn import_pointz(&self, legacy: LegacyPointz) -> Result<StoreRevision, AuthorityError> {
        self.mutate_current(|state| {
            if state.pointz.is_some() {
                return Err(AuthorityError::PointzMigrated);
            }
            state.pointz = Some(PointzState {
                seed: legacy.seed,
                devices: legacy
                    .devices
                    .into_iter()
                    .map(|device| PointzDeviceRecord {
                        id: device.id,
                        key: device.key,
                        name: sanitize_name(&device.name, DEVICE_FALLBACK_NAME),
                        paired_at_ms: device.paired_at_ms,
                    })
                    .collect(),
                import: legacy.import,
            });
            Ok(())
        })
    }

    pub fn initialize_pointz(&self) -> Result<Option<StoreRevision>, AuthorityError> {
        if self.lock()?.state.pointz.is_some() {
            return Ok(None);
        }
        let seed = Seed::generate().ok_or(AuthorityError::Identity)?;
        match self.mutate_current(|state| {
            if state.pointz.is_some() {
                return Err(AuthorityError::PointzMigrated);
            }
            state.pointz = Some(PointzState {
                seed,
                devices: Vec::new(),
                import: PointzImport::FRESH,
            });
            Ok(())
        }) {
            Ok(revision) => Ok(Some(revision)),
            Err(AuthorityError::PointzMigrated) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn remove_pointz_device(
        &self,
        expected: StoreRevision,
        device: PointzDeviceId,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate(expected, |state| {
            let pointz = pointz_mut(state)?;
            let index = pointz
                .devices
                .iter()
                .position(|entry| entry.id == device)
                .ok_or(AuthorityError::UnknownDevice)?;
            pointz.devices.remove(index);
            Ok(())
        })
    }

    pub(crate) fn pointz_seed(&self) -> Result<Seed, AuthorityError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        inner
            .state
            .pointz
            .as_ref()
            .map(|pointz| pointz.seed.clone())
            .ok_or(AuthorityError::PointzAbsent)
    }

    pub(crate) fn pointz_key(&self, device: &PointzDeviceId) -> Option<Zeroizing<[u8; 32]>> {
        let inner = self.lock().ok()?;
        inner.ensure_ready().ok()?;
        inner
            .state
            .pointz
            .as_ref()?
            .devices
            .iter()
            .find(|entry| entry.id == *device)
            .map(|entry| entry.key.clone())
    }

    pub(crate) fn pair_pointz(
        &self,
        device: PointzDeviceId,
        key: Zeroizing<[u8; 32]>,
        name: &str,
    ) -> Result<StoreRevision, AuthorityError> {
        let name = sanitize_name(name, DEVICE_FALLBACK_NAME);
        self.mutate_current(|state| {
            let pointz = pointz_mut(state)?;
            pointz.devices.retain(|entry| entry.id != device);
            if pointz.devices.len() == MAX_DEVICES {
                return Err(AuthorityError::Capacity);
            }
            pointz.devices.push(PointzDeviceRecord {
                id: device,
                key,
                name,
                paired_at_ms: unix_time_ms(),
            });
            Ok(())
        })
    }
}

fn pointz_mut(state: &mut State) -> Result<&mut PointzState, AuthorityError> {
    state.pointz.as_mut().ok_or(AuthorityError::PointzAbsent)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}
