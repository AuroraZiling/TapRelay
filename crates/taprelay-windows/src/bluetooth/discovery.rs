//! Association-endpoint observation; pairing/trust remains in Windows Settings.
use super::*;
use std::collections::BTreeMap;
use taprelay_core::devices::{AdapterState, Availability, DiscoveryState, Inventory};
use windows::Devices::Enumeration::{
    DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher,
};

enum Event {
    Added(DeviceInformation),
    Updated(DeviceInformationUpdate),
    Removed(String),
    Complete,
    Stopped,
}
struct Stream {
    watcher: DeviceWatcher,
    tokens: [Option<i64>; 5],
    events: mpsc::Receiver<Event>,
    devices: BTreeMap<String, DeviceInformation>,
    state: DiscoveryState,
}
impl Stream {
    fn start() -> windows::core::Result<Self> {
        // Protocol IDs identify Bluetooth association endpoints, not HID services.
        let protocol = "(System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" OR System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\")";
        let query = format!(
            "{protocol} AND System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True"
        );
        let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
            &HSTRING::from(query),
            &identity_properties(),
            DeviceInformationKind::AssociationEndpoint,
        )?;
        let (tx, events) = mpsc::channel();
        let mut stream = Self {
            watcher,
            tokens: [None; 5],
            events,
            devices: BTreeMap::new(),
            state: DiscoveryState::Scanning,
        };
        let sender = tx.clone();
        stream.tokens[0] = Some(stream.watcher.Added(&TypedEventHandler::<
            DeviceWatcher,
            DeviceInformation,
        >::new(move |_, info| {
            if let Some(info) = info.as_ref() {
                let _ = sender.send(Event::Added(info.clone()));
            }
            Ok(())
        }))?);
        let sender = tx.clone();
        stream.tokens[1] = Some(stream.watcher.Updated(&TypedEventHandler::<
            DeviceWatcher,
            DeviceInformationUpdate,
        >::new(move |_, info| {
            if let Some(info) = info.as_ref() {
                let _ = sender.send(Event::Updated(info.clone()));
            }
            Ok(())
        }))?);
        let sender = tx.clone();
        stream.tokens[2] = Some(stream.watcher.Removed(&TypedEventHandler::<
            DeviceWatcher,
            DeviceInformationUpdate,
        >::new(move |_, info| {
            if let Some(info) = info.as_ref() {
                let _ = sender.send(Event::Removed(info.Id()?.to_string()));
            }
            Ok(())
        }))?);
        let sender = tx.clone();
        stream.tokens[3] = Some(
            stream
                .watcher
                .EnumerationCompleted(&TypedEventHandler::new(move |_, _| {
                    let _ = sender.send(Event::Complete);
                    Ok(())
                }))?,
        );
        stream.tokens[4] = Some(
            stream
                .watcher
                .Stopped(&TypedEventHandler::new(move |_, _| {
                    let _ = tx.send(Event::Stopped);
                    Ok(())
                }))?,
        );
        stream.watcher.Start()?;
        Ok(stream)
    }
    fn drain(&mut self) -> windows::core::Result<Option<Vec<Target>>> {
        let mut changed = false;
        for event in self.events.try_iter() {
            changed = true;
            match event {
                Event::Added(info) => {
                    self.devices.insert(info.Id()?.to_string(), info);
                }
                Event::Updated(update) => {
                    if let Some(info) = self.devices.get(&update.Id()?.to_string()) {
                        info.Update(&update)?;
                    }
                }
                Event::Removed(id) => {
                    self.devices.remove(&id);
                }
                Event::Complete => self.state = DiscoveryState::ResultsAvailable,
                Event::Stopped => self.state = DiscoveryState::Failed,
            }
        }
        if !changed {
            return Ok(None);
        }
        self.devices
            .values()
            .map(|info| {
                Ok(Target {
                    id: info.Id()?.to_string(),
                    name: info.Name()?.to_string(),
                    kind: device_kind(info),
                    pairing: if info.Pairing()?.IsPaired()? {
                        Knowledge::Yes
                    } else {
                        Knowledge::No
                    },
                    link: bool_property(info, "System.Devices.Aep.IsConnected"),
                    availability: match bool_property(info, "System.Devices.Aep.IsPresent") {
                        Knowledge::Yes => Availability::Nearby,
                        Knowledge::No => Availability::Unavailable,
                        Knowledge::Unknown => Availability::Unknown,
                    },
                    identity: device_identity(info),
                    ..Default::default()
                })
            })
            .collect::<windows::core::Result<Vec<_>>>()
            .map(Some)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        if let Err(e) = self.watcher.Stop() {
            tracing::debug!("Stop device watcher: {e}");
        }
        for (i, token) in self.tokens.iter().enumerate() {
            if let Some(token) = token {
                let result = match i {
                    0 => self.watcher.RemoveAdded(*token),
                    1 => self.watcher.RemoveUpdated(*token),
                    2 => self.watcher.RemoveRemoved(*token),
                    3 => self.watcher.RemoveEnumerationCompleted(*token),
                    _ => self.watcher.RemoveStopped(*token),
                };
                if let Err(e) = result {
                    tracing::warn!("Remove device watcher handler: {e}");
                }
            }
        }
    }
}
fn bool_property(info: &DeviceInformation, key: &str) -> Knowledge {
    info.Properties()
        .and_then(|p| p.Lookup(&HSTRING::from(key)))
        .and_then(|v| v.cast::<IPropertyValue>())
        .and_then(|v| v.GetBoolean())
        .map(|v| if v { Knowledge::Yes } else { Knowledge::No })
        .unwrap_or_default()
}

