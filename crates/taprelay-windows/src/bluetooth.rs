mod discovery;
mod maintenance;
mod restore;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use taprelay_core::devices::{
    AdapterState, Availability, Coordinator, DeviceError, PairingHandoff,
};
use taprelay_core::{
    command::{CommandPhase, QueuedCommand},
    hid,
    metadata::{Metadata, MetadataCache},
    ports::BackendError,
    state::{Knowledge, Snapshot, Target, TransportActivity},
};
use tokio::sync::watch;
use windows::{
    Devices::{
        Bluetooth::{BluetoothError, BluetoothLEDevice, GenericAttributeProfile::*},
        Enumeration::{DeviceInformation, DeviceInformationKind},
        Radios::{Radio, RadioState},
    },
    Foundation::{IPropertyValue, TypedEventHandler},
    Storage::Streams::{DataReader, DataWriter, IBuffer},
    core::{GUID, HSTRING, Interface},
};

fn identity_properties() -> windows_collections::IIterable<HSTRING> {
    vec![
        HSTRING::from("System.Devices.Aep.IsPresent"),
        HSTRING::from("System.Devices.Aep.IsConnected"),
        HSTRING::from("System.Devices.Aep.IsPaired"),
        HSTRING::from("System.Devices.Aep.Bluetooth.Cod.Major"),
        HSTRING::from("System.Devices.Aep.Bluetooth.Cod.Minor"),
        HSTRING::from("System.Devices.Aep.Bluetooth.Le.Appearance.Category"),
        HSTRING::from("System.Devices.Aep.ContainerId"),
        HSTRING::from("System.Devices.Aep.DeviceAddress"),
        // BluetoothLEDevice.DeviceInformation is often a Device object rather
        // than an AssociationEndpoint. Request both identity shapes so a GATT
        // session can be joined to the discovery row for the same peer.
        HSTRING::from("System.Devices.ContainerId"),
        HSTRING::from("System.Devices.DeviceInstanceId"),
        HSTRING::from("System.Devices.Parent"),
    ]
    .into()
}
fn device_identity(info: &DeviceInformation) -> Vec<String> {
    let Ok(properties) = info.Properties() else {
        return vec![];
    };
    let mut keys = vec![];
    for property in [
        "System.Devices.Aep.ContainerId",
        "System.Devices.ContainerId",
    ] {
        if let Ok(guid) = properties
            .Lookup(&HSTRING::from(property))
            .and_then(|v| v.cast::<IPropertyValue>())
            .and_then(|v| v.GetGuid())
            && guid != GUID::from_u128(0)
        {
            keys.push(format!("aep-container:{guid:?}"));
        }
    }
    for property in ["System.Devices.DeviceInstanceId", "System.Devices.Parent"] {
        if let Ok(value) = properties
            .Lookup(&HSTRING::from(property))
            .and_then(|v| v.cast::<IPropertyValue>())
            .and_then(|v| v.GetString())
        {
            let value = value.to_string();
            if !value.is_empty() {
                keys.push(format!("device-instance:{value}"));
            }
        }
    }
    if let Ok(address) = properties
        .Lookup(&HSTRING::from("System.Devices.Aep.DeviceAddress"))
        .and_then(|v| v.cast::<IPropertyValue>())
        .and_then(|v| v.GetString())
    {
        let address: String = address
            .to_string()
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if address.len() == 12 {
            keys.push(format!("bluetooth-address:{address}"));
        }
    }
    keys
}
fn add_bluetooth_address(keys: &mut Vec<String>, address: u64) {
    if address != 0 {
        keys.push(format!("bluetooth-address:{address:012x}"));
    }
}

const HID_SERVICE: u16 = 0x1812;
const BATTERY_SERVICE: u16 = 0x180f;
const HID_INFORMATION_CHARACTERISTIC: u16 = 0x2a4a;
const REPORT_MAP_CHARACTERISTIC: u16 = 0x2a4b;
const HID_CONTROL_POINT_CHARACTERISTIC: u16 = 0x2a4c;
const REPORT_CHARACTERISTIC: u16 = 0x2a4d;
const PROTOCOL_MODE_CHARACTERISTIC: u16 = 0x2a4e;
const PROTOCOL_MODE: [u8; 1] = [1];
// Windows reserves the Device Information Service (0x180A), so do not try to
// publish it: CreateAsync returns DisabledByPolicy and aborts the whole build.
const ADVERTISED_SERVICES: &[u16] = &[HID_SERVICE, BATTERY_SERVICE];
#[cfg(test)]
const HID_CHARACTERISTICS: &[u16] = &[
    HID_INFORMATION_CHARACTERISTIC,
    REPORT_MAP_CHARACTERISTIC,
    REPORT_CHARACTERISTIC,
    HID_CONTROL_POINT_CHARACTERISTIC,
    PROTOCOL_MODE_CHARACTERISTIC,
];

fn extended_identity(info: &DeviceInformation) -> windows::core::Result<Vec<String>> {
    let keys = device_identity(info);
    if !keys.is_empty() {
        return Ok(keys);
    }
    let kind = info.Kind()?;
    let details = DeviceInformation::CreateFromIdAsyncWithKindAndAdditionalProperties(
        &info.Id()?,
        &identity_properties(),
        kind,
    )?
    .join()?;
    let keys = device_identity(&details);
    if !keys.is_empty() || kind == DeviceInformationKind::AssociationEndpoint {
        return Ok(keys);
    }
    // A GATT session can expose a Device id while discovery owns the related
    // AssociationEndpoint. Ask Windows for that endpoint as a final identity
    // bridge; a failure here is non-fatal because the live Bluetooth address
    // is added by the caller when available.
    Ok(
        DeviceInformation::CreateFromIdAsyncWithKindAndAdditionalProperties(
            &info.Id()?,
            &identity_properties(),
            DeviceInformationKind::AssociationEndpoint,
        )?
        .join()
        .map(|endpoint| device_identity(&endpoint))
        .unwrap_or(keys),
    )
}

