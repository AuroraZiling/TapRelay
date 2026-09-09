use crate::{
    AppWindow, BindingRow, CheckRow, Theme, ThemeMode,
    action::{Action, CaptureTarget},
    administrator,
    config::{self, Config, Language, SaveQueue, Theme as ThemeSetting},
    feedback::{TestStatus, WaitWarning},
    i18n::{self, keys},
    logging::{self, Logs},
    platform::desktop::{self, Desktop, DesktopEvent},
    runtime::Runtime,
};
use anyhow::{Context, Result, bail};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use taprelay_core::{
    binding::Binding,
    command::MediaCommand,
    state::{Target, TransportActivity},
};

pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let startup = parse_startup_options(&args)?;
    let handoff = startup.handoff;
    if let Some(pid) = handoff {
        administrator::wait_for_parent(pid)?;
    } else if desktop::activate_existing() {
        return Ok(());
    }
    let path = config::path()?;
    let loaded = Config::load(&path);
    let status = administrator::status()?;
    let needs_admin = should_request_admin(
        startup.privilege,
        loaded.as_ref().is_ok_and(|c| c.options.always_admin),
    );
    if handoff.is_none() && needs_admin && !status.elevated {
        match administrator::restart() {
            Ok(true) => return Ok(()),
            Ok(false) if startup.privilege == PrivilegeMode::Admin => {
                bail!("Administrator privileges are required when starting with --admin")
            }
            Ok(false) => {}
            Err(e) => desktop::show_error(&format!(
                "Administrator request failed; continuing normally.\n{e:#}"
            )),
        }
    }
    let _instance = match crate::platform::InstanceLock::acquire() {
        Ok(i) => i,
        Err(e) => {
            if desktop::activate_existing() {
                return Ok(());
            }
            return Err(e.into());
        }
    };
    let ui = AppWindow::new()?;
    let mut startup_error = loaded.as_ref().err().map(|e| format!("{e:#}"));
    if let Err(e) = config::check_writable(&path) {
        startup_error = Some(format!("{e:#}"));
    }
    let config = loaded.unwrap_or_default();
    let log_init = logging::init(&path);
    let (logs, _guard) = match log_init {
        Ok((l, g)) => (l, Some(g)),
        Err(e) => {
            startup_error = Some(format!("Cannot open logs: {e:#}"));
            (Logs::default(), None)
        }
    };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        elevated = status.elevated,
        account_admin = status.account_admin,
        "TapRelay started"
    );
    let chinese = match config.options.language {
        Language::Chinese => true,
        Language::English => false,
        Language::System => desktop::system_chinese(),
    };
    let desktop = Desktop::new(i18n::tray_labels(chinese))?;
    let size = &config.window;
    if let (Some(x), Some(y)) = (size.x, size.y)
        && desktop::visible_position(x, y)
    {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    }
    ui.window().set_maximized(size.maximized);
    let actions = Rc::new(RefCell::new(Vec::<(Action, i32, String)>::new()));
    let window_keys = Rc::new(RefCell::new(Vec::new()));
    let keys = window_keys.clone();
    let a = actions.clone();
    ui.on_action(move |name, index, value| {
        if name == "capture-key" {
            if !desktop::ui_input_is_injected()
                && let Some(key) = crate::capture_key::virtual_key(&value)
            {
                keys.borrow_mut().push(taprelay_core::input::InputEvent {
                    code: taprelay_core::input::InputCode::Key(key),
                    down: index != 0,
                    captured: Instant::now(),
                });
            }
            return;
        }
        match Action::try_from(name.as_str()) {
            Ok(action) => a.borrow_mut().push((action, index, value.to_string())),
            Err(e) => tracing::error!("{e}"),
        }
    });
    let a = actions.clone();
    ui.window().on_close_requested(move || {
        a.borrow_mut().push((Action::Close, 0, String::new()));
        slint::CloseRequestResponse::KeepWindowShown
    });
    let mut controller = Controller::new(config, path, logs, desktop, status, startup_error);
    ui.set_log_lines(controller.log_model.clone().into());
    controller.sync(&ui);
    if controller.fatal.is_none() {
        controller.runtime.start_bluetooth()?;
    }
    ui.show()?;
    // Initial winit attributes only set ICON_SMALL. Updating the image after
    // native window creation lets Slint set both title-bar and taskbar icons.
    let icon_window = ui.as_weak();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(ui) = icon_window.upgrade() {
            ui.set_app_icon(ui.get_app_icon_artwork());
        }
    });
    let controller = Rc::new(RefCell::new(controller));
    let weak = ui.as_weak();
    let c = controller.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let Ok(mut c) = c.try_borrow_mut() else {
                return;
            };
            for event in desktop::events() {
                let name = match event {
                    DesktopEvent::Show => Action::Show,
                    DesktopEvent::Toggle => Action::Listen,
                    DesktopEvent::Quit => Action::Quit,
                    DesktopEvent::Resume => Action::Resume,
                    DesktopEvent::TrayReset => Action::TrayReset,
                };
                actions.borrow_mut().push((name, 0, String::new()));
            }
            let pending = std::mem::take(&mut *actions.borrow_mut());
            for (name, index, value) in pending {
                if let Err(e) = c.action(&ui, name, index, &value) {
                    c.error(&format!("{e:#}"));
                }
            }
            c.runtime
                .window_keys
                .extend(window_keys.borrow_mut().drain(..));
            c.tick(&ui);
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    let mut c = controller.borrow_mut();
    c.save_geometry(&ui);
    c.flush(true);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegeMode {
    Config,
    User,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupOptions {
    handoff: Option<u32>,
    privilege: PrivilegeMode,
}

fn parse_startup_options(args: &[String]) -> Result<StartupOptions> {
    let mut options = StartupOptions {
        handoff: None,
        privilege: PrivilegeMode::Config,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--user" => {
                if options.privilege == PrivilegeMode::Admin {
                    bail!("--user and --admin cannot be used together");
                }
                options.privilege = PrivilegeMode::User;
            }
            "--admin" => {
                if options.privilege == PrivilegeMode::User {
                    bail!("--user and --admin cannot be used together");
                }
                options.privilege = PrivilegeMode::Admin;
            }
            "--handoff" => {
                let value = args
                    .get(index + 1)
                    .context("--handoff requires a process id")?;
                options.handoff = Some(
                    value
                        .parse::<u32>()
                        .with_context(|| format!("Invalid --handoff process id `{value}`"))?,
                );
                index += 1;
            }
            argument => {
                bail!("Unknown argument `{argument}`. Supported arguments: --user, --admin.")
            }
        }
        index += 1;
    }
    Ok(options)
}

