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
    DataFolder => "data-folder", LogsFolder => "logs-folder", Capture => "capture",
    CancelCapture => "cancel-capture", Delete => "delete", Device => "device",
    ToggleFunction => "toggle-function",
    PairDevice => "pair-device", DisconnectDevice => "disconnect-device",
    Wizard => "wizard", WizardNext => "wizard-next", WizardBack => "wizard-back",
    WizardFinish => "wizard-finish", Theme => "theme", Language => "language",
    Setting => "setting", Elevate => "elevate", Logs => "logs", ClearLogs => "clear-logs",
    CopyLogs => "copy-logs",
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    Function {
        id: taprelay_core::function::FunctionId,
        slot: usize,
    },
}