pub enum Request {
    Select(Option<String>),
    Send(
        QueuedCommand,
        tokio::sync::oneshot::Sender<Result<(), String>>,
    ),
    Refresh,
    Restart,
    Pair(String),
}
pub struct BleHandle {
    commands: mpsc::SyncSender<Request>,
    pub state: watch::Receiver<Snapshot>,
    stop: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl BleHandle {
    pub fn request(&self, request: Request) -> Result<(), BackendError> {
        self.commands
            .try_send(request)
            .map_err(|_| BackendError::Unavailable("Bluetooth control queue unavailable".into()))?;
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        self.state.has_changed().is_err() || self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    pub fn invalidate_commands(&self) -> u64 {
        // UI invalidation and native subscription/radio/session transitions
        // share one monotonic generation; queued input may not cross it.
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub fn start(remembered: Option<Target>) -> Result<Self, BackendError> {
        let (commands, rx) = mpsc::sync_channel(8);
        let (tx, state) = watch::channel(Snapshot {
            service: false,
            ..Default::default()
        });
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let revision = Arc::new(AtomicU64::new(1));
        let worker_revision = revision.clone();
        let thread = thread::Builder::new()
            .name("taprelay-bluetooth".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), BackendError> {
                        let startup_restore = Rc::new(RefCell::new(restore::Restore::new(
                            remembered,
                            Instant::now(),
                        )));
                        let _apartment = super::Apartment::new()?;
                        let mut discovery = discovery::Discovery::new();
                        let manager = Rc::new(RefCell::new(Coordinator::default()));
                        let mut service_retry_at = Instant::now();
                        let mut server = None;
                        let mut restart = true;
                        let mut refresh_at = Instant::now();
                        while !stopping.load(Ordering::Acquire) {
                            let previous_adapter = discovery.adapter;
                            let previous_adapter_id = discovery.adapter_id.clone();
                            if let Err(e) = discovery.tick() {
                                tracing::error!("Bluetooth discovery: {e}");
                                discovery.fail();
                            }
                            if discovery.adapter == AdapterState::Available
                                && (previous_adapter != AdapterState::Available
                                    || previous_adapter_id != discovery.adapter_id)
                            {
                                worker_revision.fetch_add(1, Ordering::AcqRel);
                                restart = true;
                            }
                            if server.is_none() && Instant::now() >= service_retry_at {
                                restart = true;
                            }
                            if restart && discovery.adapter == AdapterState::Available {
                                // Drop the old server on its own worker before publishing another.
                                server.take();
                                manager.borrow_mut().disconnect();
                                tx.send_replace(Snapshot {
                                    generation: worker_revision.load(Ordering::Acquire),
                                    activity: TransportActivity::CheckingEnvironment,
                                    ..Default::default()
                                });
                                match Server::create(
                                    tx.clone(),
                                    worker_revision.clone(),
                                    manager.clone(),
                                    startup_restore.clone(),
                                ) {
                                    Ok(s) => server = Some(s),
                                    Err(e) => {
                                        tracing::error!("{e}");
                                        tx.send_modify(|s| s.last_error = Some(e.to_string()));
                                    }
                                }
                                restart = false;
                                service_retry_at = Instant::now() + Duration::from_secs(5);
                                refresh_at = Instant::now();
                            }
                            if Instant::now() >= refresh_at {
                                let candidates = discovery.inventory.merged();
                                if let Some(s) = &mut server {
                                    s.connected = candidates;
                                    s.state.adapter_state = discovery.adapter;
                                    s.state.discovery = discovery.state;
                                    if startup_restore
                                        .borrow()
                                        .refresh_due(discovery.adapter, Instant::now())
                                    {
                                        if let Err(e) = s.refresh() {
                                            s.fail(&e);
                                        } else {
                                            s.try_startup_restore();
                                            startup_restore.borrow_mut().observe(
                                                &s.state,
                                                worker_revision.load(Ordering::Acquire),
                                            );
                                        }
                                    } else if discovery.adapter != AdapterState::Available {
                                        s.state.ready = false;
                                        s.state.adapter = false;
                                        s.state.broadcasting = false;
                                    }
                                    if s.state.selected.is_none() {
                                        manager.borrow_mut().disconnect();
                                    }
                                    s.publish();
                                } else {
                                    tx.send_modify(|state| {
                                        state.targets = candidates;
                                        state.adapter_state = discovery.adapter;
                                        state.adapter =
                                            discovery.adapter == AdapterState::Available;
                                        state.discovery = discovery.state;
                                        state.selected = None;
                                        manager.borrow_mut().reconcile(state, Instant::now());
                                    });
                                }
                                refresh_at = Instant::now() + Duration::from_millis(100);
                            }
                            let request = match rx.try_recv() {
                                Ok(request) => Ok(request),
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    Err(mpsc::RecvTimeoutError::Disconnected)
                                }
                                Err(mpsc::TryRecvError::Empty) => {
                                    thread::park_timeout(
                                        refresh_at.saturating_duration_since(Instant::now()),
                                    );
                                    Err(mpsc::RecvTimeoutError::Timeout)
                                }
                            };
                            match request {
                                Ok(Request::Restart) => {
                                    restart = true;
                                }
                                Ok(Request::Refresh) => {
                                    discovery.restart();
                                    refresh_at = Instant::now();
                                }
                                Ok(Request::Pair(id)) => {
                                    startup_restore.borrow_mut().cancel("pair");
                                    let candidate = tx
                                        .borrow()
                                        .targets
                                        .iter()
                                        .find(|t| t.matches_id(&id))
                                        .cloned();
                                    if let Some(target) = candidate {
                                        if !manager.borrow_mut().pair(target, Instant::now()) {
                                            continue;
                                        }
                                        if let Err(e) =
                                            super::desktop::open("ms-settings:bluetooth")
                                        {
                                            tracing::error!("Open pairing settings: {e}");
                                            manager.borrow_mut().cancel_pairing();
                                            if let Some(s) = &mut server {
                                                s.state.device_error =
                                                    Some(DeviceError::PairingLaunchFailed);
                                                s.state.pairing_handoff = PairingHandoff::Idle;
                                            }
                                        } else if let Some(s) = &mut server {
                                            s.state.pairing_handoff = PairingHandoff::WaitingForOS;
                                        }
                                    }
                                }
                                Ok(Request::Select(id)) => {
                                    startup_restore.borrow_mut().cancel(if id.is_some() {
                                        "select"
                                    } else {
                                        "disconnect"
                                    });
                                    if let Some(id) = &id {
                                        let accepted = if let Some(server) = &server {
                                            manager.borrow_mut().connect(
                                                &server.state,
                                                id.clone(),
                                                Instant::now(),
                                            )
                                        } else {
                                            let state = tx.borrow();
                                            manager.borrow_mut().connect(
                                                &state,
                                                id.clone(),
                                                Instant::now(),
                                            )
                                        };
                                        if !accepted {
                                            continue;
                                        }
                                    } else {
                                        manager.borrow_mut().disconnect();
                                    }
                                    if let Some(s) = &mut server {
                                        s.state.device_error = None;
                                        s.state.pairing_handoff = PairingHandoff::Idle;
                                        if let Err(e) = s.select(id) {
                                            s.fail(&e);
                                        }
                                    } else {
                                        manager.borrow_mut().disconnect();
                                    }
                                }
                                Ok(Request::Send(c, reply)) => {
                                    let result = if let Some(s) = &mut server {
                                        s.send(c)
                                    } else {
                                        Err(BackendError::Unavailable(
                                            "Bluetooth service is not running".into(),
                                        ))
                                    };
                                    let _ = reply.send(
                                        result.as_ref().map(|_| ()).map_err(ToString::to_string),
                                    );
                                    if let Err(e) = result
                                        && !matches!(e, BackendError::Stale)
                                        && let Some(s) = &mut server
                                    {
                                        s.fail(&e);
                                    }
                                }

                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                            }
                        }
                        // The input hook is already released. Clear every HID
                        // collection before dropping the service; never enqueue
                        // a shutdown release behind stale pointer movement.
                        if let Some(s) = &mut server {
                            for (index, kind) in hid::ReportKind::ALL.into_iter().enumerate() {
                                if let Some(client) = s.selected_clients[index].clone() {
                                    let bytes = vec![0; kind.payload_len()];
                                    if let Err(error) = s.notify(kind, &client, &bytes) {
                                        tracing::warn!(?error, "Shutdown HID release failed");
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(())
                    },
                ))
                .unwrap_or_else(|_| {
                    Err(BackendError::Unavailable(
                        "Bluetooth worker panicked; retry to restart".into(),
                    ))
                });
                if let Err(e) = outcome {
                    tracing::error!("{e}");
                    tx.send_modify(|s| {
                        taprelay_core::devices::revoke_session(s);
                        s.service = false;
                        s.last_error = Some(e.to_string());
                    });
                }
            })
            .map_err(|e| BackendError::Unavailable(format!("Start Bluetooth worker: {e}")))?;
        Ok(Self {
            commands,
            state,
            stop,
            revision,
            thread: Some(thread),
        })
    }
}
impl Drop for BleHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.try_send(Request::Refresh);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !t.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if t.is_finished() {
                if t.join().is_err() {
                    tracing::error!("Bluetooth worker panicked during shutdown");
                }
            } else {
                tracing::error!(
                    "Native operation still pending at shutdown; remote release cannot be guaranteed"
                );
            }
        }
    }
}
fn uuid(id: u16) -> GUID {
    GUID::from_u128(((id as u128) << 96) | 0x00001000800000805f9b34fb)
}
fn buffer(bytes: &[u8]) -> windows::core::Result<IBuffer> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(bytes)?;
    writer.DetachBuffer()
}
fn api<T>(name: &'static str, r: windows::core::Result<T>) -> Result<T, BackendError> {
    r.map_err(|e| super::native_error(name, e))
}
fn callback(f: impl FnOnce() -> windows::core::Result<()>) -> windows::core::Result<()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            tracing::error!("GATT request: {e}");
            Err(e)
        }
        Err(_) => {
            tracing::error!("GATT callback panicked; request failed (see panic log)");
            Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            )))
        }
    }
}