fn device_kind(info: &DeviceInformation) -> taprelay_core::state::DeviceKind {
    let read = |key: &str| -> Option<u16> {
        info.Properties()
            .ok()?
            .Lookup(&HSTRING::from(key))
            .ok()?
            .cast::<IPropertyValue>()
            .ok()?
            .GetUInt16()
            .ok()
    };
    classify_device(
        read("System.Devices.Aep.Bluetooth.Cod.Major"),
        read("System.Devices.Aep.Bluetooth.Cod.Minor"),
        read("System.Devices.Aep.Bluetooth.Le.Appearance.Category"),
    )
}

fn classify_device(
    major: Option<u16>,
    minor: Option<u16>,
    appearance: Option<u16>,
) -> taprelay_core::state::DeviceKind {
    use taprelay_core::state::DeviceKind;
    use windows::Devices::Bluetooth::{
        BluetoothLEAppearanceCategories, BluetoothMajorClass, BluetoothMinorClass,
    };
    if major == Some(BluetoothMajorClass::Computer.0 as u16) {
        return if minor == Some(BluetoothMinorClass::ComputerTablet.0 as u16) {
            DeviceKind::Tablet
        } else {
            DeviceKind::Computer
        };
    }
    if major == Some(BluetoothMajorClass::Phone.0 as u16) {
        return DeviceKind::Phone;
    }
    // Missing properties must not match a failed static API lookup (None).
    if let Some(category) = appearance {
        if BluetoothLEAppearanceCategories::Phone().ok() == Some(category) {
            return DeviceKind::Phone;
        }
        if BluetoothLEAppearanceCategories::Computer().ok() == Some(category) {
            return DeviceKind::Computer;
        }
    }
    DeviceKind::Unknown
}

#[cfg(test)]
mod classification_tests {
    use super::*;
    use taprelay_core::state::DeviceKind;
    #[test]
    fn system_classification_handles_missing_and_unknown_properties() {
        assert_eq!(classify_device(None, None, None), DeviceKind::Unknown);
        assert_eq!(
            classify_device(Some(31), None, Some(65535)),
            DeviceKind::Unknown
        );
        assert_eq!(classify_device(Some(1), Some(7), None), DeviceKind::Tablet);
        assert_eq!(
            classify_device(Some(1), Some(3), None),
            DeviceKind::Computer
        );
        assert_eq!(classify_device(Some(2), None, None), DeviceKind::Phone);
        assert_eq!(
            classify_device(
                None,
                None,
                Some(
                    windows::Devices::Bluetooth::BluetoothLEAppearanceCategories::Phone().unwrap()
                )
            ),
            DeviceKind::Phone
        );
    }
}

