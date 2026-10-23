//! System paired-device inventory. Native queries run off the HID run loop.
use objc2::{extern_class, extern_conformance, extern_methods, msg_send, rc::Retained, sel};
use objc2_core_bluetooth::CBPeripheral;
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString};
use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};
use taprelay_core::{
    devices::Availability,
    state::{DeviceKind, Knowledge, Target, upsert_target},
};

#[link(name = "IOBluetooth", kind = "framework")]
unsafe extern "C" {}

extern_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct IOBluetoothDevice;
);
extern_conformance!(
    unsafe impl NSObjectProtocol for IOBluetoothDevice {}
);
impl IOBluetoothDevice {
    extern_methods!(
        #[unsafe(method(pairedDevices))]
        #[unsafe(method_family = none)]
        unsafe fn paired_devices() -> Option<Retained<NSArray<Self>>>;
        #[unsafe(method(name))]
        #[unsafe(method_family = none)]
        unsafe fn name(&self) -> Option<Retained<NSString>>;
        #[unsafe(method(addressString))]
        #[unsafe(method_family = none)]
        unsafe fn address_string(&self) -> Option<Retained<NSString>>;
        #[unsafe(method(isConnected))]
        #[unsafe(method_family = none)]
        unsafe fn is_connected(&self) -> bool;
        #[unsafe(method(isPaired))]
        #[unsafe(method_family = none)]
        unsafe fn is_paired(&self) -> bool;
        #[unsafe(method(classOfDevice))]
        #[unsafe(method_family = none)]
        unsafe fn class_of_device(&self) -> u32;
    );
}

/// IOBluetooth's public inventory has addresses; Core Bluetooth uses UUIDs.
/// macOS 27 also exposes the wrapped CBPeripheral through an undocumented
/// getter. This optional metadata bridge is isolated here: absent/wrong types
/// leave records separate. Never infer identity from a display name.
fn peripheral_id(device: &IOBluetoothDevice) -> Option<String> {
    if !device.respondsToSelector(sel!(peripheral)) {
        return None;
    }
    let method = device.class().instance_method(sel!(peripheral))?;
    if method.return_type().to_bytes() != b"@" {
        return None;
    }
    let object: Option<Retained<objc2::runtime::AnyObject>> =
        unsafe { msg_send![device, peripheral] };
    let peripheral = object.as_deref()?.downcast_ref::<CBPeripheral>()?;
    Some(unsafe { peripheral.identifier() }.UUIDString().to_string())
}

fn target(
    address: &str,
    name: &str,
    peripheral: Option<String>,
    connected: bool,
    class: u32,
) -> Option<Target> {
    let address: String = address
        .chars()
        .filter(|c| !matches!(c, ':' | '-'))
        .collect();
    if address.len() != 12 || !address.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let address = address.to_ascii_lowercase();
    let address_id = format!("macos-bt:{address}");
    let mut identity = vec![format!("bt-address:{address}")];
    let reconnectable = peripheral.is_some();
    let (id, aliases) = match peripheral {
        Some(id) => {
            identity.push(format!("cbcentral:{id}"));
            (id, vec![address_id])
        }
        None => (address_id, vec![]),
    };
    let major = (class >> 8) & 0x1f;
    let minor = (class >> 2) & 0x3f;
    Some(Target {
        id,
        name: if name.trim().is_empty() {
            address
        } else {
            name.into()
        },
        kind: match (major, minor) {
            (1, 5 | 7) => DeviceKind::Tablet,
            (1, _) => DeviceKind::Computer,
            (2, _) => DeviceKind::Phone,
            _ => DeviceKind::Unknown,
        },
        pairing: Knowledge::Yes,
        link: if connected {
            Knowledge::Yes
        } else {
            Knowledge::No
        },
        // A system Bluetooth connection does not prove a HID subscription.
        subscribed: Knowledge::Unknown,
        reconnectable,
        identity,
        aliases,
        availability: if connected {
            Availability::Nearby
        } else {
            Availability::Unknown
        },
        ..Default::default()
    })
}

fn enumerate() -> Vec<Target> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mut targets = vec![];
        if let Some(devices) = IOBluetoothDevice::paired_devices() {
            for device in devices {
                if !device.is_paired() {
                    continue;
                }
                let Some(address) = device.address_string() else {
                    continue;
                };
                let name = device
                    .name()
                    .map(|name| name.to_string())
                    .unwrap_or_default();
                if let Some(target) = target(
                    &address.to_string(),
                    &name,
                    peripheral_id(&device),
                    device.is_connected(),
                    device.class_of_device(),
                ) {
                    // Some macOS versions return a dual-mode device twice.
                    upsert_target(&mut targets, target);
                }
            }
        }
        targets.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        targets
    })
}

