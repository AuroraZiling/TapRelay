macro_rules! actions {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Action { $($variant),+ }
        impl TryFrom<&str> for Action {
            type Error = String;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value { $($name => Ok(Self::$variant),)+ _ => Err(format!("Unknown UI action: {value}")) }
            }
        }
    }
}
actions! {
    Show => "show", Close => "close", Quit => "quit", Navigate => "navigate",
    Listen => "listen", Test => "test", Refresh => "refresh", Retry => "retry",
    Resume => "resume", TrayReset => "tray-reset", BluetoothSettings => "bluetooth-settings",
    DataFolder => "data-folder", LogsFolder => "logs-folder", Device => "device",
    PairDevice => "pair-device", DisconnectDevice => "disconnect-device",
    Wizard => "wizard", WizardNext => "wizard-next", WizardBack => "wizard-back",
    WizardFinish => "wizard-finish", Theme => "theme", Language => "language",
    Setting => "setting", Elevate => "elevate",
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CaptureTarget {
    pub id: taprelay_core::function::FunctionId,
    pub slot: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BindingCommand {
    BeginCapture(CaptureTarget),
    CancelCapture,
    DeleteShortcut(CaptureTarget),
    SetFunctionEnabled {
        id: taprelay_core::function::FunctionId,
        enabled: bool,
    },
}