pub(super) struct Discovery {
    known: Option<Stream>,
    pub inventory: Inventory,
    pub adapter: AdapterState,
    pub adapter_id: Option<String>,
    pub state: DiscoveryState,
    radio: Option<(Radio, i64)>,
    radio_events: mpsc::Receiver<()>,
    radio_sender: mpsc::Sender<()>,
    check_at: Instant,
    failed: bool,
}
impl Discovery {
    pub fn new() -> Self {
        let (radio_sender, radio_events) = mpsc::channel();
        Self {
            known: None,
            inventory: Inventory::default(),
            adapter: AdapterState::Unknown,
            adapter_id: None,
            state: DiscoveryState::Idle,
            radio: None,
            radio_events,
            radio_sender,
            check_at: Instant::now(),
            failed: false,
        }
    }
    pub fn restart(&mut self) {
        self.known.take();
        self.failed = false;
    }
    pub fn tick(&mut self) -> windows::core::Result<()> {
        let radio_changed = self.radio_events.try_iter().count() > 0;
        if radio_changed || Instant::now() >= self.check_at {
            self.check_at = Instant::now() + Duration::from_secs(5);
            match windows::Devices::Bluetooth::BluetoothAdapter::GetDefaultAsync()
                .and_then(|op| op.join())
            {
                Ok(adapter) => {
                    let id = adapter.DeviceId()?.to_string();
                    if self.adapter_id.as_ref() != Some(&id) || self.radio.is_none() {
                        self.release_radio();
                        self.restart();
                        self.inventory = Inventory::default();
                        let radio = adapter.GetRadioAsync()?.join()?;
                        let sender = self.radio_sender.clone();
                        let token = radio.StateChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = sender.send(());
                            Ok(())
                        }))?;
                        self.radio = Some((radio, token));
                        self.adapter_id = Some(id);
                    }
                }
                Err(e) => {
                    self.release_radio();
                    self.adapter_id = None;
                    self.adapter = AdapterState::Unavailable;
                    tracing::debug!("Adapter unavailable: {e}");
                }
            }
            if let Some((radio, _)) = &self.radio {
                self.adapter = match radio.State() {
                    Ok(RadioState::On) => AdapterState::Available,
                    Ok(_) => AdapterState::Disabled,
                    Err(e) => {
                        tracing::warn!("Read radio: {e}");
                        self.release_radio();
                        AdapterState::Unavailable
                    }
                };
            }
        }
        if self.adapter != AdapterState::Available {
            self.known.take();
            for target in &mut self.inventory.known {
                target.availability = Availability::Unavailable;
                target.link = Knowledge::No;
            }
            self.state = DiscoveryState::Idle;
            self.failed = false;
            return Ok(());
        }
        if self.failed {
            return Ok(());
        }
        if self.known.is_none() {
            self.known = Some(Stream::start()?);
        }
        if let Some(stream) = &mut self.known
            && let Some(rows) = stream.drain()?
        {
            self.inventory.known = rows;
        }
        self.state = self
            .known
            .as_ref()
            .map_or(DiscoveryState::Idle, |s| s.state);
        if self
            .known
            .as_ref()
            .is_some_and(|s| s.state == DiscoveryState::Failed)
        {
            self.state = DiscoveryState::Failed;
        }
        Ok(())
    }
    fn release_radio(&mut self) {
        if let Some((radio, token)) = self.radio.take()
            && let Err(e) = radio.RemoveStateChanged(token)
        {
            tracing::warn!("Remove adapter observer: {e}");
        }
    }
    pub fn fail(&mut self) {
        if self.radio.is_none() {
            self.adapter = AdapterState::Unavailable;
        }
        self.failed = true;
        self.state = DiscoveryState::Failed;
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some((radio, token)) = self.radio.take()
            && let Err(e) = radio.RemoveStateChanged(token)
        {
            tracing::warn!("Remove adapter observer: {e}");
        }
    }
}