fn active_subscriber(target: Option<&Target>) -> bool {
    target.is_some_and(|t| t.link == Knowledge::Yes && t.subscribed == Knowledge::Yes)
}

fn startup_restore_candidate(
    remembered: &Target,
    targets: &[Target],
    suspended: &std::collections::BTreeMap<String, bool>,
) -> Option<String> {
    targets
        .iter()
        .find(|target| {
            target.same_device(remembered)
                && active_subscriber(Some(target))
                && !suspended.get(&target.id).copied().unwrap_or(false)
        })
        .map(|target| target.id.clone())
}

fn selected_subscriber_index(
    selected: &str,
    targets: &[Target],
    subscribers: &[Target],
) -> Option<usize> {
    let selected_target = targets.iter().find(|target| target.matches_id(selected));
    subscribers.iter().position(|subscriber| {
        selected_target.is_some_and(|target| target.same_device(subscriber))
            || subscriber.matches_id(selected)
    })
}

// Every publication, including refresh/notify intermediate snapshots, must use
// the same lifecycle projection as the worker loop.
fn publish_snapshot(
    updates: &watch::Sender<Snapshot>,
    state: &mut Snapshot,
    manager: &mut Coordinator,
) {
    manager.reconcile(state, Instant::now());
    if *updates.borrow() != *state {
        updates.send_replace(state.clone());
    }
}

struct ReportCharacteristic {
    kind: hid::ReportKind,
    characteristic: GattLocalCharacteristic,
    read_token: i64,
    subscription_token: i64,
    current: Arc<Mutex<Vec<u8>>>,
}

