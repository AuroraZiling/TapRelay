mod discovery;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use taprelay_core::devices::{
    AdapterState, Availability, Coordinator, DeviceError, PairingHandoff,
};
use taprelay_core::{
    command::QueuedCommand,
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
    Discover(bool),
    Pair(String),
}
pub struct BleHandle {
    pub commands: mpsc::SyncSender<Request>,
    pub state: watch::Receiver<Snapshot>,
    stop: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl BleHandle {
    pub fn is_finished(&self) -> bool {
        self.state.has_changed().is_err() || self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    pub fn invalidate_commands(&self) -> u64 {
        // UI invalidation and native subscription/radio/session transitions
        // share one monotonic generation; queued input may not cross it.
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub fn start() -> Result<Self, BackendError> {
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
                        let _apartment = super::Apartment::new()?;
                        let mut discovery = discovery::Discovery::new();
                        let mut manager = Coordinator::default();
                        let mut discovery_active = false;
                        let mut service_retry_at = Instant::now();
                        let mut server = None;
                        let mut restart = true;
                        let mut refresh_at = Instant::now();
                        while !stopping.load(Ordering::Acquire) {
                            let previous_adapter = discovery.adapter;
                            let previous_adapter_id = discovery.adapter_id.clone();
                            discovery.set_active(discovery_active || manager.pairing_pending());
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
                                manager.clear_selection();
                                tx.send_replace(Snapshot {
                                    generation: worker_revision.load(Ordering::Acquire),
                                    activity: TransportActivity::CheckingEnvironment,
                                    ..Default::default()
                                });
                                match Server::create(tx.clone(), worker_revision.clone()) {
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
                                    if let Err(e) = s.refresh() {
                                        s.fail(&e);
                                    }
                                    if s.state.selected.is_none() {
                                        manager.clear_selection();
                                    }
                                    manager.reconcile(&mut s.state, Instant::now());
                                    s.publish();
                                } else {
                                    tx.send_modify(|state| {
                                        state.targets = candidates;
                                        state.adapter_state = discovery.adapter;
                                        state.adapter =
                                            discovery.adapter == AdapterState::Available;
                                        state.discovery = discovery.state;
                                        state.selected = None;
                                        manager.reconcile(state, Instant::now());
                                    });
                                }
                                refresh_at = Instant::now() + Duration::from_millis(100);
                            }
                            match rx
                                .recv_timeout(refresh_at.saturating_duration_since(Instant::now()))
                            {
                                Ok(Request::Restart) => {
                                    restart = true;
                                }
                                Ok(Request::Refresh) => {
                                    discovery.restart();
                                    refresh_at = Instant::now();
                                }
                                Ok(Request::Discover(active)) => {
                                    discovery_active = active;
                                }
                                Ok(Request::Pair(id)) => {
                                    let candidate = tx
                                        .borrow()
                                        .targets
                                        .iter()
                                        .find(|t| t.matches_id(&id))
                                        .cloned();
                                    if let Some(target) = candidate {
                                        if !manager.pair(target, Instant::now()) {
                                            continue;
                                        }
                                        if let Err(e) =
                                            super::desktop::open("ms-settings:bluetooth")
                                        {
                                            tracing::error!("Open pairing settings: {e}");
                                            manager.cancel_pairing();
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
                                    if let Some(id) = &id {
                                        if !manager.connect(id.clone(), Instant::now()) {
                                            continue;
                                        }
                                    } else {
                                        manager.disconnect();
                                    }
                                    if let Some(s) = &mut server {
                                        s.state.device_error = None;
                                        s.state.pairing_handoff = PairingHandoff::Idle;
                                        if let Err(e) = s.select(id) {
                                            s.fail(&e);
                                        }
                                    } else {
                                        manager.clear_selection();
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

struct Server {
    providers: Vec<GattServiceProvider>,
    input: Option<GattLocalCharacteristic>,
    control: Option<GattLocalCharacteristic>,
    protocol: Option<GattLocalCharacteristic>,
    read_token: Option<i64>,
    write_token: Option<i64>,
    protocol_token: Option<i64>,
    subscription_token: Option<i64>,
    session: Option<(GattSession, i64)>,
    radio: Radio,
    radio_token: Option<i64>,
    advertisement_tokens: Vec<i64>,
    revision: Arc<AtomicU64>,
    current: Arc<AtomicU8>,
    suspended: Arc<Mutex<std::collections::BTreeMap<String, bool>>>,
    selected_client: Option<GattSubscribedClient>,
    state: Snapshot,
    updates: watch::Sender<Snapshot>,
    synced: bool,
    generation_started: Instant,
    retry_at: Instant,
    connected: Vec<Target>,
    metadata: MetadataCache,
}
impl Server {
    fn create(
        updates: watch::Sender<Snapshot>,
        revision: Arc<AtomicU64>,
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
            providers: vec![],
            input: None,
            control: None,
            protocol: None,
            read_token: None,
            write_token: None,
            protocol_token: None,
            subscription_token: None,
            session: None,
            radio,
            radio_token: None,
            advertisement_tokens: vec![],
            revision,
            current: Arc::new(AtomicU8::new(0)),
            suspended: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            selected_client: None,
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
            generation_started: Instant::now(),
            retry_at: Instant::now(),
            connected: vec![],
            metadata: MetadataCache::default(),
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
                let input = characteristic(
                    &p,
                    REPORT_CHARACTERISTIC,
                    GattCharacteristicProperties::Read | GattCharacteristicProperties::Notify,
                    None,
                )?;
                self.input = Some(input.clone());
                let params = GattLocalDescriptorParameters::new()?;
                params.SetStaticValue(&buffer(&hid::REPORT_REFERENCE)?)?;
                params.SetReadProtectionLevel(GattProtectionLevel::EncryptionRequired)?;
                let reference = input.CreateDescriptorAsync(uuid(0x2908), &params)?.join()?;
                if reference.Error()? != BluetoothError::Success {
                    return Err(windows::core::Error::new(
                        windows::core::HRESULT(0x80004005u32 as i32),
                        format!("Report Reference: {:?}", reference.Error()?),
                    ));
                }
                let current = self.current.clone();
                self.read_token =
                    Some(input.ReadRequested(&TypedEventHandler::<
                        GattLocalCharacteristic,
                        GattReadRequestedEventArgs,
                    >::new(move |_, args| {
                        callback(|| {
                            let args = args.as_ref().ok_or_else(windows::core::Error::empty)?;
                            let deferral = args.GetDeferral()?;
                            let result = (|| {
                                let r = args.GetRequestAsync()?.join()?;
                                let offset = r.Offset()?;
                                tracing::debug!("GATT input read offset={offset}");
                                let value = [current.load(Ordering::Acquire)];
                                if offset > 1 {
                                    r.RespondWithProtocolError(7)
                                } else {
                                    r.RespondWithValue(&buffer(if offset == 0 {
                                        &value[..]
                                    } else {
                                        &[]
                                    })?)
                                }
                            })();
                            deferral.Complete()?;
                            result
                        })
                    }))?);
                let rev = self.revision.clone();
                self.subscription_token = Some(input.SubscribedClientsChanged(
                    &TypedEventHandler::new(move |_, _| {
                        callback(|| {
                            rev.fetch_add(1, Ordering::AcqRel);
                            tracing::info!("HID subscription changed");
                            Ok(())
                        })
                    }),
                )?);
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
    fn publish(&self) {
        if *self.updates.borrow() != self.state {
            self.updates.send_replace(self.state.clone());
        }
    }
    fn fail(&mut self, e: &BackendError) {
        self.synced = false;
        self.state.ready = false;
        self.state.last_error = Some(e.to_string());
        self.state.device_error = Some(DeviceError::ConnectionFailed);
        self.retry_at = Instant::now() + Duration::from_secs(3);
        tracing::error!("{e}");
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
            .input
            .as_ref()
            .expect("constructed input")
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
        let selected_link_lost = self.selected_client.is_some()
            && selected_index.is_none_or(|index| {
                subscriber_targets
                    .get(index)
                    .is_none_or(|target| target.link != Knowledge::Yes)
            });
        let selected_client_replaced = self.selected_client.is_some()
            && selected_client.is_some()
            && self.selected_client != selected_client;
        let selected_client_lost = self.selected_client.is_some()
            && (selected_client.is_none() || selected_link_lost || selected_client_replaced);
        let next_selected_client = if selected_client_lost {
            None
        } else {
            selected_client
        };
        let client_changed = self.selected_client != next_selected_client;
        if client_changed {
            self.synced = false;
            self.state.generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
            self.generation_started = Instant::now();
            if let Some((s, token)) = self.session.take() {
                s.RemoveSessionStatusChanged(token)?;
            }
            if let Some(client) = &next_selected_client {
                let session = client.Session()?;
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
        self.selected_client = next_selected_client;
        if selected_client_lost {
            tracing::info!(
                "Selected HID subscriber disappeared or changed; clearing session selection"
            );
            self.state.selected = None;
            self.state.target_status = None;
            self.state.ready = false;
            self.state.device_error = None;
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
        self.state.target_status = match &self.state.selected {
            Some(id) => match self.state.targets.iter().find(|t| t.matches_id(id)) {
                Some(target) => Some(target.clone()),
                None => {
                    let mut target = self
                        .state
                        .target_status
                        .take()
                        .filter(|t| t.matches_id(id))
                        .unwrap_or_else(|| Target {
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
                    Some(target)
                }
            },
            None => None,
        };
        if let Some(target) = &mut self.state.target_status
            && !self.state.targets.iter().any(|t| t.same_device(target))
        {
            target.availability = Availability::Unavailable;
            self.state.targets.push(target.clone());
        }
        self.state.hid_suspended = self.state.target_status.as_ref().is_some_and(|target| {
            self.suspended
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&target.id)
                .copied()
                .unwrap_or(false)
        });
        let available = radio_on
            && !self.state.hid_suspended
            && self.state.targets.iter().any(|t| {
                self.state
                    .selected
                    .as_ref()
                    .is_some_and(|id| t.matches_id(id))
                    && t.link == Knowledge::Yes
                    && t.subscribed == Knowledge::Yes
            });
        if !available {
            self.synced = false;
        }
        self.state.ready = available && self.synced;
        self.state.activity = TransportActivity::Idle;
        self.publish();
        if available && !self.synced && Instant::now() >= self.retry_at {
            let client = self.selected_client.clone().expect("available client");
            let revision = self.revision.load(Ordering::Acquire);
            match self.notify(&client, &hid::NEUTRAL) {
                Ok(()) => {
                    self.synced = revision == self.revision.load(Ordering::Acquire);
                    self.state.ready = self.synced;
                    self.state.last_error = None;
                    tracing::info!(
                        "Neutral notification completed; transport ready={}",
                        self.synced
                    );
                }
                Err(e) => self.fail(&e),
            }
        }
        self.publish();
        Ok(())
    }
    fn notify(&mut self, client: &GattSubscribedClient, bytes: &[u8]) -> Result<(), BackendError> {
        self.state.activity = if self.synced {
            TransportActivity::Sending
        } else {
            TransportActivity::Synchronizing
        };
        self.publish();
        self.current.store(bytes[0], Ordering::Release);
        let pending = api(
            "NotifyValueForSubscribedClientAsync",
            self.input
                .as_ref()
                .expect("input")
                .NotifyValueForSubscribedClientAsync(&api("DataWriter", buffer(bytes))?, client),
        )?;
        let start = Instant::now();
        let mut reported = false;
        // Never overlap an uncertain native operation with a later report.
        while api("Notification.Status", pending.Status())? == windows_future::AsyncStatus::Started
        {
            if !reported && start.elapsed() > Duration::from_secs(5) {
                reported = true;
                self.synced = false;
                self.state.ready = false;
                self.state.last_error =
                    Some("Notification pending >5s; waiting for Windows before release".into());
                self.state.device_error = Some(DeviceError::ConnectionFailed);
                self.publish();
                tracing::error!("Notification pending >5s; queue expires, no press retry");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let result = api("Notification.GetResults", pending.GetResults())?;
        let status = api("Notification.Status result", result.Status())?;
        tracing::info!(
            report = bytes[0],
            elapsed_ms = start.elapsed().as_millis(),
            "HID notification completed: {status:?}"
        );
        self.state.activity = TransportActivity::Idle;
        if status != GattCommunicationStatus::Success {
            return Err(BackendError::Unavailable(format!(
                "HID notification: {status:?}"
            )));
        }
        Ok(())
    }
}
impl Server {
    fn select(&mut self, target: Option<String>) -> Result<(), BackendError> {
        self.metadata.clear();
        if let Some((session, token)) = self.session.take() {
            api(
                "RemoveSessionStatusChanged",
                session.RemoveSessionStatusChanged(token),
            )?;
        }
        self.selected_client = None;
        self.state.selected = target;
        self.state.target_status = None;
        self.synced = false;
        self.state.ready = false;
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
        let target = self
            .selected_client
            .clone()
            .ok_or_else(|| BackendError::Unavailable("No selected subscriber".into()))?;
        let result = hid::click(
            command.action,
            |bytes| self.notify(&target, bytes),
            // Give the receiver a distinct press before neutral. Do not shorten
            // without receiver compatibility measurements.
            || thread::sleep(Duration::from_millis(40)),
        );
        self.current.store(0, Ordering::Release);
        result
    }
}
impl Drop for Server {
    fn drop(&mut self) {
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
        if let Some(c) = &self.input {
            if let Some(t) = self.read_token
                && let Err(e) = c.RemoveReadRequested(t)
            {
                tracing::warn!("Remove GATT callback: {e}");
            }
            if let Some(t) = self.subscription_token
                && let Err(e) = c.RemoveSubscribedClientsChanged(t)
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
