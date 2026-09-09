use serde::Serialize;
use taprelay_core::ports::BackendError;
use windows::Devices::{Bluetooth::BluetoothAdapter, Radios::RadioState};
#[derive(Debug, Serialize)]
pub struct Diagnosis {
    pub platform: &'static str,
    pub adapter_present: bool,
    pub adapter_id: Option<String>,
    pub radio_name: Option<String>,
    pub radio_on: Option<bool>,
    pub low_energy: Option<bool>,
    pub peripheral_role: Option<bool>,
    pub issue: Option<String>,
}
pub fn doctor() -> Result<Diagnosis, BackendError> {
    let _apartment = super::Apartment::new()?;
    inspect().map_err(|e| super::native_error("BluetoothAdapter diagnostics", e))
}
fn inspect() -> windows::core::Result<Diagnosis> {
    let mut d = Diagnosis {
        platform: "Windows",
        adapter_present: false,
        adapter_id: None,
        radio_name: None,
        radio_on: None,
        low_energy: None,
        peripheral_role: None,
        issue: None,
    };
    let adapter = match BluetoothAdapter::GetDefaultAsync()?.join() {
        Ok(a) => a,
        Err(e) if e.code() == windows::core::HRESULT(0x80004003u32 as i32) => {
            d.issue = Some("No Bluetooth adapter".into());
            return Ok(d);
        }
        Err(e) => return Err(e),
    };
    d.adapter_present = true;
    d.adapter_id = Some(adapter.DeviceId()?.to_string());
    d.low_energy = Some(adapter.IsLowEnergySupported()?);
    d.peripheral_role = Some(adapter.IsPeripheralRoleSupported()?);
    let radio = adapter.GetRadioAsync()?.join()?;
    d.radio_name = Some(radio.Name()?.to_string());
    d.radio_on = Some(radio.State()? == RadioState::On);
    d.issue = if d.low_energy == Some(false) {
        Some("BLE unsupported".into())
    } else if d.peripheral_role == Some(false) {
        Some("Peripheral Role unsupported".into())
    } else if d.radio_on != Some(true) {
        Some("Bluetooth off or unavailable; enable it in Windows Settings".into())
    } else {
        None
    };
    Ok(d)
}