fn should_request_admin(privilege: PrivilegeMode, configured_always_admin: bool) -> bool {
    match privilege {
        PrivilegeMode::User => false,
        PrivilegeMode::Admin => true,
        PrivilegeMode::Config => configured_always_admin,
    }
}

#[cfg(test)]
mod startup_option_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn defaults_to_configured_privilege_mode() {
        assert_eq!(
            parse_startup_options(&args(&[])).unwrap(),
            StartupOptions {
                handoff: None,
                privilege: PrivilegeMode::Config,
            }
        );
    }

    #[test]
    fn explicit_user_and_admin_modes_override_config() {
        assert_eq!(
            parse_startup_options(&args(&["--user"])).unwrap().privilege,
            PrivilegeMode::User
        );
        assert_eq!(
            parse_startup_options(&args(&["--admin"]))
                .unwrap()
                .privilege,
            PrivilegeMode::Admin
        );
        assert!(!should_request_admin(PrivilegeMode::User, true));
        assert!(should_request_admin(PrivilegeMode::Admin, false));
        assert!(should_request_admin(PrivilegeMode::Config, true));
        assert!(!should_request_admin(PrivilegeMode::Config, false));
    }

    #[test]
    fn parses_private_handoff_argument() {
        assert_eq!(
            parse_startup_options(&args(&["--handoff", "1234"])).unwrap(),
            StartupOptions {
                handoff: Some(1234),
                privilege: PrivilegeMode::Config,
            }
        );
    }

    #[test]
    fn rejects_conflicting_or_unknown_arguments() {
        assert!(parse_startup_options(&args(&["--user", "--admin"])).is_err());
        assert!(parse_startup_options(&args(&["--unexpected"])).is_err());
        assert!(parse_startup_options(&args(&["--handoff"])).is_err());
    }
}

