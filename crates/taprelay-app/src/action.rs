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
    Show => "show", Close => "close", Quit => "quit",
    // Payload: the page number declared in ui/navigation.slint.
    Navigate => "navigate",
    Listen => "listen", Test => "test", Refresh => "refresh", Retry => "retry",
    Resume => "resume", TrayReset => "tray-reset", BluetoothSettings => "bluetooth-settings",
    DataFolder => "data-folder", LogsFolder => "logs-folder",
    Repository => "repository", License => "license", ThirdPartyNotices => "third-party-notices",
    // Payload: the device id the row was built from.
    Device => "device",
    PairDevice => "pair-device", DisconnectDevice => "disconnect-device",
    Wizard => "wizard", WizardNext => "wizard-next", WizardBack => "wizard-back",
    WizardFinish => "wizard-finish",
    // Payload: a `Theme` name ("system", "light", "dark").
    Theme => "theme",
    // Payload: a `Language` name ("system", "chinese", "english").
    Language => "language",
    // One command per option: the name says which setting, and the payload is
    // its new state, so the UI never has to number them.
    SetAutostart => "set-autostart",
    SetStartHidden => "set-start-hidden",
    SetAutoListen => "set-auto-listen",
    SetPassthroughReverseScroll => "set-passthrough-reverse-scroll",
    SetCloseToTray => "set-close-to-tray",
    SetAlwaysAdmin => "set-always-admin",
    SetNotifications => "set-notifications",
    SetConnectionWaitWarning => "set-connection-wait-warning",
    Elevate => "elevate",
}

/// Payload of the `set-*` actions: a switch sends its new state as "0" or "1".
pub fn toggle(value: &str) -> anyhow::Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        other => anyhow::bail!("Invalid toggle value: {other}"),
    }
}

/// Payload of [`Action::Navigate`]: the page number declared in
/// `ui/navigation.slint`, as text because the action channel only carries text.
pub fn page(value: &str) -> anyhow::Result<i32> {
    value
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid page number: {value}"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
