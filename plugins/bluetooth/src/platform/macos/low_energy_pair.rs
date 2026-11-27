use std::cell::RefCell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicProperties,
    CBManager, CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService,
};
use objc2_foundation::{NSArray, NSError, NSString, NSUUID};

use super::{access_denied, deliver_callbacks_until, low_energy_denied, unready_reason};

const READY_TIMEOUT: Duration = Duration::from_secs(5);
const PAIR_TIMEOUT: Duration = Duration::from_secs(45);
const SETTLE: Duration = Duration::from_secs(3);
const ATT_ERROR_DOMAIN: &str = "CBATTErrorDomain";
const NEEDS_PAIRING: [isize; 4] = [0x05, 0x08, 0x0C, 0x0F];

#[derive(Default)]
struct Progress {
    connected: bool,
    dropped: bool,
    refused: bool,
}

impl Progress {
    fn ended(&self) -> bool {
        self.dropped || self.refused
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Progress>]
    struct LowEnergyPairer;

    unsafe impl NSObjectProtocol for LowEnergyPairer {}

    unsafe impl CBCentralManagerDelegate for LowEnergyPairer {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn central_state_changed(&self, _central: &CBCentralManager) {}

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn peripheral_connected(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            self.ivars().borrow_mut().connected = true;
            unsafe { peripheral.discoverServices(None) };
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn peripheral_refused(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            _error: Option<&NSError>,
        ) {
            self.ivars().borrow_mut().dropped = true;
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn peripheral_dropped(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            _error: Option<&NSError>,
        ) {
            self.ivars().borrow_mut().dropped = true;
        }
    }

    unsafe impl CBPeripheralDelegate for LowEnergyPairer {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn services_found(&self, peripheral: &CBPeripheral, _error: Option<&NSError>) {
            let Some(services) = (unsafe { peripheral.services() }) else {
                return;
            };
            for service in services.iter() {
                unsafe { peripheral.discoverCharacteristics_forService(None, &service) };
            }
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn characteristics_found(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            _error: Option<&NSError>,
        ) {
            let Some(characteristics) = (unsafe { service.characteristics() }) else {
                return;
            };
            for characteristic in characteristics.iter() {
                touch(peripheral, &characteristic);
            }
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn value_read(
            &self,
            _peripheral: &CBPeripheral,
            _characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            self.note(error);
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn notification_set(
            &self,
            _peripheral: &CBPeripheral,
            _characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            self.note(error);
        }
    }
);

impl LowEnergyPairer {
    fn new(marker: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(RefCell::new(Progress::default()));
        unsafe { msg_send![super(this), init] }
    }

    fn note(&self, error: Option<&NSError>) {
        if error.is_some_and(needs_pairing) {
            self.ivars().borrow_mut().refused = true;
        }
    }
}

fn touch(peripheral: &CBPeripheral, characteristic: &CBCharacteristic) {
    let properties = unsafe { characteristic.properties() };
    if properties.contains(CBCharacteristicProperties::Read) {
        unsafe { peripheral.readValueForCharacteristic(characteristic) };
    }
    if properties
        .intersects(CBCharacteristicProperties::Notify | CBCharacteristicProperties::Indicate)
    {
        unsafe { peripheral.setNotifyValue_forCharacteristic(true, characteristic) };
    }
}

fn needs_pairing(error: &NSError) -> bool {
    error.domain().to_string() == ATT_ERROR_DOMAIN && NEEDS_PAIRING.contains(&error.code())
}

pub(super) fn pair(identifier: &str, bonded: impl Fn() -> HashSet<String>) -> Result<String> {
    let marker = MainThreadMarker::new()
        .ok_or_else(|| anyhow!("pairing a Bluetooth LE device needs the main thread"))?;
    if low_energy_denied(unsafe { CBManager::authorization_class() }) {
        bail!("{}", access_denied());
    }
    let pairer = LowEnergyPairer::new(marker);
    let central = unsafe { CBCentralManager::new() };
    unsafe { central.setDelegate(Some(ProtocolObject::from_ref(&*pairer))) };
    let ready = deliver_callbacks_until(READY_TIMEOUT, || unsafe {
        central.state() == CBManagerState::PoweredOn
    });
    if !ready {
        unsafe { central.setDelegate(None) };
        bail!("{}", unready_reason(unsafe { central.state() }));
    }
    let result = pair_with(&central, &pairer, identifier, bonded);
    unsafe { central.setDelegate(None) };
    result
}

fn pair_with(
    central: &CBCentralManager,
    pairer: &LowEnergyPairer,
    identifier: &str,
    bonded: impl Fn() -> HashSet<String>,
) -> Result<String> {
    let uuid = NSUUID::from_string(&NSString::from_str(identifier))
        .ok_or_else(|| anyhow!("{identifier} is not a Bluetooth LE identifier"))?;
    let known = unsafe {
        central.retrievePeripheralsWithIdentifiers(&NSArray::from_retained_slice(&[uuid]))
    };
    let peripheral = known
        .iter()
        .next()
        .ok_or_else(|| anyhow!("this Mac no longer knows {identifier}; search again"))?;
    let before = bonded();
    unsafe { peripheral.setDelegate(Some(ProtocolObject::from_ref(pairer))) };
    unsafe { central.connectPeripheral_options(&peripheral, None) };
    let mut new_bond = None;
    let mut ended_at = None;
    deliver_callbacks_until(PAIR_TIMEOUT, || {
        new_bond = bonded().difference(&before).next().cloned();
        if new_bond.is_some() {
            return true;
        }
        if pairer.ivars().borrow().ended() {
            let ended = *ended_at.get_or_insert_with(Instant::now);
            return ended.elapsed() >= SETTLE;
        }
        false
    });
    unsafe { central.cancelPeripheralConnection(&peripheral) };
    unsafe { peripheral.setDelegate(None) };
    if let Some(address) = new_bond {
        return Ok(address);
    }
    let progress = pairer.ivars().borrow();
    if !progress.connected {
        bail!("the device did not accept a connection; put it in pairing mode and try again");
    }
    if progress.refused {
        bail!("macOS did not pair: click Connect on its prompt within 30 seconds");
    }
    bail!("pairing did not complete; put the device in pairing mode and try again")
}
