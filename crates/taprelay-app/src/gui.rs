use crate::{
    AppWindow, BindingCaptureState, BindingRow, BindingUi, CheckRow, FunctionBindingRow,
    GestureAction, ShortcutItem, Theme, ThemeMode,
    action::{self, Action, BindingCommand, CaptureTarget},
    administrator,
    config::{self, Config, Device, Language, SaveQueue, Theme as ThemeSetting},
    feedback::{TestStatus, WaitWarning},
    i18n::{self, keys},
    logging,
    platform::desktop::{self, Desktop, DesktopEvent},
    runtime_worker::RuntimeHandle as Runtime,
};
use anyhow::{Context, Result, bail};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use taprelay_core::{
    function::{self, FunctionId},
    state::{Target, TransportActivity},
};

fn dispatch_controller(
    controller: &Rc<RefCell<Controller>>,
    window: &slint::Weak<AppWindow>,
    command: impl FnOnce(&mut Controller, &AppWindow) -> Result<()>,
) {
    let Some(ui) = window.upgrade() else { return };
    let Ok(mut controller) = controller.try_borrow_mut() else {
        tracing::error!("Ignored re-entrant UI command");
        return;
    };
    if let Err(error) = command(&mut controller, &ui) {
        controller.error(&format!("{error:#}"));
        controller.sync(&ui);
    }
}