struct Controller {
    runtime: Runtime,
    path: PathBuf,
    logs: Logs,
    desktop: Desktop,
    saves: SaveQueue,
    status: administrator::Status,
    fatal: Option<String>,
    passed: bool,
    recovering: bool,
    wait_warning: WaitWarning,
    started: Instant,
    last_slow: Instant,
    stage_key: String,
    stage_since: Instant,
    toast: String,
    toast_until: Instant,
    capture: Option<CaptureTarget>,
    capture_error: String,
    last_error: String,
    last_notification: Instant,
    export: Option<std::sync::mpsc::Receiver<Result<Option<PathBuf>, String>>>,
    shutdown: bool,
    hidden: bool,
    previous_devices: Option<(
        Vec<Target>,
        Option<String>,
        bool,
        taprelay_core::devices::AdapterState,
    )>,
    previous_bindings: Option<u64>,
    previous_checks: Option<([bool; 4], bool, bool)>,
    previous_capture_keys: Vec<String>,
    system_theme: bool,
    system_language: bool,
    last_system_poll: Instant,
    last_ui_sync: Instant,
    last_log_revision: u64,
    last_log_filter: String,
    last_dropped_logs: usize,
    log_model: Rc<VecModel<slint::SharedString>>,
}
impl Controller {
    fn new(
        mut config: Config,
        path: PathBuf,
        logs: Logs,
        desktop: Desktop,
        status: administrator::Status,
        fatal: Option<String>,
    ) -> Self {
        config.options.autostart = desktop::autostart_enabled();
        let now = Instant::now();
        Self {
            runtime: Runtime::new(config),
            path,
            logs,
            desktop,
            saves: SaveQueue::default(),
            status,
            fatal,
            passed: false,
            recovering: false,
            wait_warning: WaitWarning::default(),
            started: now,
            last_slow: now - Duration::from_secs(2),
            stage_key: String::new(),
            stage_since: now,
            toast: String::new(),
            toast_until: now,
            capture: None,
            capture_error: String::new(),
            last_error: String::new(),
            last_notification: now - Duration::from_secs(120),
            export: None,
            shutdown: false,
            hidden: false,
            previous_devices: None,
            previous_bindings: None,
            previous_checks: None,
            previous_capture_keys: vec![],
            system_theme: desktop::system_dark(),
            system_language: desktop::system_chinese(),
            last_system_poll: now,
            last_ui_sync: now,
            last_log_revision: 0,
            last_log_filter: String::new(),
            last_dropped_logs: 0,
            log_model: Rc::new(VecModel::default()),
        }
    }
    fn zh(&self) -> bool {
        match self.runtime.config.options.language {
            Language::Chinese => true,
            Language::English => false,
            Language::System => self.system_language,
        }
    }
    fn tr<'a>(&self, key: &'a str) -> &'a str {
        i18n::text(self.zh(), key)
    }
    fn say(&mut self, message: String) {
        self.toast = message;
        self.toast_until = Instant::now() + Duration::from_secs(6);
    }
    fn error(&mut self, message: &str) {
        tracing::error!("{message}");
        self.say(format!("{}: {message}", self.tr(keys::ERROR_ACTION)));
        if self.runtime.config.options.notifications
            && self.last_notification.elapsed() > Duration::from_secs(60)
        {
            self.desktop.notify("TapRelay", message);
            self.last_notification = Instant::now();
        }
    }
    fn flush(&mut self, force: bool) {
        if self.fatal.is_some() {
            return;
        }
        if let Err(e) = self.saves.flush(&self.runtime.config, &self.path, force) {
            self.error(&format!("{}: {e:#}", self.tr(keys::ERROR_SAVE)));
        }
    }
    fn recording(&self) -> bool {
        self.capture.is_some()
    }
    fn cancel_capture(&mut self) {
        self.runtime.finish_recording();
        self.capture = None;
        self.capture_error.clear();
    }
    fn action(&mut self, ui: &AppWindow, name: Action, index: i32, value: &str) -> Result<()> {
        if !matches!(name, Action::Show | Action::Resume | Action::TrayReset) {
            self.runtime.consume_ui_input();
        }
        if (!self.passed || self.fatal.is_some())
            && !matches!(
                name,
                Action::Show
                    | Action::Close
                    | Action::Quit
                    | Action::Retry
                    | Action::BluetoothSettings
                    | Action::TrayReset
            )
        {
            return Ok(());
        }
        match name {
            Action::Show => {
                ui.show()?;
                ui.window().set_minimized(false);
                desktop::foreground_app();
                self.hidden = false;
            }
            Action::Close => {
                if self.runtime.config.options.close_to_tray && self.fatal.is_none() {
                    self.cancel_capture();
                    self.save_geometry(ui);
                    self.flush(true);
                    if !self.runtime.config.options.close_hint_seen {
                        self.desktop
                            .notify("TapRelay", self.tr(keys::TRAY_CLOSE_HINT));
                        self.runtime.config.options.close_hint_seen = true;
                        self.saves.changed();
                    }
                    ui.hide()?;
                    self.hidden = true;
                } else {
                    self.action(ui, Action::Quit, 0, "")?;
                }
            }
            Action::Quit => {
                self.cancel_capture();
                self.save_geometry(ui);
                self.flush(true);
                self.shutdown = true;
                slint::quit_event_loop()?;
            }
            Action::Navigate => {
                if self.recording() {
                    self.cancel_capture();
                }
                ui.set_page(index.clamp(0, 4));
            }
            Action::Listen => {
                self.runtime.set_listening(!self.runtime.listening)?;
            }
            Action::Test => {
                self.runtime.send()?;
            }
            Action::Refresh => self.runtime.refresh()?,
            Action::Retry | Action::Resume => {
                if self.recovering && self.started.elapsed() < Duration::from_secs(2) {
                    return Ok(());
                }
                self.recovering = true;
                self.runtime.start_bluetooth()?;
                self.started = Instant::now();
                self.stage_since = Instant::now();
                self.last_error.clear();
                if name == Action::Resume && self.runtime.listening {
                    self.cancel_capture();
                    self.runtime.set_listening(false)?;
                    self.runtime.set_listening(true)?;
                }
            }
            Action::TrayReset => self.desktop.restore(),
            Action::BluetoothSettings => self.runtime.bluetooth_settings()?,
            Action::DataFolder => desktop::open(
                &self
                    .path
                    .parent()
                    .context("Missing data folder")?
                    .to_string_lossy(),
            )?,
            Action::LogsFolder => {
                desktop::open(&self.path.parent().unwrap().join("logs").to_string_lossy())?
            }
            Action::Capture => {
                self.cancel_capture();
                anyhow::ensure!(
                    index < 0 || (index as usize) < self.runtime.config.bindings.len(),
                    "Binding no longer exists"
                );
                self.runtime.record()?;
                if index >= 0 {
                    self.capture = Some(CaptureTarget::Existing(index as usize));
                } else {
                    self.capture = Some(CaptureTarget::New);
                }
            }
            Action::CancelCapture => self.cancel_capture(),
            Action::Delete => {
                self.cancel_capture();
                if (index as usize) < self.runtime.config.bindings.len() {
                    self.runtime.config.bindings.remove(index as usize);
                    self.runtime.bindings_changed();
                    self.saves.changed();
                }
            }
            Action::Device | Action::PairDevice => {
                if let Some(target) = self
                    .runtime
                    .state
                    .targets
                    .iter()
                    .find(|t| t.matches_id(value))
                    .cloned()
                {
                    if name == Action::PairDevice {
                        self.runtime.pair(target.id)?;
                    } else {
                        self.runtime.choose(target.id)?;
                    }
                }
            }
            Action::DisconnectDevice => self.runtime.disconnect()?,
            Action::Wizard => {
                self.cancel_capture();
                ui.set_mode(1);
                ui.set_wizard_page(0);
                self.runtime.config.wizard.dismissed = false;
                self.runtime.config.wizard.page = 0;
                self.saves.changed();
            }
            Action::WizardNext | Action::WizardBack => {
                if name == Action::WizardNext
                    && ui.get_wizard_page() == 1
                    && !taprelay_core::devices::receiver_next_allowed(&self.runtime.state)
                {
                    return Ok(());
                }
                let page = (ui.get_wizard_page() + if name == Action::WizardNext { 1 } else { -1 })
                    .clamp(0, 2);
                ui.set_wizard_page(page);
                self.runtime.config.wizard.page = page as u8;
                self.saves.changed();
            }
            Action::WizardFinish => {
                self.cancel_capture();
                self.runtime.config.wizard.dismissed = true;
                self.saves.changed();
                ui.set_mode(2);
                ui.set_page(0);
            }
            Action::Theme => {
                self.runtime.config.options.theme = match index {
                    1 => ThemeSetting::Light,
                    2 => ThemeSetting::Dark,
                    _ => ThemeSetting::System,
                };
                self.saves.changed();
            }
            Action::Language => {
                self.runtime.config.options.language = match index {
                    1 => Language::Chinese,
                    2 => Language::English,
                    _ => Language::System,
                };
                self.saves.changed();
            }
            Action::Setting => {
                anyhow::ensure!(matches!(value, "0" | "1"), "Invalid setting value");
                let v = value == "1";
                match index {
                    0 => {
                        desktop::set_autostart(v)?;
                        self.runtime.config.options.autostart = desktop::autostart_enabled();
                    }
                    1 => self.runtime.config.options.start_hidden = v,
                    2 => self.runtime.config.options.auto_listen = v,
                    3 => self.runtime.config.options.close_to_tray = v,
                    4 => self.runtime.config.options.always_admin = v,
                    5 => self.runtime.config.options.notifications = v,
                    6 => self.runtime.config.options.connection_wait_warning = v,
                    _ => anyhow::bail!("Unknown setting index: {index}"),
                }
                self.saves.changed();
            }
            Action::Elevate => {
                self.save_geometry(ui);
                self.runtime.config.save(&self.path)?;
                if !self.status.elevated && administrator::restart()? {
                    self.shutdown = true;
                    slint::quit_event_loop()?;
                }
            }
            Action::Logs => self.last_log_filter.clear(),
            Action::ClearLogs => {
                self.logs.clear();
                self.log_model.set_vec(vec![]);
                ui.set_log_text("".into());
                self.last_log_filter.clear();
            }
            Action::CopyLogs => {
                self.desktop.copy_text(&ui.get_log_text())?;
                self.say(self.tr(keys::LOGS_COPIED).into());
            }
            Action::ExportLogs => {
                if self.export.is_none() {
                    self.export = Some(desktop::export_logs(
                        self.path
                            .parent()
                            .context("Missing data folder")?
                            .join("taprelay-export.log"),
                        ui.get_log_text().to_string(),
                    )?);
                }
            }
        }
        self.sync(ui);
        Ok(())
    }
    fn tick(&mut self, ui: &AppWindow) {
        if self.shutdown {
            return;
        }
        let now = Instant::now();
        if self.fatal.is_none() {
            self.runtime.tick();
            let active = !self.hidden
                && ((ui.get_mode() == 1 && ui.get_wizard_page() == 1)
                    || (ui.get_mode() == 2 && ui.get_page() == 2));
            if let Err(e) = self.runtime.discover(active) {
                tracing::warn!("Discovery lifecycle: {e}");
            }
        }
        if self.recording() {
            if !desktop::foreground_is_ours() || self.runtime.capture_cancelled {
                self.cancel_capture();
            } else if let Some(t) = self.runtime.learned.take() {
                if !t.valid() {
                    self.capture_error = self.tr(keys::CAPTURE_INVALID).into();
                } else if self
                    .runtime
                    .config
                    .bindings
                    .iter()
                    .enumerate()
                    .any(|(i, b)| Some(CaptureTarget::Existing(i)) != self.capture && b.tap == t)
                {
                    self.capture_error = self.tr(keys::CAPTURE_DUPLICATE).into();
                } else {
                    let common = matches!(&t,taprelay_core::input::Trigger::Keyboard{keys} if keys.len()==1 && matches!(keys[0],0x0d|0x20|0x30..=0x5a));
                    if let Some(CaptureTarget::Existing(i)) = self.capture {
                        self.runtime.config.bindings[i].tap = t;
                    } else {
                        self.runtime.config.bindings.push(Binding {
                            tap: t,
                            relay: MediaCommand::PlayPause,
                            enabled: true,
                        });
                    }
                    self.runtime.bindings_changed();
                    self.saves.changed();
                    self.cancel_capture();
                    if common {
                        self.say(self.tr(keys::BINDINGS_COMMON_KEY_HINT).into());
                    }
                }
            }
        }
        if let Some(e) = self.runtime.error.take() {
            if !self.runtime.state.input {
                self.error(&e);
            } else {
                tracing::warn!("{e}");
                self.say(e);
            }
        }
        let state = &self.runtime.state;
        let passed = state.adapter && state.peripheral && state.service && state.broadcasting;
        if passed || state.last_error.is_some() {
            self.recovering = false;
        }
        if !self.passed && passed && self.fatal.is_none() {
            self.passed = true;
            ui.set_mode(if self.runtime.config.wizard.dismissed {
                2
            } else {
                1
            });
            ui.set_wizard_page(self.runtime.config.wizard.page.into());
            if self.runtime.config.options.auto_listen
                && let Err(e) = self.runtime.set_listening(true)
            {
                self.error(&e.to_string());
            }
            if self.runtime.config.options.start_hidden && self.runtime.config.wizard.dismissed {
                let _ = ui.hide();
                self.hidden = true;
            }
        }
        if let Some(e) = &self.runtime.state.last_error
            && self.last_error != *e
        {
            let e = e.clone();
            self.last_error = e.clone();
            if self.passed && (!self.runtime.state.adapter || !self.runtime.state.service) {
                self.error(&e);
            }
        }
        let warning_enabled = self.passed
            && self.runtime.config.options.connection_wait_warning
            && self.runtime.config.options.notifications
            && self.runtime.config.wizard.dismissed;
        if self.wait_warning.update(
            self.runtime.state.selected.as_deref(),
            self.runtime.state.ready,
            warning_enabled,
            now,
        ) {
            self.error(self.tr(keys::RECEIVER_TIMEOUT));
        }
        if let Some(result) = self.export.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("Log export worker stopped".into()))
            }
        }) {
            self.export = None;
            match result {
                Ok(Some(path)) => self.say(format!(
                    "{} {}",
                    self.tr(keys::LOGS_EXPORTED),
                    path.display()
                )),
                Ok(None) => {}
                Err(e) => self.error(&e),
            }
        }
        self.flush(false);
        if !self.hidden || self.last_ui_sync.elapsed() >= Duration::from_secs(1) {
            self.sync(ui);
            self.last_ui_sync = now;
        }
        if self.last_slow.elapsed() >= Duration::from_secs(1) {
            self.last_slow = now;
            let tip = format!("TapRelay · {} · {}", ui.get_device_name(), ui.get_stage());
            self.desktop.update(
                ui.get_state_color() as u8,
                &tip,
                self.runtime.listening,
                i18n::tray_labels(self.zh()),
            );
            self.update_logs(ui);
            let dropped = self.logs.dropped_lines();
            if dropped != self.last_dropped_logs {
                tracing::warn!(
                    dropped_since_last_check = dropped.saturating_sub(self.last_dropped_logs),
                    total_dropped = dropped,
                    "Disk log queue overflow; some events were not written"
                );
                self.last_dropped_logs = dropped;
            }
            if let Err(e) = logging::cleanup_due(self.path.parent().unwrap()) {
                tracing::warn!("Log cleanup: {e}");
            }
        }
    }
    fn sync(&mut self, ui: &AppWindow) {
        if self.last_system_poll.elapsed() >= Duration::from_secs(1) {
            self.system_theme = desktop::system_dark();
            self.system_language = desktop::system_chinese();
            self.last_system_poll = Instant::now();
        }
        if self.previous_bindings != Some(self.runtime.bindings_revision) {
            ui.set_binding_summary(
                self.runtime
                    .config
                    .bindings
                    .iter()
                    .filter(|b| b.enabled)
                    .take(3)
                    .map(|b| b.tap.to_string())
                    .collect::<Vec<_>>()
                    .join(" · ")
                    .into(),
            );
        }
        let zh = self.zh();
        i18n::apply(ui, zh);
        let o = &self.runtime.config.options;
        let dark = match o.theme {
            ThemeSetting::System => self.system_theme,
            ThemeSetting::Dark => true,
            ThemeSetting::Light => false,
        };
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.set_theme_choice(match o.theme {
            ThemeSetting::System => 0,
            ThemeSetting::Light => 1,
            ThemeSetting::Dark => 2,
        });
        ui.set_language_choice(match o.language {
            Language::System => 0,
            Language::Chinese => 1,
            Language::English => 2,
        });
        ui.set_auto_start(o.autostart);
        ui.set_auto_listen(o.auto_listen);
        ui.set_start_hidden(o.start_hidden);
        ui.set_close_to_tray(o.close_to_tray);
        ui.set_always_admin(o.always_admin);
        ui.set_notifications(o.notifications);
        ui.set_connection_wait_warning(o.connection_wait_warning);
        ui.set_elevated(self.status.elevated);
        ui.set_account_admin(self.status.account_admin);
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        ui.set_data_directory(
            self.path
                .parent()
                .unwrap()
                .to_string_lossy()
                .as_ref()
                .into(),
        );
        let s = &self.runtime.state;
        let flags = [s.adapter, s.peripheral, s.service, s.broadcasting];
        ui.set_bluetooth_available(
            s.adapter_state == taprelay_core::devices::AdapterState::Available,
        );
        ui.set_scanning(s.discovery == taprelay_core::devices::DiscoveryState::Scanning);
        ui.set_adapter_label(
            self.tr(crate::receiver_view::adapter_label_key(s.adapter_state))
                .into(),
        );
        ui.set_receiver_next_allowed(taprelay_core::devices::receiver_next_allowed(s));
        ui.set_device_selected(s.selected.is_some());
        ui.set_discovery_status(self.tr(crate::receiver_view::page_status_key(s)).into());
        let first = flags.iter().position(|p| !*p).unwrap_or(4);
        let timeout = !self.passed && self.started.elapsed() > Duration::from_secs(30);
        let failure = self
            .fatal
            .clone()
            .or_else(|| s.last_error.clone())
            .or_else(|| {
                (self.passed && !self.recovering && flags.iter().any(|v| !*v))
                    .then(|| self.tr(keys::RECEIVER_UNAVAILABLE).to_owned())
            })
            .or_else(|| timeout.then(|| self.tr(keys::STARTUP_TIMEOUT).into()));
        if self.fatal.is_some() {
            ui.set_mode(3);
        }
        ui.set_problem(failure.clone().unwrap_or_default().into());
        ui.set_busy((!self.passed || self.recovering) && failure.is_none());
        let names = [
            keys::CHECK_BLUETOOTH,
            keys::CHECK_PERIPHERAL,
            keys::CHECK_SERVICE,
            keys::CHECK_ADVERTISING,
        ]
        .map(|key| self.tr(key));
        let checks_key = (flags, failure.is_some(), zh);
        if self.previous_checks != Some(checks_key) {
            self.previous_checks = Some(checks_key);
            ui.set_checks(ModelRc::new(VecModel::from(
                (0..4)
                    .map(|i| CheckRow {
                        title: names[i].into(),
                        state: if flags[i] {
                            2
                        } else if i == first {
                            if failure.is_some() { 3 } else { 1 }
                        } else {
                            0
                        },
                    })
                    .collect::<Vec<_>>(),
            )));
        }
        let target = s.target_status.as_ref();
        let (key, stage) = if self.recovering && failure.is_none() {
            ("prepare", self.tr(keys::RECEIVER_PREPARING))
        } else if self.fatal.is_some()
            || (self.passed && (!s.adapter || !s.service || !s.broadcasting))
        {
            ("fault", self.tr(keys::RECEIVER_FAULT))
        } else if s.activity == TransportActivity::ResolvingDevice {
            ("identity", self.tr(keys::RECEIVER_RESOLVING))
        } else if s.selected.is_none() {
            ("unselected", self.tr(keys::RECEIVER_CHOOSE))
        } else {
            let status = crate::receiver_view::session_status_key(s);
            (status, self.tr(status))
        };
        let stage = stage.to_owned();
        if self.stage_key != key {
            tracing::info!(previous=%self.stage_key,elapsed_ms=self.stage_since.elapsed().as_millis(),next=key,"Connection stage changed");
            self.stage_key = key.into();
            self.stage_since = Instant::now();
        }
        ui.set_stage(stage.into());
        ui.set_stage_detail(if s.ready {
            if self.runtime.config.bindings.iter().any(|b| b.enabled) {
                self.tr(keys::RECEIVER_READY_HINT).into()
            } else {
                self.tr(keys::RECEIVER_WAIT_BINDING).into()
            }
        } else {
            format!(
                "{} {} s{}",
                self.tr(keys::RECEIVER_WAIT_TIME),
                self.stage_since.elapsed().as_secs(),
                if self.stage_since.elapsed() > Duration::from_secs(20) {
                    self.tr(keys::RECEIVER_WAIT_HINT)
                } else {
                    ""
                }
            )
            .into()
        });
        ui.set_ready(s.ready);
        ui.set_listening(self.runtime.listening);
        ui.set_state_color(if (!self.passed && failure.is_some()) || key == "fault" {
            3
        } else if self.passed && !self.runtime.listening {
            1
        } else if s.ready && self.runtime.config.bindings.iter().any(|b| b.enabled) {
            0
        } else {
            2
        });
        ui.set_device_name(
            s.target_status
                .as_ref()
                .filter(|_| s.selected.is_some())
                .map(|t| t.name.as_str())
                .unwrap_or(self.tr(keys::RECEIVER_NONE))
                .into(),
        );
        ui.set_diagnostics(format!("Adapter={} · Peripheral={} · Service={} · Advertising={}\nLink={:?} · Subscription={:?} · Input={}\n{}: {}",s.adapter,s.peripheral,s.service,s.broadcasting,target.map(|t|t.link),target.map(|t|t.subscribed),s.input,self.tr(keys::DIAGNOSTICS_INPUTS),self.runtime.matched).into());
        if self.previous_bindings != Some(self.runtime.bindings_revision) {
            self.previous_bindings = Some(self.runtime.bindings_revision);
            ui.set_bindings(ModelRc::new(VecModel::from(
                self.runtime
                    .config
                    .bindings
                    .iter()
                    .map(|b| BindingRow {
                        text: b.tap.to_string().into(),
                        keys: ModelRc::new(VecModel::from(
                            b.tap
                                .key_labels()
                                .into_iter()
                                .map(Into::into)
                                .collect::<Vec<slint::SharedString>>(),
                        )),
                        enabled: b.enabled,
                    })
                    .collect::<Vec<_>>(),
            )));
        }
        if self
            .previous_devices
            .as_ref()
            .is_none_or(|(targets, selected, locale, adapter)| {
                targets != &s.targets
                    || selected != &s.selected
                    || *locale != zh
                    || *adapter != s.adapter_state
            })
        {
            self.previous_devices =
                Some((s.targets.clone(), s.selected.clone(), zh, s.adapter_state));
            let rows: Vec<_> = s
                .targets
                .iter()
                .map(|d| crate::receiver_view::row(d, s, |key| self.tr(key).to_owned()))
                .collect();
            ui.set_paired_devices(ModelRc::new(VecModel::from(
                rows.iter()
                    .filter(|r| r.paired)
                    .cloned()
                    .collect::<Vec<_>>(),
            )));
            ui.set_nearby_devices(ModelRc::new(VecModel::from(
                rows.iter()
                    .filter(|r| !r.paired)
                    .cloned()
                    .collect::<Vec<_>>(),
            )));
        }
        ui.set_capture_index(
            self.capture
                .map(|target| match target {
                    CaptureTarget::Existing(i) => i as i32,
                    CaptureTarget::New => -2,
                })
                .unwrap_or(-1),
        );
        ui.set_capture_text(if self.runtime.capture_waiting() {
            self.tr(keys::CAPTURE_RELEASE).into()
        } else if self.runtime.capture_preview.is_empty() {
            self.tr(keys::CAPTURE_RECORDING).into()
        } else {
            self.runtime.capture_preview.clone().into()
        });
        ui.set_capture_error(self.capture_error.clone().into());
        let capture_keys = self.runtime.capture_key_labels();
        if capture_keys != self.previous_capture_keys {
            self.previous_capture_keys.clone_from(&capture_keys);
            ui.set_capture_keys(ModelRc::new(VecModel::from(
                capture_keys
                    .into_iter()
                    .map(Into::into)
                    .collect::<Vec<slint::SharedString>>(),
            )));
        }
        ui.set_test_result(
            match &self.runtime.test {
                TestStatus::Idle => String::new(),
                TestStatus::Pending(_) => self.tr(keys::TEST_SENDING).into(),
                TestStatus::Succeeded => self.tr(keys::TEST_SENT).into(),
                TestStatus::Failed(error) => format!("{}: {error}", self.tr(keys::ERROR_ACTION)),
            }
            .into(),
        );
        ui.set_input_count(self.runtime.matched.min(i32::MAX as u64) as i32);
        ui.set_toast(if Instant::now() < self.toast_until {
            self.toast.clone().into()
        } else {
            "".into()
        });
    }
    fn update_logs(&mut self, ui: &AppWindow) {
        let revision = self
            .logs
            .revision
            .load(std::sync::atomic::Ordering::Acquire);
        let filter = format!("{}:{}", ui.get_log_filter(), ui.get_log_search());
        if ui.get_logs_paused() && filter == self.last_log_filter {
            return;
        }
        if revision == self.last_log_revision && filter == self.last_log_filter {
            return;
        }
        self.last_log_revision = revision;
        self.last_log_filter = filter;
        let lines = self
            .logs
            .filtered(ui.get_log_filter(), &ui.get_log_search());
        for (index, line) in lines.iter().enumerate() {
            match self.log_model.row_data(index) {
                None => self.log_model.push(line.as_str().into()),
                Some(previous) if previous.as_str() != line => {
                    self.log_model.set_row_data(index, line.as_str().into())
                }
                _ => {}
            }
        }
        while self.log_model.row_count() > lines.len() {
            self.log_model.remove(self.log_model.row_count() - 1);
        }
        let text = lines.join("\n");
        ui.set_log_text(text.into());
    }
    fn save_geometry(&mut self, ui: &AppWindow) {
        if self.hidden || self.fatal.is_some() {
            return;
        }
        let g = &mut self.runtime.config.window;
        g.maximized = ui.window().is_maximized();
        if !g.maximized && !ui.window().is_minimized() {
            let size = ui.window().size().to_logical(ui.window().scale_factor());
            g.width = size.width;
            g.height = size.height;
            let p = ui.window().position();
            g.x = Some(p.x);
            g.y = Some(p.y);
        }
        self.saves.changed();
    }
}