struct Server {
    manager: Rc<RefCell<Coordinator>>,
    providers: Vec<GattServiceProvider>,
    reports: Vec<ReportCharacteristic>,
    control: Option<GattLocalCharacteristic>,
    protocol: Option<GattLocalCharacteristic>,
    write_token: Option<i64>,
    protocol_token: Option<i64>,
    session: Option<(GattSession, i64)>,
    maintained_session: Option<GattSession>,
    session_active: bool,
    radio: Radio,
    radio_token: Option<i64>,
    advertisement_tokens: Vec<i64>,
    revision: Arc<AtomicU64>,
    suspended: Arc<Mutex<std::collections::BTreeMap<String, bool>>>,
    selected_clients: [Option<GattSubscribedClient>; 1],
    consumer: hid::ConsumerState,
    pending_pulses: Vec<(Instant, u64, taprelay_core::command::MediaCommand)>,
    next_pulse_owner: u64,
    state: Snapshot,
    updates: watch::Sender<Snapshot>,
    synced: bool,
    synced_reports: [bool; 1],
    generation_started: Instant,
    retry_at: Instant,
    maintenance: maintenance::Maintenance,
    connected: Vec<Target>,
    metadata: MetadataCache,
    startup_restore: Rc<RefCell<restore::Restore>>,
}
impl Server {
    fn create(
        updates: watch::Sender<Snapshot>,
        revision: Arc<AtomicU64>,
        manager: Rc<RefCell<Coordinator>>,
        startup_restore: Rc<RefCell<restore::Restore>>,
    ) -> Result<Self, BackendError> {
        let d = super::diagnostics::doctor()?;
        tracing::info!(adapter = ?d.adapter_id, radio = ?d.radio_name, peripheral = ?d.peripheral_role, low_energy = ?d.low_energy, "Bluetooth capabilities inspected");
        updates.send_modify(|s| {
            s.adapter = d.adapter_present && d.radio_on == Some(true) && d.low_energy == Some(true);
            s.peripheral = d.peripheral_role == Some(true);
        });
        if let Some(issue) = d.issue {
            return Err(BackendError::Unavailable(issue));
        }
        let radio = api(
            "BluetoothAdapter.GetRadioAsync",
            (|| {
                windows::Devices::Bluetooth::BluetoothAdapter::GetDefaultAsync()?
                    .join()?
                    .GetRadioAsync()?
                    .join()
            })(),
        )?;
        let initial_generation = revision.load(Ordering::Acquire);
        let mut s = Self {
            manager,
            providers: vec![],
            reports: vec![],
            control: None,
            protocol: None,
            write_token: None,
            protocol_token: None,
            session: None,
            maintained_session: None,
            session_active: false,
            radio,
            radio_token: None,
            advertisement_tokens: vec![],
            revision,
            suspended: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            selected_clients: std::array::from_fn(|_| None),
            consumer: hid::ConsumerState::default(),
            pending_pulses: vec![],
            next_pulse_owner: 0,
            state: Snapshot {
                generation: initial_generation,
                activity: TransportActivity::Publishing,
                adapter: true,
                peripheral: true,
                service: false,
                ..Default::default()
            },
            updates,
            synced: false,
            synced_reports: [false; 1],
            generation_started: Instant::now(),
            retry_at: Instant::now(),
            maintenance: maintenance::Maintenance::new(Instant::now()),
            connected: vec![],
            metadata: MetadataCache::default(),
            startup_restore,
        };
        api("GATT service construction", s.build())?;
        // StartAdvertising completes asynchronously and can transiently report Aborted.
        // Do not start it again while the original request is still progressing.
        s.retry_at = Instant::now() + Duration::from_secs(3);
        s.state.service = true;
        s.state.activity = TransportActivity::Idle;
        s.publish();
        Ok(s)
    }
    fn build(&mut self) -> windows::core::Result<()> {
        let rev = self.revision.clone();
        self.radio_token = Some(self.radio.StateChanged(&TypedEventHandler::new(
            move |_, _| {
                callback(|| {
                    rev.fetch_add(1, Ordering::AcqRel);
                    tracing::info!("Bluetooth radio state changed");
                    Ok(())
                })
            },
        ))?);
        for service in ADVERTISED_SERVICES {
            let result = GattServiceProvider::CreateAsync(uuid(*service))?.join()?;
            if result.Error()? != BluetoothError::Success {
                return Err(windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    format!("GattServiceProvider {service:04x}: {:?}", result.Error()?),
                ));
            }
            let p = result.ServiceProvider()?;
            self.providers.push(p.clone());
            self.advertisement_tokens
                .push(p.AdvertisementStatusChanged(&TypedEventHandler::<
                    GattServiceProvider,
                    GattServiceProviderAdvertisementStatusChangedEventArgs,
                >::new(move |_, args| {
                    callback(|| {
                        if let Some(args) = args.as_ref() {
                            tracing::info!(
                                "Advertising {service:04x}: {:?}; error={:?}",
                                args.Status()?,
                                args.Error()?
                            );
                        }
                        Ok(())
                    })
                }))?);
            if *service == HID_SERVICE {
                characteristic(
                    &p,
                    HID_INFORMATION_CHARACTERISTIC,
                    GattCharacteristicProperties::Read,
                    Some(&hid::HID_INFORMATION),
                )?;
                characteristic(
                    &p,
                    REPORT_MAP_CHARACTERISTIC,
                    GattCharacteristicProperties::Read,
                    Some(hid::REPORT_MAP),
                )?;
                let protocol = characteristic(
                    &p,
                    PROTOCOL_MODE_CHARACTERISTIC,
                    GattCharacteristicProperties::Read
                        | GattCharacteristicProperties::WriteWithoutResponse,
                    Some(&PROTOCOL_MODE),
                )?;
                self.protocol_token = Some(protocol.WriteRequested(&TypedEventHandler::<
                    GattLocalCharacteristic,
                    GattWriteRequestedEventArgs,
                >::new(
                    move |_, args| {
                        callback(|| {
                            let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
                            let deferral = args.GetDeferral()?;
                            let result = (|| {
                                let request = args.GetRequestAsync()?.join()?;
                                let reader = DataReader::FromBuffer(&request.Value()?)?;
                                if request.Offset()? == 0 && reader.UnconsumedBufferLength()? == 1 {
                                    let value = reader.ReadByte()?;
                                    if value != PROTOCOL_MODE[0] {
                                        tracing::warn!(
                                            value,
                                            "Ignoring unsupported HID protocol mode"
                                        );
                                    }
                                }
                                Ok(())
                            })();
                            deferral.Complete()?;
                            result
                        })
                    },
                ))?);
                self.protocol = Some(protocol);
                for kind in hid::ReportKind::ALL {
                    let report = self.create_report_characteristic(&p, kind)?;
                    self.reports.push(report);
                }
                let control = characteristic(
                    &p,
                    HID_CONTROL_POINT_CHARACTERISTIC,
                    GattCharacteristicProperties::WriteWithoutResponse,
                    None,
                )?;
                self.control = Some(control.clone());
                let suspended = self.suspended.clone();
                let rev = self.revision.clone();
                self.write_token =
                    Some(control.WriteRequested(&TypedEventHandler::<
                        GattLocalCharacteristic,
                        GattWriteRequestedEventArgs,
                    >::new(move |_, args| {
                        callback(|| {
                            let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
                            let deferral = args.GetDeferral()?;
                            let result = (|| {
                                let r = args.GetRequestAsync()?.join()?;
                                let reader = DataReader::FromBuffer(&r.Value()?)?;
                                if r.Offset()? == 0 && reader.UnconsumedBufferLength()? == 1 {
                                    let v = reader.ReadByte()?;
                                    if v <= 1 {
                                        let id = args.Session()?.DeviceId()?.Id()?.to_string();
                                        suspended
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .insert(id, v == 0);
                                        rev.fetch_add(1, Ordering::AcqRel);
                                        tracing::info!("HID suspend={}", v == 0);
                                    }
                                }
                                Ok(())
                            })();
                            deferral.Complete()?;
                            result
                        })
                    }))?);
            } else if *service == BATTERY_SERVICE {
                characteristic(&p, 0x2a19, GattCharacteristicProperties::Read, Some(&[100]))?;
            }
            advertise(&p)?;
            tracing::info!("GATT service {service:04x} created; advertising requested");
        }
        Ok(())
    }

    fn create_report_characteristic(
        &mut self,
        service: &GattServiceProvider,
        kind: hid::ReportKind,
    ) -> windows::core::Result<ReportCharacteristic> {
        let characteristic = characteristic(
            service,
            REPORT_CHARACTERISTIC,
            GattCharacteristicProperties::Read | GattCharacteristicProperties::Notify,
            None,
        )?;
        let params = GattLocalDescriptorParameters::new()?;
        params.SetStaticValue(&buffer(&kind.reference())?)?;
        params.SetReadProtectionLevel(GattProtectionLevel::EncryptionRequired)?;
        let reference = characteristic
            .CreateDescriptorAsync(uuid(0x2908), &params)?
            .join()?;
        if reference.Error()? != BluetoothError::Success {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                format!("Report Reference {:?}: {:?}", kind, reference.Error()?),
            ));
        }

        let current = Arc::new(Mutex::new(hid::neutral(kind)));
        let read_current = current.clone();
        let read_token = characteristic.ReadRequested(&TypedEventHandler::<
            GattLocalCharacteristic,
            GattReadRequestedEventArgs,
        >::new(move |_, args| {
            callback(|| {
                let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
                let deferral = args.GetDeferral()?;
                let result = (|| {
                    let request = args.GetRequestAsync()?.join()?;
                    let offset = request.Offset()? as usize;
                    let value = read_current
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone();
                    if offset > value.len() {
                        request.RespondWithProtocolError(7)
                    } else {
                        request.RespondWithValue(&buffer(&value[offset..])?)
                    }
                })();
                deferral.Complete()?;
                result
            })
        }))?;
        let revision = self.revision.clone();
        let subscription_token =
            characteristic.SubscribedClientsChanged(&TypedEventHandler::new(move |_, _| {
                callback(|| {
                    revision.fetch_add(1, Ordering::AcqRel);
                    tracing::info!(?kind, "HID report subscription changed");
                    Ok(())
                })
            }))?;
        Ok(ReportCharacteristic {
            kind,
            characteristic,
            read_token,
            subscription_token,
            current,
        })
    }

    fn report(&self, kind: hid::ReportKind) -> &ReportCharacteristic {
        self.reports
            .iter()
            .find(|report| report.kind == kind)
            .expect("all HID report characteristics are constructed together")
    }

    fn reset_report_state(&mut self) {
        for report in &self.reports {
            *report
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = hid::neutral(report.kind);
        }
    }

    fn maintenance_report(&self, kind: hid::ReportKind) -> Vec<u8> {
        self.report(kind)
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn publish(&mut self) {
        publish_snapshot(
            &self.updates,
            &mut self.state,
            &mut self.manager.borrow_mut(),
        );
    }
    fn fail(&mut self, e: &BackendError) {
        let restoring = self.startup_restore.borrow().pending();
        if restoring {
            self.startup_restore.borrow_mut().failure(Instant::now());
        } else {
            self.maintenance.failure(Instant::now());
        }
        self.synced = false;
        self.synced_reports = [false; 1];
        self.consumer = hid::ConsumerState::default();
        self.pending_pulses.clear();
        self.reset_report_state();
        self.state.ready = false;
        self.state.last_error = Some(if !restoring && self.maintenance.failures >= 6 {
            format!(
                "Automatic recovery stopped after repeated failures; disconnect and connect again: {e}"
            )
        } else {
            e.to_string()
        });
        self.state.device_error = Some(DeviceError::ConnectionFailed);
        self.retry_at = Instant::now() + Duration::from_secs(3);
        if restoring || self.maintenance.failures == 1 {
            tracing::error!("{e}; retrying with backoff");
        } else if self.maintenance.failures == 6 {
            tracing::error!(
                "Automatic recovery stopped after 6 failures; disconnect and connect again: {e}"
            );
        }
        self.publish();
    }
    fn refresh(&mut self) -> Result<(), BackendError> {
        api("GATT state refresh", self.refresh_native())
    }
    fn refresh_native(&mut self) -> windows::core::Result<()> {
        let revision = self.revision.load(Ordering::Acquire);
        if revision != self.state.generation {
            self.state.generation = revision;
            self.generation_started = Instant::now();
            self.synced = false;
            self.synced_reports = [false; 1];
            self.consumer = hid::ConsumerState::default();
            self.pending_pulses.clear();
            self.reset_report_state();
        }
        let radio_on = self.radio.State()? == RadioState::On;
        self.state.adapter = radio_on;
        self.state.broadcasting = matches!(
            self.providers[0].AdvertisementStatus()?,
            GattServiceProviderAdvertisementStatus::Started
                | GattServiceProviderAdvertisementStatus::StartedWithoutAllAdvertisementData
        );
        if radio_on && Instant::now() >= self.retry_at {
            for p in &self.providers {
                if matches!(
                    p.AdvertisementStatus()?,
                    GattServiceProviderAdvertisementStatus::Stopped
                        | GattServiceProviderAdvertisementStatus::Aborted
                ) {
                    advertise(p)?;
                    self.retry_at = Instant::now() + Duration::from_secs(3);
                }
            }
        }
        let clients = self
            .report(hid::ReportKind::Consumer)
            .characteristic
            .SubscribedClients()?;
        let mut targets = self.connected.clone();
        for target in &mut targets {
            if let Some(previous) = self.state.targets.iter().find(|t| t.same_device(target)) {
                target.aliases.extend(previous.aliases.iter().cloned());
                target.aliases.push(previous.id.clone());
                target.aliases.sort();
                target.aliases.dedup();
            }
        }
        let mut subscribers = vec![];
        let mut subscriber_targets = vec![];
        for client in clients {
            let session = client.Session()?;
            let id = session.DeviceId()?.Id()?.to_string();
            let updates = self.updates.clone();
            let Metadata { name, pairing, identity } = self.metadata.resolve(&id, Instant::now(), || {
                updates.send_modify(|s| s.activity = TransportActivity::ResolvingDevice);
                let resolving_started = Instant::now();
                tracing::debug!(device_id = %id, "Resolving subscriber identity");
                match BluetoothLEDevice::FromIdAsync(&windows::core::HSTRING::from(&id))
                    .and_then(|p| p.join())
                {
                    Ok(device) => {
                        tracing::info!(
                            elapsed_ms = resolving_started.elapsed().as_millis(),
                            "Subscriber identity resolved"
                        );
                        let name = device
                            .Name()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|e| { tracing::warn!("Subscriber name: {e}"); "Unknown".into() });
                        let pairing = device
                            .DeviceInformation()
                            .and_then(|i| i.Pairing())
                            .and_then(|p| p.IsPaired())
                            .map(|p| if p { Knowledge::Yes } else { Knowledge::No })
                            .unwrap_or_else(|e| { tracing::warn!("Subscriber pairing: {e}"); Knowledge::Unknown });
                        let mut identity = device
                            .DeviceInformation()
                            .and_then(|info| extended_identity(&info))
                            .unwrap_or_else(|e| {
                                tracing::warn!("Subscriber physical identity: {e}");
                                vec![]
                            });
                        if let Ok(address) = device.BluetoothAddress() {
                            add_bluetooth_address(&mut identity, address);
                        }
                        identity.sort();
                        identity.dedup();
                        if let Err(e) = device.Close() { tracing::warn!("Close subscriber metadata handle: {e}"); }
                        Metadata { name, pairing, identity }
                    }
                    Err(e) => {
                        tracing::warn!(elapsed_ms = resolving_started.elapsed().as_millis(), error = %e, "Subscriber identity resolution failed");
                        Metadata { name: "Unknown".into(), pairing: Knowledge::Unknown, identity: vec![] }
                    }
                }
            });
            let link = if session.SessionStatus()? == GattSessionStatus::Active {
                Knowledge::Yes
            } else {
                Knowledge::No
            };
            let mut target = Target {
                id: id.clone(),
                name,
                pairing,
                link,
                subscribed: Knowledge::Yes,
                availability: Availability::Nearby,
                identity,
                ..Default::default()
            };
            // Keep selected endpoint aliases across discovery refreshes, but never stale readiness.
            if let Some(previous) = self.state.targets.iter().find(|t| t.same_device(&target)) {
                target.aliases = previous.aliases.clone();
                target.aliases.push(previous.id.clone());
            }
            subscriber_targets.push(target.clone());
            taprelay_core::state::upsert_target(&mut targets, target);
            subscribers.push((id, client));
        }
        self.metadata
            .retain(|id| subscribers.iter().any(|(live, _)| live == id));
        let selected_index = self
            .state
            .selected
            .as_ref()
            .and_then(|id| selected_subscriber_index(id, &targets, &subscriber_targets));
        let selected_client = selected_index
            .and_then(|index| subscribers.get(index))
            .map(|(_, client)| client.clone());
        let next_selected_clients = [selected_client.clone()];
        // Discovery's generic Bluetooth link must not override the live HID session.
        let selected_session_active =
            active_subscriber(selected_index.and_then(|index| subscriber_targets.get(index)));
        let selected_link_lost = self.session_active && !selected_session_active;
        let selected_client_replaced = self.selected_clients[0].is_some()
            && selected_client.is_some()
            && self.selected_clients[0] != selected_client;
        let selected_client_lost = self.selected_clients[0].is_some()
            && (selected_client.is_none() || selected_link_lost || selected_client_replaced);
        // Keep the session and its connection request alive while inactive.
        // Sending remains gated separately by the live session status.
        let client_changed = self.selected_clients != next_selected_clients;
        if client_changed || self.session_active != selected_session_active {
            self.synced = false;
            self.synced_reports = [false; 1];
            self.consumer = hid::ConsumerState::default();
            self.pending_pulses.clear();
            self.reset_report_state();
            self.state.generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
            self.generation_started = Instant::now();
        }
        self.session_active = selected_session_active;
        if client_changed {
            self.release_maintenance();
            if let Some((s, token)) = self.session.take() {
                s.RemoveSessionStatusChanged(token)?;
            }
            if let Some(client) = &next_selected_clients[0] {
                let session = client.Session()?;
                self.enable_connection_maintenance(client);
                let rev = self.revision.clone();
                let token =
                    session.SessionStatusChanged(&TypedEventHandler::new(move |_, _| {
                        callback(|| {
                            rev.fetch_add(1, Ordering::AcqRel);
                            tracing::info!("Target session changed");
                            Ok(())
                        })
                    }))?;
                self.session = Some((session, token));
            }
        }
        self.selected_clients = next_selected_clients;
        if selected_client_lost {
            tracing::info!(
                subscriber_missing = selected_index.is_none(),
                session_inactive = selected_link_lost,
                client_replaced = selected_client_replaced,
                "Selected HID transport interrupted; retaining target for resynchronization"
            );
            self.state.ready = false;
            if self.maintenance.failures == 0 {
                self.state.device_error = None;
            }
        }

        let previous_selected = self.state.selected_target().cloned();
        if let Some(id) = self.state.selected.clone()
            && !targets.iter().any(|target| target.matches_id(&id))
        {
            let mut target = previous_selected.unwrap_or_else(|| Target {
                id: id.clone(),
                name: "Unknown".into(),
                ..Default::default()
            });
            target.subscribed = Knowledge::No;
            target.link = match &self.session {
                Some((session, _)) if session.DeviceId()?.Id()? == id.as_str() => {
                    if session.SessionStatus()? == GattSessionStatus::Active {
                        Knowledge::Yes
                    } else {
                        Knowledge::No
                    }
                }
                _ => Knowledge::No,
            };
            target.availability = Availability::Unavailable;
            targets.push(target);
        }
        targets.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        self.state.targets = targets;
        self.suspended
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id, _| {
                self.state
                    .targets
                    .iter()
                    .any(|t| &t.id == id && t.subscribed == Knowledge::Yes)
            });
        self.state.hid_suspended = self.state.selected_target().is_some_and(|target| {
            self.suspended
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&target.id)
                .copied()
                .unwrap_or(false)
        });
        let report_active = hid::ReportKind::ALL.map(|kind| {
            let index = match kind {
                hid::ReportKind::Consumer => 0,
            };
            self.selected_clients[index].as_ref().is_some_and(|client| {
                client
                    .Session()
                    .and_then(|session| session.SessionStatus())
                    .is_ok_and(|status| status == GattSessionStatus::Active)
            })
        });
        for (index, active) in report_active.into_iter().enumerate() {
            if !active {
                self.synced_reports[index] = false;
            }
        }
        let available = radio_on
            && !self.state.hid_suspended
            && selected_session_active
            && self.selected_clients[0].is_some();
        if !available {
            self.synced = false;
            self.synced_reports = [false; 1];
        }
        self.state.ready = available
            && self.synced_reports[0]
            && self.state.generation == self.revision.load(Ordering::Acquire);
        self.state.activity = TransportActivity::Idle;
        self.publish();
        if available
            && (if self.startup_restore.borrow().pending() {
                self.startup_restore.borrow().due(Instant::now())
            } else {
                self.maintenance.due(Instant::now()) && Instant::now() >= self.retry_at
            })
        {
            if self.startup_restore.borrow().pending() {
                self.startup_restore.borrow_mut().started();
            }
            let revision = self.revision.load(Ordering::Acquire);
            let recovering = !self.synced || self.maintenance.failures > 0;
            let mut failed = None;
            for (index, kind) in hid::ReportKind::ALL.into_iter().enumerate() {
                if !report_active[index] {
                    continue;
                }
                let Some(report_client) = self.selected_clients[index].clone() else {
                    self.synced_reports[index] = false;
                    continue;
                };
                let bytes = self.maintenance_report(kind);
                match self.notify(kind, &report_client, &bytes) {
                    Ok(()) => {
                        self.synced_reports[index] =
                            revision == self.revision.load(Ordering::Acquire);
                    }
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                }
            }
            if let Some(error) = failed {
                self.fail(&error);
            } else if self.synced_reports[0] && revision == self.revision.load(Ordering::Acquire) {
                self.synced = true;
                self.maintenance.success(Instant::now());
                self.state.last_error = None;
                self.state.device_error = None;
                if recovering {
                    tracing::info!("Media HID report synchronization completed");
                }
            } else {
                self.synced = false;
            }
        }
        self.state.ready = available
            && self.synced_reports[0]
            && self.state.generation == self.revision.load(Ordering::Acquire);
        if available && let Err(error) = self.process_due_pulses() {
            self.fail(&error);
        }
        self.publish();
        Ok(())
    }
    fn notify(
        &mut self,
        kind: hid::ReportKind,
        client: &GattSubscribedClient,
        bytes: &[u8],
    ) -> Result<(), BackendError> {
        if bytes.len() != kind.payload_len() {
            return Err(BackendError::Unavailable(format!(
                "{} report has {} bytes; expected {}",
                kind_name(kind),
                bytes.len(),
                kind.payload_len()
            )));
        }
        self.state.activity = if self.synced {
            TransportActivity::Sending
        } else {
            TransportActivity::Synchronizing
        };
        if !self.synced {
            self.publish();
        }
        *self
            .report(kind)
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = bytes.to_vec();
        let characteristic = self.report(kind).characteristic.clone();
        let pending = api(
            "NotifyValueForSubscribedClientAsync",
            characteristic
                .NotifyValueForSubscribedClientAsync(&api("DataWriter", buffer(bytes))?, client),
        )?;
        let (finished, completion) = mpsc::sync_channel(1);
        api(
            "Notification.Completed",
            pending.SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(
                move |_, _| {
                    let _ = finished.try_send(());
                    Ok(())
                },
            )),
        )?;
        // Completion wakes this serial sender immediately. On timeout, revoke
        // local capture first, but never overlap an uncertain native operation.
        let timed_out = completion.recv_timeout(Duration::from_millis(250)).is_err();
        if timed_out {
            self.synced = false;
            self.state.ready = false;
            self.revision.fetch_add(1, Ordering::AcqRel);
            self.state.last_error =
                Some("HID notification stalled; input returned to this computer".into());
            self.state.device_error = Some(DeviceError::ConnectionFailed);
            self.publish();
            completion.recv().map_err(|_| {
                BackendError::Unavailable("Notification completion disconnected".into())
            })?;
        }
        let result = api("Notification.GetResults", pending.GetResults())?;
        let status = api("Notification.Status result", result.Status())?;
        self.state.activity = TransportActivity::Idle;
        if timed_out {
            return Err(BackendError::Stale);
        }
        if status != GattCommunicationStatus::Success {
            return Err(BackendError::Unavailable(format!(
                "HID notification: {status:?}"
            )));
        }
        Ok(())
    }
}