pub(super) struct Discovery {
    requests: SyncSender<()>,
    updates: Receiver<Result<Vec<Target>, String>>,
    next: Instant,
    pending: Option<Instant>,
    timed_out: bool,
}
impl Discovery {
    pub fn start() -> std::io::Result<Self> {
        let (requests, rx) = mpsc::sync_channel(1);
        let (tx, updates) = mpsc::channel();
        std::thread::Builder::new()
            .name("bluetooth-inventory".into())
            .spawn(move || {
                let mut last_unresolved = None;
                while rx.recv().is_ok() {
                    let started = Instant::now();
                    let result = std::panic::catch_unwind(enumerate)
                        .map_err(|_| "Bluetooth inventory query panicked".into());
                    if let Ok(targets) = &result {
                        let unresolved = targets.iter().filter(|target| !target.reconnectable).count();
                        if unresolved > 0 && last_unresolved != Some(unresolved) {
                            tracing::info!(unresolved, "Paired devices without a Core Bluetooth identity require a live HID subscription");
                        }
                        last_unresolved = Some(unresolved);
                    }
                    tracing::debug!(
                        devices = result.as_ref().map_or(0, Vec::len),
                        elapsed_ms = started.elapsed().as_millis(),
                        "System paired-device inventory refreshed"
                    );
                    if tx.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            updates,
            next: Instant::now(),
            pending: None,
            timed_out: false,
        })
    }

    pub fn refresh(&mut self) {
        self.next = Instant::now();
    }

    pub fn poll(&mut self, available: bool) -> Option<Result<Vec<Target>, String>> {
        let now = Instant::now();
        match self.updates.try_recv() {
            Ok(result) => {
                self.pending = None;
                self.timed_out = false;
                self.next = now + Duration::from_secs(5);
                return Some(result);
            }
            Err(TryRecvError::Disconnected) => {
                if !self.timed_out {
                    self.timed_out = true;
                    return Some(Err("Bluetooth inventory thread exited".into()));
                }
            }
            Err(TryRecvError::Empty) => {}
        }
        if let Some(since) = self.pending {
            if !self.timed_out && now.duration_since(since) >= Duration::from_secs(5) {
                self.timed_out = true;
                return Some(Err("Bluetooth inventory query timed out".into()));
            }
        } else if available && now >= self.next && self.requests.try_send(()).is_ok() {
            self.pending = Some(now);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_inventory_queries_time_out_once_and_recover_without_piling_up_requests() {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (results, updates) = mpsc::channel();
        let mut discovery = Discovery {
            requests,
            updates,
            next: Instant::now(),
            pending: Some(Instant::now() - Duration::from_secs(6)),
            timed_out: false,
        };
        assert!(discovery.poll(true).unwrap().is_err());
        assert!(discovery.poll(true).is_none());
        assert!(matches!(incoming.try_recv(), Err(TryRecvError::Empty)));
        results.send(Ok(vec![])).unwrap();
        assert!(discovery.poll(true).unwrap().is_ok());
        assert!(discovery.pending.is_none());
        assert!(!discovery.timed_out);
        discovery.refresh();
        assert!(discovery.poll(false).is_none());
        assert!(matches!(incoming.try_recv(), Err(TryRecvError::Empty)));
        assert!(discovery.poll(true).is_none());
        assert!(incoming.try_recv().is_ok());
        assert!(discovery.poll(true).is_none());
        assert!(matches!(incoming.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn duplicate_native_records_join_by_address_and_retain_the_uuid_bridge() {
        let classic = target("80-A9-97-8F-C4-49", "Tablet", None, true, 0x114).unwrap();
        assert!(!classic.reconnectable);
        let le = target(
            "80:a9:97:8f:c4:49",
            "Tablet",
            Some("uuid".into()),
            true,
            0x114,
        )
        .unwrap();
        assert!(le.reconnectable);
        let mut rows = vec![];
        upsert_target(&mut rows, classic);
        upsert_target(&mut rows, le);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].reconnectable);
        assert!(rows[0].identity.contains(&"cbcentral:uuid".into()));
        assert_eq!(rows[0].kind, DeviceKind::Tablet);
        assert_eq!(rows[0].subscribed, Knowledge::Unknown);
        assert!(target("invalid", "Tablet", None, true, 0).is_none());
    }

    #[test]
    fn same_names_and_missing_bridges_never_merge_different_devices() {
        let mut rows = vec![];
        for address in ["001122334455", "001122334466"] {
            upsert_target(
                &mut rows,
                target(address, "Same name", None, false, 0).unwrap(),
            );
        }
        upsert_target(
            &mut rows,
            Target {
                id: "live-uuid".into(),
                name: "Same name".into(),
                identity: vec!["cbcentral:live-uuid".into()],
                subscribed: Knowledge::Yes,
                ..Default::default()
            },
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].availability, Availability::Unknown);
    }
}