fn capture_target(function_id: &str, slot: i32) -> Result<CaptureTarget> {
    Ok(CaptureTarget {
        id: FunctionId::from_stable_id(function_id).context("Unknown function")?,
        slot: usize::try_from(slot).context("Invalid shortcut slot")?,
    })
}

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
    #[cfg(windows)]
    crate::window_rendering::install(ui.window());
    let mut startup_error = loaded.as_ref().err().map(|e| format!("{e:#}"));
    if let Err(e) = config::check_writable(&path) {
        startup_error = Some(format!("{e:#}"));
    }
    let config = loaded.unwrap_or_default();
    let log_init = logging::init(&path);
    let (logs, _guard) = match log_init {
        Ok((l, g)) => (Some(l), Some(g)),
        Err(e) => {
            startup_error = Some(format!("Cannot open logs: {e:#}"));
            (None, None)
        }
    };
    tracing::info!(
        version = crate::version::VERSION,
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
    let actions = Rc::new(RefCell::new(Vec::<(Action, String)>::new()));
    let window_keys = Rc::new(RefCell::new(Vec::new()));
    let a = actions.clone();
    ui.window().on_close_requested(move || {
        a.borrow_mut().push((Action::Close, String::new()));
        slint::CloseRequestResponse::KeepWindowShown
    });
    let mut controller = Controller::new(config, path, logs, desktop, status, startup_error)?;
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
    let action_controller = controller.clone();
    let action_window = ui.as_weak();
    ui.on_action(move |name, value| match Action::try_from(name.as_str()) {
        Ok(action) => dispatch_controller(&action_controller, &action_window, |controller, ui| {
            controller.action(ui, action, &value)
        }),
        Err(e) => tracing::error!("{e}"),
    });

    let binding_ui = ui.global::<BindingUi>();
    let binding_controller = controller.clone();
    let binding_window = ui.as_weak();
    binding_ui.on_begin_capture(move |function_id, slot| {
        dispatch_controller(&binding_controller, &binding_window, |controller, ui| {
            controller.binding_action(
                ui,
                BindingCommand::BeginCapture(capture_target(&function_id, slot)?),
            )
        });
    });
    let binding_controller = controller.clone();
    let binding_window = ui.as_weak();
    binding_ui.on_cancel_capture(move || {
        dispatch_controller(&binding_controller, &binding_window, |controller, ui| {
            controller.binding_action(ui, BindingCommand::CancelCapture)
        });
    });
    let binding_controller = controller.clone();
    let binding_window = ui.as_weak();
    binding_ui.on_delete_shortcut(move |function_id, slot| {
        dispatch_controller(&binding_controller, &binding_window, |controller, ui| {
            controller.binding_action(
                ui,
                BindingCommand::DeleteShortcut(capture_target(&function_id, slot)?),
            )
        });
    });
    let binding_controller = controller.clone();
    let binding_window = ui.as_weak();
    binding_ui.on_set_function_enabled(move |function_id, enabled| {
        dispatch_controller(&binding_controller, &binding_window, |controller, ui| {
            let id = FunctionId::from_stable_id(&function_id).context("Unknown function")?;
            controller.binding_action(ui, BindingCommand::SetFunctionEnabled { id, enabled })
        });
    });
    let key_controller = controller.clone();
    let keys = window_keys.clone();
    binding_ui.on_key_input(move |text, down| {
        if desktop::ui_input_is_injected() {
            return;
        }
        let Ok(controller) = key_controller.try_borrow() else {
            tracing::error!("Ignored re-entrant binding key input");
            return;
        };
        if !controller.recording() {
            tracing::warn!("Ignored binding key input while capture was inactive");
            return;
        }
        drop(controller);
        if let Some(key) = crate::capture_key::virtual_key(&text) {
            keys.borrow_mut().push(taprelay_core::input::InputEvent {
                code: taprelay_core::input::InputCode::Key(key),
                down,
                captured: Instant::now(),
            });
        } else {
            tracing::debug!(key_text = %text, "Ignored unsupported binding key");
        }
    });
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
                actions.borrow_mut().push((name, String::new()));
            }
            let pending = std::mem::take(&mut *actions.borrow_mut());
            for (name, value) in pending {
                if let Err(e) = c.action(&ui, name, &value) {
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
#[allow(clippy::items_after_test_module)]
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
    logs: Option<tracing_appender::non_blocking::ErrorCounter>,
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
    previous_test: TestStatus,
    capture: Option<CaptureTarget>,
    capture_error: String,
    last_error: String,
    last_notification: Instant,
    shutdown: bool,
    hidden: bool,
    previous_devices: Option<(
        Vec<Target>,
        Option<String>,
        bool,
        taprelay_core::devices::AdapterState,
    )>,
    previous_bindings: Option<(u64, bool)>,
    previous_checks: Option<([bool; 4], bool, bool)>,
    system_theme: bool,
    system_language: bool,
    last_system_poll: Instant,
    last_ui_sync: Instant,
    last_dropped_logs: usize,
    persisted_remembered_device: Option<Device>,
}
impl Controller {
    fn new(
        mut config: Config,
        path: PathBuf,
        logs: Option<tracing_appender::non_blocking::ErrorCounter>,
        desktop: Desktop,
        status: administrator::Status,
        fatal: Option<String>,
    ) -> Result<Self> {
        config.options.autostart = desktop::autostart_enabled();
        let persisted_remembered_device = config.remembered_device.clone();
        let now = Instant::now();
        Ok(Self {
            runtime: Runtime::new(config)?,
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
            previous_test: TestStatus::Idle,
            capture: None,
            capture_error: String::new(),
            last_error: String::new(),
            last_notification: now - Duration::from_secs(120),
            shutdown: false,
            hidden: false,
            previous_devices: None,
            previous_bindings: None,
            previous_checks: None,
            system_theme: desktop::system_dark(),
            system_language: desktop::system_chinese(),
            last_system_poll: now,
            last_ui_sync: now,
            last_dropped_logs: 0,
            persisted_remembered_device,
        })
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
        self.track_remembered_device();
        if let Err(e) = self.saves.flush(&self.runtime.config, &self.path, force) {
            self.error(&format!("{}: {e:#}", self.tr(keys::ERROR_SAVE)));
        }
    }
    fn track_remembered_device(&mut self) {
        if self.persisted_remembered_device != self.runtime.config.remembered_device {
            self.persisted_remembered_device = self.runtime.config.remembered_device.clone();
            self.saves.changed();
        }
    }
    fn recording(&self) -> bool {
        self.capture.is_some()
    }
    fn apply_binding_command(&mut self, command: BindingCommand) -> Result<()> {
        if command == BindingCommand::CancelCapture
            && self.capture.is_none()
            && !self.runtime.recording
        {
            self.capture_error.clear();
            return Ok(());
        }

        let result = self.runtime.apply_binding_command(command);
        if result.is_ok() {
            match command {
                BindingCommand::BeginCapture(target) => {
                    self.capture = Some(target);
                    self.capture_error.clear();
                    tracing::debug!(
                        function_id = target.id.stable_id(),
                        slot = target.slot,
                        "Binding capture started"
                    );
                }
                BindingCommand::CancelCapture => {
                    self.capture = None;
                    self.capture_error.clear();
                    tracing::debug!("Binding capture cancelled");
                }
                BindingCommand::DeleteShortcut(target) => {
                    self.capture = None;
                    self.capture_error.clear();
                    self.saves.changed();
                    tracing::info!(
                        function_id = target.id.stable_id(),
                        slot = target.slot,
                        "Binding shortcut deleted"
                    );
                }
                BindingCommand::SetFunctionEnabled { id, enabled } => {
                    self.capture = None;
                    self.capture_error.clear();
                    self.saves.changed();
                    tracing::info!(
                        function_id = id.stable_id(),
                        enabled,
                        "Binding function state changed"
                    );
                }
            }
        } else if !self.runtime.recording {
            // The worker may have completed a capture transition before a later
            // validation step failed. Mirror the confirmed worker state.
            self.capture = None;
            self.capture_error.clear();
        }
        result
    }
    fn cancel_capture(&mut self) -> Result<()> {
        self.apply_binding_command(BindingCommand::CancelCapture)
    }
    fn binding_action(&mut self, ui: &AppWindow, command: BindingCommand) -> Result<()> {
        self.apply_binding_command(command)?;
        self.sync(ui);
        Ok(())
    }
    fn action(&mut self, ui: &AppWindow, name: Action, value: &str) -> Result<()> {
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
                    self.cancel_capture()?;
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
                    self.action(ui, Action::Quit, "")?;
                }
            }
            Action::Quit => {
                self.cancel_capture()?;
                self.runtime.shutdown();
                self.save_geometry(ui);
                self.flush(true);
                self.shutdown = true;
                slint::quit_event_loop()?;
            }
            Action::Navigate => {
                if self.recording() {
                    self.cancel_capture()?;
                }
                ui.set_page(action::page(value)?.clamp(0, 3));
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
                    self.cancel_capture()?;
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
                self.cancel_capture()?;
                ui.set_mode(1);
                ui.set_wizard_page(0);
                self.runtime.config.wizard.dismissed = false;
                self.runtime.config.wizard.page = 0;
                self.saves.changed();
            }
            Action::WizardNext | Action::WizardBack => {
                if name == Action::WizardNext
                    && ui.get_wizard_page() == 2
                    && !taprelay_core::devices::receiver_next_allowed(&self.runtime.state)
                {
                    return Ok(());
                }
                let page = (ui.get_wizard_page() + if name == Action::WizardNext { 1 } else { -1 })
                    .clamp(0, 3);
                // Preserve step IDs saved by the former four-step wizard.
                let page = if page == 1 {
                    if name == Action::WizardNext { 2 } else { 0 }
                } else {
                    page
                };
                ui.set_wizard_page(page);
                self.runtime.config.wizard.page = page as u8;
                self.saves.changed();
            }
            Action::WizardFinish => {
                self.cancel_capture()?;
                self.runtime.config.wizard.dismissed = true;
                self.saves.changed();
                ui.set_mode(2);
                ui.set_page(0);
            }
            Action::Theme => {
                self.runtime.config.options.theme = ThemeSetting::from_name(value)
                    .with_context(|| format!("Unknown theme: {value}"))?;
                self.saves.changed();
            }
            Action::Language => {
                self.runtime.config.options.language = Language::from_name(value)
                    .with_context(|| format!("Unknown language: {value}"))?;
                self.saves.changed();
            }
            Action::SetStartHidden => {
                self.runtime.config.options.start_hidden = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetAutoListen => {
                self.runtime.config.options.auto_listen = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetCloseToTray => {
                self.runtime.config.options.close_to_tray = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetAlwaysAdmin => {
                self.runtime.config.options.always_admin = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetNotifications => {
                self.runtime.config.options.notifications = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetConnectionWaitWarning => {
                self.runtime.config.options.connection_wait_warning = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetAutostart => {
                desktop::set_autostart(action::toggle(value)?)?;
                // Read back what the OS accepted instead of trusting the request.
                self.runtime.config.options.autostart = desktop::autostart_enabled();
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
        }
        self.track_remembered_device();
        if self.recording() {
            if !desktop::foreground_is_ours() || self.runtime.capture_cancelled {
                if let Err(error) = self.cancel_capture() {
                    self.error(&format!("{error:#}"));
                }
            } else if let Some(t) = self.runtime.learned.take() {
                if !t.valid() {
                    self.capture_error = self.tr(keys::CAPTURE_INVALID).into();
                    tracing::debug!("Rejected invalid binding capture");
                } else if let Some(CaptureTarget { id, slot }) = self.capture {
                    if let Some(conflict) = self.runtime.shortcut_conflict(id, slot, &t) {
                        self.capture_error = format!(
                            "{}: {}",
                            self.tr(keys::CAPTURE_DUPLICATE),
                            self.tr(function::function_definition(conflict).name_key)
                        );
                        tracing::debug!(
                            function_id = id.stable_id(),
                            slot,
                            conflict = conflict.stable_id(),
                            "Rejected duplicate binding capture"
                        );
                    } else {
                        match self.runtime.replace_shortcut(id, slot, t) {
                            Ok(()) => {
                                self.saves.changed();
                                tracing::info!(
                                    function_id = id.stable_id(),
                                    slot,
                                    "Binding shortcut saved"
                                );
                                if let Err(error) = self.cancel_capture() {
                                    self.error(&format!("{error:#}"));
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    function_id = id.stable_id(),
                                    slot,
                                    error = %error,
                                    "Failed to save binding shortcut"
                                );
                                self.capture_error = error.to_string();
                            }
                        }
                    }
                } else {
                    self.capture_error = self.tr(keys::CAPTURE_INVALID).into();
                }
            } else if self.runtime.capture_invalid {
                self.runtime.capture_invalid = false;
                self.capture_error = self.tr(keys::CAPTURE_INVALID).into();
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
            let dropped = self
                .logs
                .as_ref()
                .map_or(0, |counter| counter.dropped_lines());
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
        ui.set_app_version(crate::version::VERSION.into());
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
        ui.set_ready(s.ready);
        ui.set_listening(self.runtime.listening);
        ui.set_state_color(if (!self.passed && failure.is_some()) || key == "fault" {
            3
        } else if self.passed && !self.runtime.listening {
            1
        } else if s.ready
            && self
                .runtime
                .config
                .functions
                .values()
                .any(|function| function.enabled && !function.shortcuts.is_empty())
        {
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
        if self.previous_bindings != Some((self.runtime.bindings_revision, zh)) {
            self.previous_bindings = Some((self.runtime.bindings_revision, zh));
            let mut rows = Vec::new();
            let mut summary = Vec::new();
            for definition in taprelay_core::function::FUNCTION_CATALOG {
                let Some(config) = self.runtime.config.functions.get(&definition.id) else {
                    continue;
                };
                // The catalog states a function as up to two gestures, so the
                // row shows exactly what each one does instead of naming the
                // pair and leaving the reader to guess which half is the hold.
                let mut gestures = vec![GestureAction {
                    gesture: self.tr(keys::BINDINGS_GESTURE_PRESS).into(),
                    action: self.tr(definition.action.name_key()).into(),
                }];
                if let Some(hold) = definition.hold_action {
                    gestures.push(GestureAction {
                        gesture: self.tr(keys::BINDINGS_GESTURE_HOLD).into(),
                        action: self.tr(hold.name_key()).into(),
                    });
                }
                let shortcuts = config
                    .shortcuts
                    .iter()
                    .enumerate()
                    .map(|(slot, shortcut)| ShortcutItem {
                        text: shortcut.display().into(),
                        keys: ModelRc::new(VecModel::from(
                            shortcut
                                .key_labels()
                                .into_iter()
                                .map(Into::into)
                                .collect::<Vec<slint::SharedString>>(),
                        )),
                        slot: slot as i32,
                        enabled: config.enabled,
                    })
                    .collect::<Vec<_>>();
                if config.enabled {
                    for shortcut in &config.shortcuts {
                        summary.push(BindingRow {
                            text: shortcut.display().into(),
                            keys: ModelRc::new(VecModel::from(
                                shortcut
                                    .key_labels()
                                    .into_iter()
                                    .map(Into::into)
                                    .collect::<Vec<slint::SharedString>>(),
                            )),
                            enabled: true,
                        });
                    }
                }
                let row = FunctionBindingRow {
                    id: definition.id.stable_id().into(),
                    label: self.tr(definition.name_key).into(),
                    gestures: ModelRc::new(VecModel::from(gestures)),
                    enabled: config.enabled,
                    shortcuts: ModelRc::new(VecModel::from(shortcuts)),
                };
                rows.push(row);
            }
            ui.global::<BindingUi>()
                .set_rows(ModelRc::new(VecModel::from(rows)));
            ui.set_bindings(ModelRc::new(VecModel::from(summary)));
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
        }
        ui.global::<BindingUi>().set_capture(BindingCaptureState {
            function_id: self
                .capture
                .map(|target| target.id.stable_id())
                .unwrap_or("")
                .into(),
            slot: self.capture.map(|target| target.slot as i32).unwrap_or(-1),
            text: if self.runtime.capture_waiting() {
                self.tr(keys::CAPTURE_RELEASE).into()
            } else if self.runtime.capture_preview.is_empty() {
                self.tr(keys::CAPTURE_RECORDING).into()
            } else {
                self.runtime.capture_preview.clone().into()
            },
            error: self.capture_error.clone().into(),
        });
        if self.previous_test != self.runtime.test {
            self.previous_test = self.runtime.test.clone();
            let message = match &self.runtime.test {
                TestStatus::Idle => None,
                TestStatus::Pending(_) => Some(self.tr(keys::TEST_SENDING).to_owned()),
                TestStatus::Succeeded => Some(self.tr(keys::TEST_SENT).to_owned()),
                TestStatus::Failed(error) => {
                    Some(format!("{}: {error}", self.tr(keys::ERROR_ACTION)))
                }
            };
            if let Some(message) = message {
                self.say(message);
            }
        }
        ui.set_input_count(self.runtime.matched.min(i32::MAX as u64) as i32);
        ui.set_toast(if Instant::now() < self.toast_until {
            self.toast.clone().into()
        } else {
            "".into()
        });
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