fn kind_name(kind: hid::ReportKind) -> &'static str {
    match kind {
        hid::ReportKind::Consumer => "Consumer",
    }
}
impl Server {
    fn release_maintenance(&mut self) {
        if let Some(session) = self.maintained_session.take()
            && let Err(e) = session.SetMaintainConnection(false)
        {
            tracing::warn!("Release native GATT connection maintenance: {e}");
        }
    }
    fn enable_connection_maintenance(&mut self, client: &GattSubscribedClient) {
        let Ok(session) = client.Session() else {
            return;
        };
        match session.CanMaintainConnection() {
            Ok(true) => match session.MaintainConnection() {
                Ok(false) => match session.SetMaintainConnection(true) {
                    Ok(()) => {
                        self.maintained_session = Some(session);
                        tracing::info!("Native GATT connection maintenance enabled");
                    }
                    Err(e) => tracing::warn!("Native GATT connection maintenance failed: {e}"),
                },
                Ok(true) => tracing::info!("Native GATT connection maintenance already enabled"),
                Err(e) => tracing::warn!("Read native connection maintenance: {e}"),
            },
            Ok(false) => {
                tracing::info!("Native GATT connection maintenance unsupported for this session")
            }
            Err(e) => tracing::warn!("Inspect native connection maintenance: {e}"),
        }
    }
    fn try_startup_restore(&mut self) {
        let selected = self.startup_restore.borrow().candidate(
            &self.state,
            &self
                .suspended
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Instant::now(),
        );
        let Some(selected) = selected else {
            return;
        };
        if !self
            .manager
            .borrow_mut()
            .connect(&self.state, selected.clone(), Instant::now())
        {
            return;
        }
        self.startup_restore.borrow_mut().started();
        if let Err(error) = self.select(Some(selected)) {
            self.state.selected = None;
            self.manager.borrow_mut().disconnect();
            self.fail(&error);
        }
    }
    fn select(&mut self, target: Option<String>) -> Result<(), BackendError> {
        self.state.selected = target;
        self.state.ready = false;
        self.release_maintenance();
        self.session_active = false;
        self.metadata.clear();
        if let Some((session, token)) = self.session.take() {
            api(
                "RemoveSessionStatusChanged",
                session.RemoveSessionStatusChanged(token),
            )?;
        }
        self.selected_clients = std::array::from_fn(|_| None);
        self.consumer = hid::ConsumerState::default();
        self.pending_pulses.clear();
        self.reset_report_state();
        self.maintenance = maintenance::Maintenance::new(Instant::now());
        self.synced = false;
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.publish();
        Ok(())
    }
    fn send(&mut self, command: QueuedCommand) -> Result<(), BackendError> {
        self.refresh()?;
        if !command.valid(
            self.state.selected.as_deref(),
            self.revision.load(Ordering::Acquire),
            self.state.ready,
            Instant::now(),
            self.generation_started,
        ) {
            tracing::warn!("Command dropped: expired, disconnected, changed target or not ready");
            return Err(BackendError::Stale);
        }
        let target = self.selected_clients[0]
            .clone()
            .ok_or_else(|| BackendError::Unavailable("No selected subscriber".into()))?;
        let owner = command.action.usage() as u64;
        let report = match command.phase {
            // Seeking is only ever produced as the long press of a merged
            // shortcut, so it is the one press the receiver must see held.
            // Every other press is a self-contained tap.
            CommandPhase::Press
                if matches!(
                    command.action,
                    taprelay_core::command::MediaCommand::Rewind
                        | taprelay_core::command::MediaCommand::FastForward
                ) =>
            {
                self.consumer.press(owner, command.action);
                self.consumer.report()
            }
            CommandPhase::Press => {
                self.next_pulse_owner = self.next_pulse_owner.wrapping_add(1).max(1);
                let pulse_owner = self.next_pulse_owner;
                self.consumer.press(pulse_owner, command.action);
                self.pending_pulses.push((
                    Instant::now() + Duration::from_millis(40),
                    pulse_owner,
                    command.action,
                ));
                self.consumer.report()
            }
            CommandPhase::Release => {
                self.consumer.release(owner, command.action);
                self.consumer.report()
            }
        };
        let result = self.notify(hid::ReportKind::Consumer, &target, &report);
        if result.is_ok() {
            self.maintenance.success(Instant::now());
        }
        result
    }

