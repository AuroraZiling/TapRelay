//! Core Bluetooth HID peripheral, owned exclusively by the isolated worker's
//! main thread. Callbacks never send through IPC or block on the receiver.
use core_foundation::runloop::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained, runtime::ProtocolObject,
};
use objc2_core_bluetooth::*;
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use taprelay_core::{
    command::{MediaCommand, QueuedCommand},
    devices::{AdapterState, Availability, Connection, DiscoveryState},
    hid::{self, ConsumerState, InputReports, Profile, ReportKind},
    mapping::OutputOwners,
    passthrough::{Event, InputLink, MAX_INPUT_AGE, Packet, ReportSchedule},
    ports::BackendError,
    state::{Knowledge, Snapshot, Target, upsert_target},
};
use tokio::sync::{oneshot, watch};

mod discovery;
mod names;

type Reply = oneshot::Sender<Result<(), BackendError>>;
pub enum Request {
    Select(String),
    Send(QueuedCommand, Reply),
    Refresh,
    Pair(String),
}

struct Notification {
    kind: ReportKind,
    bytes: Vec<u8>,
    generation: u64,
    created: Instant,
    reply: Option<Reply>,
}
struct Peer {
    central: Retained<CBCentral>,
    subscribed: [bool; 3],
}
struct Data {
    state: Snapshot,
    updates: watch::Sender<Snapshot>,
    remembered: Option<Target>,
    revision: Arc<AtomicU64>,
    link: InputLink,
    input: mpsc::Receiver<Packet>,
    reports: Vec<(ReportKind, Retained<CBMutableCharacteristic>)>,
    peers: BTreeMap<String, Peer>,
    known: Vec<Target>,
    discovery: Option<discovery::Discovery>,
    names: names::ReceiverNames,
    queue: VecDeque<Notification>,
    blocked: bool,
    synchronizing: bool,
    publish: bool,
    consumer: ConsumerState,
    keyboard: InputReports,
    owners: OutputOwners,
    schedule: ReportSchedule,
    pulses: Vec<(Instant, u64, MediaCommand)>,
    next_owner: u64,
    epoch: u64,
    mapping_epoch: u64,
    generation_started: Instant,
    accepted_reports: [u64; 3],
}

// Core Bluetooth's delegate is weak. BleHandle retains both objects for the
// entire run-loop lifetime; all accesses are on this process's main thread.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Data>]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn metadata_changed(&self, manager: &CBCentralManager) {
            tracing::info!(state = ?unsafe { manager.state() }, "Bluetooth name resolver state changed");
        }
    }
    unsafe impl CBPeripheralManagerDelegate for Delegate {
        #[unsafe(method(peripheralManagerDidUpdateState:))]
        fn changed(&self, manager: &CBPeripheralManager) { self.call(manager, |data, manager| data.changed(manager)); }
        #[unsafe(method(peripheralManager:didAddService:error:))]
        fn added(&self, manager: &CBPeripheralManager, _service: &CBService, error: Option<&NSError>) {
            self.call(manager, |data, manager| {
                if let Some(error) = error { data.fail(format!("Core Bluetooth addService: {error}")); return; }
                data.state.service = true;
                unsafe {
                    let keys = [CBAdvertisementDataLocalNameKey, CBAdvertisementDataServiceUUIDsKey];
                    let name = NSString::from_str("TapRelay");
                    let services = NSArray::from_retained_slice(&[service_uuid(0x1812)]);
                    let values: [&objc2::runtime::AnyObject; 2] = [&name, &services];
                    let advertisement = NSDictionary::from_slices(&keys, &values);
                    manager.startAdvertising(Some(&advertisement));
                }
            });
        }
        #[unsafe(method(peripheralManagerDidStartAdvertising:error:))]
        fn advertised(&self, manager: &CBPeripheralManager, error: Option<&NSError>) {
            self.call(manager, |data, manager| {
                if let Some(error) = error { data.fail(format!("Core Bluetooth advertising: {error}")); }
                else { data.state.broadcasting = unsafe { manager.isAdvertising() }; }
            });
        }
        #[unsafe(method(peripheralManager:central:didSubscribeToCharacteristic:))]
        fn subscribed(&self, manager: &CBPeripheralManager, central: &CBCentral, characteristic: &CBCharacteristic) {
            self.call(manager, |data, manager| data.subscription(manager, central, characteristic, true));
        }
        #[unsafe(method(peripheralManager:central:didUnsubscribeFromCharacteristic:))]
        fn unsubscribed(&self, manager: &CBPeripheralManager, central: &CBCentral, characteristic: &CBCharacteristic) {
            self.call(manager, |data, manager| data.subscription(manager, central, characteristic, false));
        }
        #[unsafe(method(peripheralManagerIsReadyToUpdateSubscribers:))]
        fn writable(&self, manager: &CBPeripheralManager) {
            self.call(manager, |data, _| data.blocked = false);
        }
        #[unsafe(method(peripheralManager:didReceiveReadRequest:))]
        fn read(&self, manager: &CBPeripheralManager, request: &CBATTRequest) {
            self.call(manager, |data, manager| unsafe {
                let characteristic = request.characteristic();
                let kind = data.kind(&characteristic);
                // Reads never disclose a selected receiver's held report to another central.
                let bytes = kind.map(hid::neutral).or_else(|| {
                    let id = characteristic.UUID();
                    if id == uuid(0x2a4a) { Some(hid::HID_INFORMATION.to_vec()) }
                    else if id == uuid(0x2a4b) { Some(Profile::Full.report_map().to_vec()) }
                    else if id == uuid(0x2a4e) { Some(hid::PROTOCOL_MODE.to_vec()) }
                    else { None }
                });
                if let Some(bytes) = bytes {
                    if characteristic.UUID() == uuid(0x2a4b) {
                        tracing::info!(offset = request.offset(), length = bytes.len(), "HID report map requested");
                    }
                    if request.offset() <= bytes.len() {
                        request.setValue(Some(&NSData::with_bytes(&bytes[request.offset()..])));
                        manager.respondToRequest_withResult(request, CBATTError::Success);
                    } else { manager.respondToRequest_withResult(request, CBATTError::InvalidOffset); }
                } else { manager.respondToRequest_withResult(request, CBATTError::ReadNotPermitted); }
            });
        }
        #[unsafe(method(peripheralManager:didReceiveWriteRequests:))]
        fn written(&self, manager: &CBPeripheralManager, requests: &NSArray<CBATTRequest>) {
            self.call(manager, |data, manager| unsafe {
                let mut result = CBATTError::Success;
                for request in requests {
                    let ch = request.characteristic();
                    let value = request.value().map(|v| v.to_vec()).unwrap_or_default();
                    if request.offset() != 0 || value.len() != 1 { result = CBATTError::InvalidAttributeValueLength; break; }
                    if ch.UUID() == uuid(0x2a4c) {
                        if data.selected_peer().is_some_and(|peer| central_id(&peer.central) == central_id(&request.central())) {
                            data.state.hid_suspended = value[0] == 0;
                            data.revoke();
                            if !data.state.hid_suspended { data.synchronize(); }
                        }
                    } else if ch.UUID() != uuid(0x2a4e) || value[0] != 1 { result = CBATTError::RequestNotSupported; break; }
                }
                if let Some(first) = requests.firstObject() { manager.respondToRequest_withResult(&first, result); }
            });
        }
    }
);
impl Delegate {
    fn call(&self, manager: &CBPeripheralManager, f: impl FnOnce(&mut Data, &CBPeripheralManager)) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut data = self.ivars().borrow_mut();
            f(&mut data, manager);
            data.emit();
        }))
        .is_err()
            && let Ok(mut data) = self.ivars().try_borrow_mut()
        {
            data.fail("Core Bluetooth callback panicked".into());
            data.emit();
        }
    }
}