    fn process_due_pulses(&mut self) -> Result<(), BackendError> {
        let now = Instant::now();
        let mut index = 0;
        while index < self.pending_pulses.len() {
            if self.pending_pulses[index].0 > now {
                index += 1;
                continue;
            }
            let (_, owner, action) = self.pending_pulses.swap_remove(index);
            self.consumer.release(owner, action);
            if let Some(target) = self.selected_clients[0].clone() {
                let report = self.consumer.report();
                self.notify(hid::ReportKind::Consumer, &target, &report)?;
            }
        }
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.release_maintenance();
        if let Some((s, t)) = self.session.take()
            && let Err(e) = s.RemoveSessionStatusChanged(t)
        {
            tracing::warn!("Remove GATT callback: {e}");
        }
        if let Some(t) = self.radio_token.take()
            && let Err(e) = self.radio.RemoveStateChanged(t)
        {
            tracing::warn!("Remove GATT callback: {e}");
        }
        for report in &self.reports {
            if let Err(e) = report.characteristic.RemoveReadRequested(report.read_token) {
                tracing::warn!("Remove GATT callback: {e}");
            }
            if let Err(e) = report
                .characteristic
                .RemoveSubscribedClientsChanged(report.subscription_token)
            {
                tracing::warn!("Remove GATT callback: {e}");
            }
        }
        if let (Some(c), Some(t)) = (&self.protocol, self.protocol_token)
            && let Err(e) = c.RemoveWriteRequested(t)
        {
            tracing::warn!("Remove GATT callback: {e}");
        }
        if let (Some(c), Some(t)) = (&self.control, self.write_token)
            && let Err(e) = c.RemoveWriteRequested(t)
        {
            tracing::warn!("Remove GATT callback: {e}");
        }
        for (p, token) in self.providers.iter().zip(&self.advertisement_tokens) {
            if let Err(e) = p.RemoveAdvertisementStatusChanged(*token) {
                tracing::warn!("Remove GATT callback: {e}");
            }
        }
        for p in &self.providers {
            if let Err(e) = p.StopAdvertising() {
                tracing::error!("StopAdvertising: {e}");
            }
        }
        tracing::info!("GATT cleanup attempted; individual failures are reported above");
    }
}
fn advertise(p: &GattServiceProvider) -> windows::core::Result<()> {
    let a = GattServiceProviderAdvertisingParameters::new()?;
    a.SetIsConnectable(true)?;
    a.SetIsDiscoverable(true)?;
    p.StartAdvertisingWithParameters(&a)
}
fn characteristic(
    p: &GattServiceProvider,
    id: u16,
    properties: GattCharacteristicProperties,
    value: Option<&[u8]>,
) -> windows::core::Result<GattLocalCharacteristic> {
    let params = GattLocalCharacteristicParameters::new()?;
    params.SetCharacteristicProperties(properties)?;
    params.SetReadProtectionLevel(GattProtectionLevel::EncryptionRequired)?;
    params.SetWriteProtectionLevel(GattProtectionLevel::EncryptionRequired)?;
    if let Some(v) = value {
        params.SetStaticValue(&buffer(v)?)?;
    }
    let r = p
        .Service()?
        .CreateCharacteristicAsync(uuid(id), &params)?
        .join()?;
    if r.Error()? != BluetoothError::Success {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80004005u32 as i32),
            format!("Characteristic {id:04x}: {:?}", r.Error()?),
        ));
    }
    r.Characteristic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_restore_requires_same_active_unsuspended_subscriber() {
        let remembered = Target {
            id: "old-endpoint".into(),
            name: "Same name".into(),
            identity: vec!["container:remembered".into()],
            ..Default::default()
        };
        let unrelated = Target {
            id: "unrelated".into(),
            name: "Same name".into(),
            link: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            ..Default::default()
        };
        let subscriber = Target {
            id: "live-endpoint".into(),
            identity: remembered.identity.clone(),
            link: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            ..Default::default()
        };
        let mut suspended = std::collections::BTreeMap::new();
        assert_eq!(
            startup_restore_candidate(
                &remembered,
                &[unrelated.clone(), subscriber.clone()],
                &suspended
            )
            .as_deref(),
            Some("live-endpoint")
        );
        suspended.insert(subscriber.id.clone(), true);
        assert!(
            startup_restore_candidate(&remembered, std::slice::from_ref(&subscriber), &suspended)
                .is_none()
        );
        suspended.clear();
        let inactive = Target {
            link: Knowledge::No,
            ..subscriber
        };
        assert!(
            startup_restore_candidate(&remembered, &[unrelated, inactive], &suspended).is_none()
        );
    }

    #[test]
    fn tap_publications_keep_ready_connection_consistent() {
        use taprelay_core::devices::Connection;
        let target = Target {
            id: "ipad".into(),
            pairing: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            ..Default::default()
        };
        let mut state = Snapshot {
            adapter_state: AdapterState::Available,
            selected: Some(target.id.clone()),
            targets: vec![target],
            ..Default::default()
        };
        let (updates, received) = watch::channel(Snapshot::default());
        let mut manager = Coordinator::default();
        manager.connect(&state, "ipad".into(), Instant::now());
        state.ready = true;
        // refresh() rebuilds native targets before notify() publishes Sending.
        for activity in [
            TransportActivity::Idle,
            TransportActivity::Sending,
            TransportActivity::Idle,
        ] {
            state.activity = activity;
            state.targets[0].connection = Connection::Disconnected;
            publish_snapshot(&updates, &mut state, &mut manager);
            assert!(received.borrow().ready);
            assert_eq!(
                received.borrow().selected_target().unwrap().connection,
                Connection::Connected,
                "ready Tap publication must not say paired/disconnected"
            );
        }
        state.ready = false;
        state.device_error = Some(DeviceError::ConnectionFailed);
        publish_snapshot(&updates, &mut state, &mut manager);
        assert!(!received.borrow().ready);
        assert_eq!(
            received.borrow().selected_target().unwrap().connection,
            Connection::Failed
        );
        state.adapter_state = AdapterState::Disabled;
        publish_snapshot(&updates, &mut state, &mut manager);
        assert_eq!(
            received.borrow().selected_target().unwrap().connection,
            Connection::Disconnected
        );
    }

    #[test]
    fn discovery_link_cannot_make_inactive_hid_session_sendable() {
        let discovery = Target {
            id: "discovery".into(),
            identity: vec!["same-device".into()],
            link: Knowledge::Yes,
            ..Default::default()
        };
        let mut subscriber = Target {
            id: "session".into(),
            identity: discovery.identity.clone(),
            link: Knowledge::No,
            subscribed: Knowledge::Yes,
            ..Default::default()
        };
        let mut merged = vec![discovery];
        taprelay_core::state::upsert_target(&mut merged, subscriber.clone());
        let index = selected_subscriber_index("discovery", &merged, &[subscriber.clone()]);
        assert_eq!(index, Some(0));
        assert_eq!(merged[0].link, Knowledge::Yes);
        assert!(!active_subscriber(index.map(|_| &subscriber)));
        // Repeated inactive polls must remain blocked even without an old client.
        assert!(!active_subscriber(Some(&subscriber)));
        assert!(!active_subscriber(None));
        subscriber.link = Knowledge::Yes;
        assert!(active_subscriber(Some(&subscriber)));
    }

    #[test]
    fn selected_subscriber_can_be_reached_through_an_endpoint_alias() {
        let selected = Target {
            id: "aep-endpoint".into(),
            aliases: vec!["gatt-session".into()],
            ..Default::default()
        };
        let subscriber = Target {
            id: "gatt-session".into(),
            ..Default::default()
        };
        assert_eq!(
            selected_subscriber_index(
                "aep-endpoint",
                std::slice::from_ref(&selected),
                std::slice::from_ref(&subscriber),
            ),
            Some(0)
        );
    }

    #[test]
    fn report_host_profile_avoids_restricted_identity_service_and_exposes_protocol_mode() {
        assert!(!ADVERTISED_SERVICES.contains(&0x180a));
        assert!(HID_CHARACTERISTICS.contains(&PROTOCOL_MODE_CHARACTERISTIC));
        assert_eq!(PROTOCOL_MODE, [1]);
    }
}