pub struct BleHandle {
    delegate: Retained<Delegate>,
    manager: Retained<CBPeripheralManager>,
    metadata_manager: Retained<CBCentralManager>,
    pub state: watch::Receiver<Snapshot>,
    progress: std::cell::Cell<u64>,
}
impl BleHandle {
    pub fn start(
        remembered: Option<Target>,
        profile: Profile,
        publish: bool,
    ) -> Result<Self, BackendError> {
        objc2::rc::autoreleasepool(|_| {
            let mtm = MainThreadMarker::new()
                .ok_or_else(|| unavailable("BLE worker must run on its main thread"))?;
            let mut data = Data::new(remembered, profile, publish);
            match discovery::Discovery::start() {
                Ok(discovery) => {
                    data.discovery = Some(discovery);
                    data.state.discovery = DiscoveryState::Scanning;
                }
                Err(error) => {
                    tracing::warn!(%error, "Could not start paired-device inventory");
                    data.state.discovery = DiscoveryState::Failed;
                }
            }
            let state = data.updates.subscribe();
            let allocated = Delegate::alloc(mtm).set_ivars(RefCell::new(data));
            let delegate: Retained<Delegate> = unsafe { msg_send![super(allocated), init] };
            let manager = unsafe {
                CBPeripheralManager::initWithDelegate_queue(
                    CBPeripheralManager::alloc(),
                    Some(ProtocolObject::from_ref(&*delegate)),
                    None,
                )
            };
            // Metadata only: never scan, connect, or discover GATT services.
            // Both managers use the worker's existing main run loop.
            let metadata_manager = unsafe {
                CBCentralManager::initWithDelegate_queue(
                    CBCentralManager::alloc(),
                    Some(ProtocolObject::from_ref(&*delegate)),
                    None,
                )
            };
            Ok(Self {
                delegate,
                manager,
                metadata_manager,
                state,
                progress: std::cell::Cell::new(0),
            })
        })
    }
    /// The worker owns this run loop; the GPUI app never starts another one.
    pub fn poll(&self) {
        objc2::rc::autoreleasepool(|_| {
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, 1);
            }
            self.delegate
                .call(&self.manager, |data, manager| data.tick(manager));
            // Resolve outside subscription callbacks, after servicing input.
            let mut data = self.delegate.ivars().borrow_mut();
            data.poll_inventory();
            if data.names.resolve(&self.metadata_manager) {
                data.update_names();
                data.emit();
            }
            self.progress.set(self.progress.get().wrapping_add(1));
        });
    }
    pub fn progress(&self) -> u64 {
        self.progress.get()
    }
    pub fn input_link(&self) -> InputLink {
        self.delegate.ivars().borrow().link.clone()
    }
    pub fn set_profile(&self, profile: Profile) {
        let mut data = self.delegate.ivars().borrow_mut();
        data.revoke();
        data.state.hid_profile = profile;
        data.synchronize();
        data.emit();
    }
    pub fn invalidate_commands(&self) -> u64 {
        let mut data = self.delegate.ivars().borrow_mut();
        data.revoke();
        data.synchronize();
        data.state.generation
    }
    pub fn request(&self, request: Request) -> Result<(), BackendError> {
        let mut data = self.delegate.ivars().borrow_mut();
        match request {
            Request::Select(id) => {
                data.select(id);
            }
            Request::Send(command, reply) => {
                if !command.valid(
                    data.state.selected.as_deref(),
                    data.state.generation,
                    data.state.ready,
                    Instant::now(),
                    data.generation_started,
                ) {
                    let _ = reply.send(Err(BackendError::Stale));
                } else {
                    let down = command.is_press();
                    data.media(command.action, down)?;
                    if let Some(last) = data.queue.back_mut() {
                        last.reply = Some(reply);
                    } else {
                        let _ = reply.send(Err(BackendError::Stale));
                    }
                }
            }
            Request::Refresh => {
                if let Some(discovery) = &mut data.discovery {
                    discovery.refresh();
                    data.state.discovery = DiscoveryState::Scanning;
                }
                let ids = data.peers.keys().cloned().collect::<Vec<_>>();
                for id in ids {
                    data.names.refresh(&id);
                }
            }
            Request::Pair(_) => {
                return Err(unavailable(
                    "Pair and disconnect in the receiver's system Bluetooth settings",
                ));
            }
        }
        data.emit();
        Ok(())
    }
}
impl Drop for BleHandle {
    fn drop(&mut self) {
        objc2::rc::autoreleasepool(|_| {
            {
                let mut data = self.delegate.ivars().borrow_mut();
                data.revoke();
                // Best effort neutral reports to the previously selected central only.
                data.synchronize();
            }
            // Keep the native loop alive for paced releases. This runs only during
            // worker shutdown, never inside a Core Bluetooth callback or event tap.
            let until = Instant::now() + MAX_INPUT_AGE;
            while !self.delegate.ivars().borrow().queue.is_empty() && Instant::now() < until {
                self.poll();
                std::thread::sleep(Duration::from_millis(1));
            }
            let reports = self.delegate.ivars().borrow().accepted_reports;
            tracing::info!(
                consumer = reports[0],
                keyboard = reports[1],
                mouse = reports[2],
                "HID reports accepted during worker session"
            );
            unsafe {
                self.manager.stopAdvertising();
                self.manager.removeAllServices();
                self.manager.setDelegate(None);
                self.metadata_manager.setDelegate(None);
            }
        });
    }
}
fn unavailable(message: impl Into<String>) -> BackendError {
    BackendError::Unavailable(message.into())
}
fn uuid(value: u16) -> Retained<CBUUID> {
    // Construct standard characteristics/descriptors in their assigned 16-bit form.
    unsafe { CBUUID::UUIDWithString(&NSString::from_str(&format!("{value:04X}"))) }
}
fn service_uuid(value: u16) -> Retained<CBUUID> {
    // macOS filters short HID service UUIDs in peripheral publication. Keep
    // this workaround confined to the service, not its standard attributes.
    unsafe {
        CBUUID::UUIDWithString(&NSString::from_str(&format!(
            "0000{value:04X}-0000-1000-8000-00805F9B34FB"
        )))
    }
}
fn central_id(central: &CBCentral) -> String {
    unsafe { central.identifier().UUIDString().to_string() }
}
fn merge_targets(
    known: &[Target],
    live: impl IntoIterator<Item = Target>,
    state: &Snapshot,
) -> Vec<Target> {
    let mut targets = known.to_vec();
    for target in live {
        upsert_target(&mut targets, target);
    }
    for target in &mut targets {
        let selected = state
            .selected
            .as_ref()
            .is_some_and(|id| target.matches_id(id));
        if state.adapter_state != AdapterState::Available {
            target.link = if state.adapter_state == AdapterState::Disabled {
                Knowledge::No
            } else {
                Knowledge::Unknown
            };
            target.availability = Availability::Unknown;
        }
        target.connection = if !selected || state.adapter_state != AdapterState::Available {
            Connection::Disconnected
        } else if state.ready {
            Connection::Connected
        } else if target.subscribed == Knowledge::Yes {
            Connection::Synchronizing
        } else {
            Connection::AwaitingHostSubscription
        };
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    targets
}
impl Data {
    fn select(&mut self, id: String) {
        // Init may select before the first native poll. Keep the intent; only
        // an actual subscription can synchronize and authorize report delivery.
        self.remembered = None;
        self.state.selected = Some(id.clone());
        self.state.hid_suspended = false;
        self.rebuild_targets();
        self.reconcile_selection(Some(&id));
    }

    fn new(remembered: Option<Target>, profile: Profile, publish: bool) -> Self {
        let revision = Arc::new(AtomicU64::new(1));
        let (link, input) = InputLink::channel(revision.clone());
        let snapshot = Snapshot {
            hid_profile: profile,
            service_paused: !publish,
            generation: 1,
            ..Default::default()
        };
        let (updates, _) = watch::channel(snapshot.clone());
        Self {
            state: snapshot,
            updates,
            names: names::ReceiverNames::new(remembered.as_ref()),
            remembered,
            revision,
            link,
            input,
            reports: vec![],
            peers: BTreeMap::new(),
            known: vec![],
            discovery: None,
            queue: VecDeque::new(),
            blocked: false,
            synchronizing: false,
            publish,
            consumer: Default::default(),
            keyboard: Default::default(),
            owners: Default::default(),
            schedule: Default::default(),
            pulses: vec![],
            next_owner: 0,
            epoch: 0,
            mapping_epoch: 0,
            generation_started: Instant::now(),
            accepted_reports: [0; 3],
        }
    }

    fn emit(&mut self) {
        self.updates.send_if_modified(|state| {
            if *state == self.state {
                false
            } else {
                *state = self.state.clone();
                true
            }
        });
    }
    fn revoke(&mut self) {
        self.state.generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        self.generation_started = Instant::now();
        self.state.ready = false;
        self.link.disconnect();
        self.link.set_mapping_profile(false);
        self.schedule.clear_input();
        self.consumer.clear();
        self.keyboard = Default::default();
        self.owners = Default::default();
        self.pulses.clear();
        for notification in self.queue.drain(..) {
            if let Some(reply) = notification.reply {
                let _ = reply.send(Err(BackendError::Stale));
            }
        }
        self.synchronizing = false;
    }
    fn fail(&mut self, error: String) {
        tracing::error!(%error, "BLE HID unavailable");
        self.revoke();
        self.state.last_error = Some(error.clone());
        self.link.fail(error);
    }
    fn changed(&mut self, manager: &CBPeripheralManager) {
        self.revoke();
        self.peers.clear();
        self.reports.clear();
        self.state.targets.clear();
        self.state.service = false;
        self.state.broadcasting = false;
        let state = unsafe { manager.state() };
        self.state.adapter_state = match state {
            CBManagerState::PoweredOn => AdapterState::Available,
            CBManagerState::PoweredOff => AdapterState::Disabled,
            CBManagerState::Unknown | CBManagerState::Resetting => AdapterState::Unknown,
            _ => AdapterState::Unavailable,
        };
        self.state.adapter = state == CBManagerState::PoweredOn;
        self.state.peripheral = self.state.adapter;
        if let Some(discovery) = &mut self.discovery {
            discovery.refresh();
            self.state.discovery = if self.state.adapter {
                DiscoveryState::Scanning
            } else {
                DiscoveryState::Idle
            };
        }
        self.rebuild_targets();
        if state == CBManagerState::Unauthorized {
            self.state.last_error = Some("Bluetooth permission denied. Enable TapRelay in System Settings → Privacy & Security → Bluetooth.".into());
        } else if state == CBManagerState::PoweredOn && self.publish {
            self.state.last_error = None;
            unsafe {
                manager.removeAllServices();
                manager.addService(&self.service());
            }
        }
    }
    fn service(&mut self) -> Retained<CBMutableService> {
        unsafe {
            let service = CBMutableService::initWithType_primary(
                CBMutableService::alloc(),
                &service_uuid(0x1812),
                true,
            );
            let read = CBCharacteristicProperties::Read;
            let readable = CBAttributePermissions::ReadEncryptionRequired;
            let mut chars = vec![
                characteristic(0x2a4a, read, Some(&hid::HID_INFORMATION), readable),
                characteristic(0x2a4b, read, Some(Profile::Full.report_map()), readable),
                characteristic(
                    0x2a4c,
                    CBCharacteristicProperties::WriteWithoutResponse,
                    None,
                    CBAttributePermissions::WriteEncryptionRequired,
                ),
                characteristic(
                    0x2a4e,
                    read | CBCharacteristicProperties::WriteWithoutResponse,
                    None,
                    readable | CBAttributePermissions::WriteEncryptionRequired,
                ),
            ];
            // Keep the on-air schema stable even in media-only routing mode.
            // iPad caches the report map/classification from initial pairing;
            // changing a consumer device into a keyboard later leaves the
            // software keyboard active despite keyboard notifications arriving.
            for kind in Profile::Full.reports() {
                let ch = characteristic(
                    0x2a4d,
                    read | CBCharacteristicProperties::Notify
                        | CBCharacteristicProperties::NotifyEncryptionRequired,
                    None,
                    readable,
                );
                let value = NSData::with_bytes(&kind.reference());
                let descriptor = CBMutableDescriptor::initWithType_value(
                    CBMutableDescriptor::alloc(),
                    &uuid(0x2908),
                    Some(&value),
                );
                ch.setDescriptors(Some(&NSArray::from_retained_slice(&[
                    descriptor.into_super()
                ])));
                self.reports.push((*kind, ch.clone()));
                chars.push(ch);
            }
            service.setCharacteristics(Some(&NSArray::from_retained_slice(
                &chars
                    .into_iter()
                    .map(|ch| ch.into_super())
                    .collect::<Vec<_>>(),
            )));
            service
        }
    }
    fn kind(&self, ch: &CBCharacteristic) -> Option<ReportKind> {
        self.reports
            .iter()
            .find(|(_, c)| std::ptr::eq::<CBCharacteristic>(&***c, ch))
            .map(|(k, _)| *k)
    }
    fn subscription(
        &mut self,
        manager: &CBPeripheralManager,
        central: &CBCentral,
        ch: &CBCharacteristic,
        subscribed: bool,
    ) {
        let Some(kind) = self.kind(ch) else {
            return;
        };
        let id = central_id(central);
        tracing::info!(central = %id, ?kind, subscribed, "Core Bluetooth report subscription changed");
        let peer = self.peers.entry(id.clone()).or_insert_with(|| Peer {
            central: central.retain(),
            subscribed: [false; 3],
        });
        let was_subscribed = peer.subscribed.iter().any(|value| *value);
        peer.subscribed[kind.index()] = subscribed;
        if subscribed && !was_subscribed {
            self.names.refresh(&id);
        }
        unsafe {
            manager.setDesiredConnectionLatency_forCentral(
                CBPeripheralManagerConnectionLatency::Low,
                central,
            );
        }
        self.rebuild_targets();
        if self.discovery.is_none() {
            self.state.discovery = DiscoveryState::ResultsAvailable;
        }
        self.reconcile_selection(Some(&id));
    }

    fn reconcile_selection(&mut self, changed_subscription: Option<&str>) {
        let resolved = self.state.targets.iter().find(|target| {
            target.subscribed == Knowledge::Yes
                && if let Some(id) = &self.state.selected {
                    target.matches_id(id)
                } else {
                    self.remembered
                        .as_ref()
                        .is_some_and(|wanted| target.same_device(wanted))
                }
        });
        let resolved = resolved.map(|target| target.id.clone());
        let restored = resolved.is_some() && resolved != self.state.selected;
        if restored {
            // Pin the live central UUID so a later metadata miss cannot erase
            // the active route. Display names never participate in selection.
            self.state.selected = resolved;
            tracing::info!("Receiver identity resolved to a subscribed session");
        }
        if restored
            || changed_subscription.is_some_and(|id| self.state.selected.as_deref() == Some(id))
        {
            self.blocked = false;
            self.revoke();
            self.rebuild_targets();
            self.synchronize();
        }
    }
    fn rebuild_targets(&mut self) {
        let live = self
            .peers
            .iter()
            .filter(|(id, peer)| {
                peer.subscribed.iter().any(|value| *value)
                    || self.state.selected.as_deref() == Some(id.as_str())
                    || self
                        .state
                        .selected_target()
                        .is_some_and(|target| target.matches_id(id))
            })
            .map(|(id, peer)| {
                let any = peer.subscribed.iter().any(|v| *v);
                Target {
                    id: id.clone(),
                    name: self.names.name(id),
                    identity: vec![format!("cbcentral:{id}")],
                    pairing: Knowledge::Unknown,
                    link: if any {
                        Knowledge::Yes
                    } else {
                        Knowledge::Unknown
                    },
                    subscribed: if any { Knowledge::Yes } else { Knowledge::No },
                    reconnectable: true,
                    availability: if any {
                        Availability::Nearby
                    } else {
                        Availability::Unknown
                    },
                    ..Default::default()
                }
            });
        self.state.targets = merge_targets(&self.known, live, &self.state);
        self.update_names();
    }
    fn update_names(&mut self) {
        for target in &mut self.state.targets {
            if let Some(known) = self.known.iter().find(|known| known.same_device(target)) {
                target.name.clone_from(&known.name);
            } else {
                target.name = self.names.name(&target.id);
            }
        }
    }
    fn poll_inventory(&mut self) {
        let available = self.state.adapter_state == AdapterState::Available;
        let Some(result) = self
            .discovery
            .as_mut()
            .and_then(|discovery| discovery.poll(available))
        else {
            return;
        };
        match result {
            Ok(known) => {
                self.known = known;
                self.rebuild_targets();
                // Inventory and subscriptions can arrive in either order.
                self.reconcile_selection(None);
                self.state.discovery = if available {
                    DiscoveryState::ResultsAvailable
                } else {
                    DiscoveryState::Idle
                };
            }
            Err(error) => {
                tracing::warn!(%error, "System paired-device inventory unavailable");
                self.state.discovery = DiscoveryState::Failed;
            }
        }
        self.emit();
    }
    fn selected_peer(&self) -> Option<&Peer> {
        self.peers.get(self.state.selected.as_ref()?)
    }
    fn subscribed(&self) -> bool {
        self.selected_peer().is_some_and(|peer| {
            self.state
                .hid_profile
                .reports()
                .iter()
                .all(|kind| peer.subscribed[kind.index()])
        }) && !self.state.hid_suspended
    }
    fn synchronize(&mut self) {
        if !self.subscribed() {
            return;
        }
        self.synchronizing = true;
        for kind in Profile::Full.reports() {
            if self
                .selected_peer()
                .is_some_and(|peer| peer.subscribed[kind.index()])
            {
                self.enqueue(*kind, hid::neutral(*kind));
            }
        }
    }
    fn enqueue(&mut self, kind: ReportKind, bytes: Vec<u8>) {
        if self.queue.len() >= 1024 {
            self.fail("BLE HID queue overflow".into());
            return;
        }
        self.queue.push_back(Notification {
            kind,
            bytes,
            generation: self.state.generation,
            created: Instant::now(),
            reply: None,
        });
    }
    fn flush(&mut self, manager: &CBPeripheralManager) {
        if self.blocked || !self.subscribed() {
            return;
        }
        let reports = self.reports.clone();
        let Some(peer) = self.selected_peer() else {
            return;
        };
        let centrals = NSArray::from_retained_slice(std::slice::from_ref(&peer.central));
        self.flush_notifications(Instant::now(), |notification| {
            let Some((_, characteristic)) =
                reports.iter().find(|(kind, _)| *kind == notification.kind)
            else {
                return false;
            };
            unsafe {
                manager.updateValue_forCharacteristic_onSubscribedCentrals(
                    &NSData::with_bytes(&notification.bytes),
                    characteristic,
                    Some(&centrals),
                )
            }
        });
    }
    fn flush_notifications(&mut self, now: Instant, mut notify: impl FnMut(&Notification) -> bool) {
        if self.blocked {
            return;
        }
        while let Some(notification) = self.queue.front() {
            if notification.generation != self.state.generation {
                self.queue.pop_front();
                continue;
            }
            if now.saturating_duration_since(notification.created) > MAX_INPUT_AGE {
                self.fail("BLE HID backpressure exceeded 250 ms".into());
                return;
            }
            let wait = if notification.kind == ReportKind::Mouse {
                self.schedule.mouse_wait(now)
            } else {
                self.schedule.wait(now)
            };
            if !wait.is_zero() {
                return;
            }
            // updateValue(true) only accepts into Core Bluetooth's queue. Like
            // Windows, pace at actual submission, not at input dequeue or at
            // the writable callback, and leave unsent motion for coalescing.
            if !notify(notification) {
                self.blocked = true;
                return;
            }
            self.schedule.sent(now);
            self.accepted_reports[notification.kind.index()] += 1;
            if let Some(notification) = self.queue.pop_front()
                && let Some(reply) = notification.reply
            {
                let _ = reply.send(Ok(()));
            }
        }
        if self.synchronizing {
            self.synchronizing = false;
            self.state.ready = true;
            for target in &mut self.state.targets {
                target.connection = if self
                    .state
                    .selected
                    .as_ref()
                    .is_some_and(|id| target.matches_id(id))
                {
                    Connection::Connected
                } else {
                    Connection::Disconnected
                };
            }
            if self.state.hid_profile == Profile::Full {
                self.link.set_ready(self.state.generation);
            }
        }
    }
    fn tick(&mut self, manager: &CBPeripheralManager) {
        if self.state.generation != self.revision.load(Ordering::Acquire) {
            self.revoke();
            self.synchronize();
        }
        if self.blocked
            && self
                .queue
                .front()
                .is_some_and(|n| n.created.elapsed() > MAX_INPUT_AGE)
        {
            self.fail("BLE HID backpressure timed out".into());
        }
        if self.epoch != self.link.epoch() || self.mapping_epoch != self.link.mapping_epoch() {
            if self.mapping_epoch != self.link.mapping_epoch() {
                self.owners.clear_mappings();
            }
            self.epoch = self.link.epoch();
            self.mapping_epoch = self.link.mapping_epoch();
            if self.state.ready {
                self.release_physical();
            }
        }
        let now = Instant::now();
        let mut index = 0;
        while index < self.pulses.len() {
            if now >= self.pulses[index].0 {
                let (_, owner, action) = self.pulses.remove(index);
                self.consumer.release(owner, action);
                self.enqueue(ReportKind::Consumer, self.consumer.report());
            } else {
                index += 1;
            }
        }
        if !self.blocked && self.queue.is_empty() && self.state.ready {
            self.schedule
                .set_mouse_report_rate(self.link.mouse_report_rate());
            match self.schedule.take(&self.input, &self.link, now) {
                Ok(Some(packet)) => {
                    let queued = self.queue.len();
                    if let Err(error) = self.event(packet.event) {
                        self.fail(error.to_string());
                    }
                    for notification in self.queue.iter_mut().skip(queued) {
                        notification.created = packet.captured;
                    }
                }
                Err(error) => self.fail(error.into()),
                _ => {}
            }
        }
        self.flush(manager);
    }
    fn release_physical(&mut self) {
        match self.session().event(Event::ReleasePhysical, false) {
            Ok(reports) => {
                for (kind, bytes) in reports {
                    if self.state.hid_profile.reports().contains(&kind) {
                        self.enqueue(kind, bytes);
                    }
                }
            }
            Err(error) => self.fail(error.to_string()),
        }
    }
    fn session(&mut self) -> hid::session::Session<'_> {
        hid::session::Session {
            keyboard: &mut self.keyboard,
            owners: &mut self.owners,
            consumer: &mut self.consumer,
            pulses: &mut self.pulses,
            next_owner: &mut self.next_owner,
        }
    }
    fn media(&mut self, action: MediaCommand, down: bool) -> Result<(), BackendError> {
        if let Some(release) = self.session().prepare_media(action, down) {
            self.enqueue(ReportKind::Consumer, release);
        }
        let bytes = self.session().media(action, down, Instant::now())?;
        self.enqueue(ReportKind::Consumer, bytes);
        Ok(())
    }
    fn event(&mut self, event: Event) -> Result<(), BackendError> {
        if let Event::Media { action, down } = event {
            return self.media(action, down);
        }
        let reverse_scroll = self.link.reverse_scroll();
        for (kind, bytes) in self.session().event(event, reverse_scroll)? {
            self.enqueue(kind, bytes);
        }
        Ok(())
    }
}
unsafe fn characteristic(
    id: u16,
    properties: CBCharacteristicProperties,
    bytes: Option<&[u8]>,
    permissions: CBAttributePermissions,
) -> Retained<CBMutableCharacteristic> {
    // Encrypted static values must be served dynamically; Core Bluetooth rejects
    // non-readable properties on characteristics with an initial static value.
    let _ = bytes;
    unsafe {
        CBMutableCharacteristic::initWithType_properties_value_permissions(
            CBMutableCharacteristic::alloc(),
            &uuid(id),
            properties,
            None,
            permissions,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taprelay_core::passthrough::KeyUsage;

    #[test]
    fn native_gatt_attributes_use_sig_widths_and_shared_report_references() {
        // Build native objects only. Never create a manager, advertise or pair.
        objc2::rc::autoreleasepool(|_| unsafe {
            let mut data = Data::new(None, Profile::MediaOnly, false);
            let service = data.service();
            assert_eq!(
                service.UUID().data().len(),
                16,
                "service workaround stays isolated"
            );
            let chars = service.characteristics().unwrap();
            assert_eq!(chars.len(), 7);
            for ch in &chars {
                assert_eq!(
                    ch.UUID().data().len(),
                    2,
                    "HID characteristic UUID must be short"
                );
            }
            assert_eq!(
                data.reports.len(),
                3,
                "media mode still publishes a stable full schema"
            );
            for (kind, ch) in &data.reports {
                assert_eq!(ch.UUID(), uuid(0x2a4d));
                assert!(ch.properties().contains(
                    CBCharacteristicProperties::Read | CBCharacteristicProperties::Notify
                ));
                assert!(
                    ch.permissions()
                        .contains(CBAttributePermissions::ReadEncryptionRequired)
                );
                let descriptors = ch.descriptors().unwrap();
                assert_eq!(descriptors.len(), 1);
                let descriptor = descriptors.objectAtIndex(0);
                assert_eq!(descriptor.UUID().data().to_vec(), [0x29, 0x08]);
                let value = descriptor.value().unwrap().downcast::<NSData>().unwrap();
                assert_eq!(value.to_vec(), kind.reference());
                assert_eq!(hid::neutral(*kind).len(), kind.payload_len());
            }
        });
    }

    #[test]
    fn selection_before_discovery_keeps_intent_without_authorizing_input() {
        let mut data = Data::new(
            Some(Target {
                id: "old".into(),
                ..Default::default()
            }),
            Profile::Full,
            true,
        );
        data.select("wanted".into());
        assert_eq!(data.state.selected.as_deref(), Some("wanted"));
        assert!(data.remembered.is_none());
        assert!(!data.state.ready && !data.link.ready());
        assert!(data.queue.is_empty() && !data.synchronizing);
        let generation = data.state.generation;
        data.state.targets = vec![Target {
            id: "unrelated".into(),
            subscribed: Knowledge::Yes,
            ..Default::default()
        }];
        data.reconcile_selection(Some("unrelated"));
        assert_eq!(data.state.selected.as_deref(), Some("wanted"));
        assert_eq!(data.state.generation, generation);
        assert!(!data.state.ready && !data.link.ready());
    }

    #[test]
    fn receiver_identity_resolves_in_either_arrival_order_and_survives_metadata_loss() {
        for inventory_first in [true, false] {
            for explicit_selection in [true, false] {
                let wanted = Target {
                    id: "macos-bt:001122334455".into(),
                    name: "Tablet".into(),
                    identity: vec!["bt-address:001122334455".into()],
                    ..Default::default()
                };
                let mut known = Target {
                    id: "uuid".into(),
                    aliases: vec![wanted.id.clone()],
                    reconnectable: true,
                    ..wanted.clone()
                };
                known.identity.push("cbcentral:uuid".into());
                let live = Target {
                    id: "uuid".into(),
                    name: "Tablet".into(),
                    identity: vec!["cbcentral:uuid".into()],
                    subscribed: Knowledge::Yes,
                    reconnectable: true,
                    ..Default::default()
                };
                let mut data = Data::new(Some(wanted.clone()), Profile::Full, true);
                data.state.adapter_state = AdapterState::Available;
                if explicit_selection {
                    data.select(wanted.id.clone());
                }
                let generation = data.state.generation;
                data.known = if inventory_first {
                    vec![known.clone()]
                } else {
                    vec![]
                };
                data.state.targets = merge_targets(
                    &data.known,
                    if inventory_first {
                        vec![]
                    } else {
                        vec![live.clone()]
                    },
                    &data.state,
                );
                data.reconcile_selection(if inventory_first { None } else { Some("uuid") });
                assert_eq!(data.state.generation, generation);
                assert_ne!(data.state.selected.as_deref(), Some("uuid"));

                data.known = vec![known];
                data.state.targets = merge_targets(&data.known, [live.clone()], &data.state);
                data.reconcile_selection(if inventory_first { Some("uuid") } else { None });
                assert_eq!(data.state.selected.as_deref(), Some("uuid"));
                assert_eq!(data.state.generation, generation + 1);
                // Resolving metadata alone cannot bypass actual native report subscriptions.
                assert!(!data.state.ready && !data.link.ready());

                data.state.ready = true;
                data.known.clear();
                data.state.targets = merge_targets(&data.known, [live], &data.state);
                data.reconcile_selection(None);
                assert_eq!(data.state.selected.as_deref(), Some("uuid"));
                assert_eq!(data.state.generation, generation + 1);
                assert!(data.state.ready);
            }
        }
    }

    #[test]
    fn paired_inventory_and_live_subscription_merge_without_changing_the_session() {
        let address_id = "macos-bt:001122334455";
        let known = Target {
            id: address_id.into(),
            name: "Tablet".into(),
            identity: vec!["bt-address:001122334455".into(), "cbcentral:uuid".into()],
            pairing: Knowledge::Yes,
            link: Knowledge::Yes,
            ..Default::default()
        };
        let live = Target {
            id: "uuid".into(),
            name: "Tablet".into(),
            identity: vec!["cbcentral:uuid".into()],
            subscribed: Knowledge::Yes,
            link: Knowledge::Yes,
            ..Default::default()
        };
        let state = Snapshot {
            selected: Some(address_id.into()),
            ready: true,
            generation: 42,
            adapter_state: AdapterState::Available,
            ..Default::default()
        };
        let rows = merge_targets(std::slice::from_ref(&known), [live.clone()], &state);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "uuid");
        assert!(rows[0].matches_id(address_id));
        assert_eq!(rows[0].pairing, Knowledge::Yes);
        assert_eq!(rows[0].connection, Connection::Connected);

        // A fresh inventory replaces old bonds; no historical pairing survives removal.
        let unpaired = merge_targets(&[], [live], &state);
        assert_eq!(unpaired[0].pairing, Knowledge::Unknown);
        let disconnected = merge_targets(
            &[known],
            [],
            &Snapshot {
                ready: false,
                ..state
            },
        );
        assert_eq!(disconnected[0].subscribed, Knowledge::Unknown);
        assert_eq!(
            disconnected[0].connection,
            Connection::AwaitingHostSubscription
        );
    }

    #[test]
    fn system_connections_do_not_create_hid_readiness_and_radio_off_clears_links() {
        let speaker = Target {
            id: "speaker".into(),
            name: "Speaker".into(),
            pairing: Knowledge::Yes,
            link: Knowledge::Yes,
            ..Default::default()
        };
        let mut data = Data::new(None, Profile::MediaOnly, true);
        data.state.adapter_state = AdapterState::Available;
        data.known = vec![speaker];
        data.rebuild_targets();
        assert!(!data.state.ready);
        assert!(data.state.selected.is_none());
        assert_eq!(data.state.targets[0].subscribed, Knowledge::Unknown);
        data.state.adapter_state = AdapterState::Disabled;
        data.rebuild_targets();
        assert_eq!(data.state.targets[0].link, Knowledge::No);
        assert_eq!(data.state.targets[0].pairing, Knowledge::Yes);
        data.known.clear();
        data.rebuild_targets();
        assert!(data.state.targets.is_empty());
    }

    #[test]
    fn resolving_a_name_preserves_selection_readiness_and_generation() {
        let remembered = Target {
            id: "receiver".into(),
            name: "My tablet".into(),
            ..Default::default()
        };
        let mut data = Data::new(Some(remembered), Profile::MediaOnly, true);
        data.state.selected = Some("receiver".into());
        data.state.ready = true;
        data.state.targets = vec![Target {
            id: "receiver".into(),
            name: "Old placeholder".into(),
            subscribed: Knowledge::Yes,
            connection: Connection::Connected,
            ..Default::default()
        }];
        let mut expected = data.state.clone();
        expected.targets[0].name = "My tablet".into();
        data.update_names();
        assert_eq!(data.state, expected);
    }
    #[test]
    fn accepted_notifications_are_paced_and_false_preserves_the_pending_release() {
        let mut data = Data::new(None, Profile::Full, true);
        data.event(Event::Key {
            usage: KeyUsage::Keyboard(4),
            down: true,
        })
        .unwrap();
        data.event(Event::Key {
            usage: KeyUsage::Keyboard(4),
            down: false,
        })
        .unwrap();
        let start = Instant::now();
        let mut received = Vec::new();
        data.flush_notifications(start, |n| {
            received.push(n.bytes.clone());
            true
        });
        assert_eq!(received.len(), 1);
        assert_ne!(received[0], hid::neutral(ReportKind::Keyboard));
        data.flush_notifications(start + Duration::from_millis(29), |_| {
            panic!("API acceptance does not grant an immediate notification credit")
        });
        data.flush_notifications(start + Duration::from_millis(30), |_| false);
        assert!(data.blocked);
        assert_eq!(data.queue.len(), 1);
        data.flush_notifications(start + Duration::from_millis(31), |_| {
            panic!("must wait for the Core Bluetooth writable callback")
        });
        data.blocked = false;
        data.flush_notifications(start + Duration::from_millis(32), |n| {
            received.push(n.bytes.clone());
            true
        });
        assert_eq!(received[1], hid::neutral(ReportKind::Keyboard));
        assert!(data.queue.is_empty());
    }

    #[test]
    fn rejected_reports_expire_and_retired_generations_never_reach_the_transport() {
        let mut data = Data::new(None, Profile::Full, true);
        data.enqueue(ReportKind::Mouse, vec![1]);
        let created = data.queue[0].created;
        data.flush_notifications(created, |_| false);
        data.blocked = false;
        data.flush_notifications(created + MAX_INPUT_AGE + Duration::from_nanos(1), |_| {
            panic!("expired input must not be delivered")
        });
        assert!(data.queue.is_empty());
        assert!(data.link.take_failure().unwrap().contains("250 ms"));
        data.enqueue(ReportKind::Mouse, vec![2]);
        data.revoke();
        data.flush_notifications(Instant::now(), |_| {
            panic!("retired input must not be delivered")
        });
        assert!(data.queue.is_empty());
    }
}
