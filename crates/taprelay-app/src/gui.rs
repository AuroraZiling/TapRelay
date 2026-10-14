use crate::{
    BindingCaptureState, BindingRow, FunctionBindingRow, GestureAction, ShortcutItem,
    action::{self, Action, BindingCommand, CaptureTarget},
    administrator,
    config::{self, Config, Device, Language, SaveQueue, Theme as ThemeSetting},
    feedback::{TestStatus, WaitWarning},
    i18n::{self, keys},
    logging,
    platform::desktop::{self, Desktop, DesktopEvent},
    runtime::{CaptureError, CaptureKind, CapturePhase},
    runtime_worker::{HandoffError, Notice, RuntimeHandle},
};
use anyhow::{Context, Result, bail};

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
mod custom_mappings;
mod foreground_app_groups;

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
    log_level: logging::LogLevel,
}

fn parse_startup_options(args: &[String]) -> Result<StartupOptions> {
    let mut options = StartupOptions {
        handoff: None,
        privilege: PrivilegeMode::Config,
        log_level: logging::LogLevel::INFO,
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
            "--log-level" => {
                let value = args
                    .get(index + 1)
                    .context("--log-level requires a level")?;
                options.log_level = logging::parse_level(value)?;
                index += 1;
            }
            argument if argument.starts_with("--log-level=") => {
                options.log_level = logging::parse_level(&argument["--log-level=".len()..])?;
            }
            argument => {
                bail!(
                    "Unknown argument `{argument}`. Supported arguments: --user, --admin, --log-level <level>."
                )
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
                log_level: logging::LogLevel::INFO,
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
                log_level: logging::LogLevel::INFO,
            }
        );
    }

    #[test]
    fn rejects_conflicting_or_unknown_arguments() {
        assert!(parse_startup_options(&args(&["--user", "--admin"])).is_err());
        assert!(parse_startup_options(&args(&["--unexpected"])).is_err());
        assert!(parse_startup_options(&args(&["--handoff"])).is_err());
    }

    #[test]
    fn log_level_accepts_both_forms_and_survives_handoff_arguments() {
        for (name, level) in [
            ("off", logging::LogLevel::OFF),
            ("error", logging::LogLevel::ERROR),
            ("warn", logging::LogLevel::WARN),
            ("info", logging::LogLevel::INFO),
            ("debug", logging::LogLevel::DEBUG),
            ("trace", logging::LogLevel::TRACE),
        ] {
            let options = parse_startup_options(&args(&["--user", "--log-level", name])).unwrap();
            assert_eq!(options.log_level, level);
            let options = parse_startup_options(&args(&[
                "--handoff",
                "1234",
                &format!("--log-level={}", name.to_ascii_uppercase()),
            ]))
            .unwrap();
            assert_eq!(options.log_level, level);
            assert_eq!(options.handoff, Some(1234));
        }
    }

    #[test]
    fn invalid_or_missing_log_level_is_rejected() {
        for values in [
            vec!["--log-level"],
            vec!["--log-level="],
            vec!["--log-level", "verbose"],
            vec!["--log-level", "--user"],
        ] {
            assert!(parse_startup_options(&args(&values)).is_err());
        }
    }
}

enum ExitIntent {
    Quit,
    Elevate,
}

enum Shell {
    Native(Desktop),
    #[cfg(test)]
    Headless,
}

impl Shell {
    fn update(&mut self, state: u8, tip: &str, listening: bool, labels: [String; 4]) {
        match self {
            Self::Native(desktop) => desktop.update(state, tip, listening, labels),
            #[cfg(test)]
            Self::Headless => {}
        }
    }
    fn notify(&self, title: &str, body: &str) {
        match self {
            Self::Native(desktop) => desktop.notify(title, body),
            #[cfg(test)]
            Self::Headless => {}
        }
    }
}

struct Controller {
    mapping_draft: Option<custom_mappings::MappingDraft>,
    pending_mappings: Option<(
        Vec<taprelay_core::mapping::CustomMapping>,
        custom_mappings::MappingSave,
    )>,
    mapping_navigation: Option<(Action, String)>,
    mapping_undo: Option<(Vec<taprelay_core::mapping::CustomMapping>, Instant)>,
    last_mapping_capture: u64,
    #[cfg(windows)]
    overlay: crate::passthrough_overlay::Overlay,
    foreground_app_icons: Rc<RefCell<foreground_app_groups::IconCache>>,
    group_draft: Option<foreground_app_groups::GroupDraft>,
    pending_group_rules: Option<taprelay_core::foreground_app::ForegroundAppRules>,
    runtime: RuntimeHandle,
    path: PathBuf,
    logs: Option<tracing_appender::non_blocking::ErrorCounter>,
    desktop: Shell,
    saves: SaveQueue,
    status: administrator::Status,
    fatal: Option<String>,
    passed: bool,
    recovering: bool,
    wait_warning: WaitWarning,
    session_presentation: crate::receiver_view::SessionPresentation,
    started: Instant,
    last_slow: Instant,
    stage_key: String,
    stage_since: Instant,
    toast: String,
    toast_until: Instant,
    exit: Option<ExitIntent>,
    last_error: String,
    last_notification: Instant,
    shutdown: bool,
    hidden: bool,
    startup_pending: bool,
    previous_devices: Option<(
        Vec<Target>,
        Option<String>,
        &'static str,
        taprelay_core::devices::AdapterState,
    )>,
    previous_bindings: Option<(u64, &'static str, usize)>,
    previous_checks: Option<([bool; 4], bool, bool, &'static str)>,
    system_theme: bool,
    system_language: Option<String>,
    last_system_poll: Instant,
    last_ui_sync: Instant,
    last_dropped_logs: usize,
    persisted_remembered_device: Option<Device>,
}
impl Controller {
    fn new(
        runtime: RuntimeHandle,
        path: PathBuf,
        logs: Option<tracing_appender::non_blocking::ErrorCounter>,
        desktop: Shell,
        status: administrator::Status,
        fatal: Option<String>,
    ) -> Result<Self> {
        let persisted_remembered_device = runtime.config.remembered_device.clone();
        let now = Instant::now();
        Ok(Self {
            mapping_draft: None,
            pending_mappings: None,
            mapping_navigation: None,
            mapping_undo: None,
            last_mapping_capture: 0,
            #[cfg(windows)]
            overlay: crate::passthrough_overlay::Overlay::default(),
            group_draft: None,
            foreground_app_icons: Rc::new(RefCell::new(foreground_app_groups::IconCache::new())),
            pending_group_rules: None,
            runtime,
            path,
            logs,
            desktop,
            saves: SaveQueue::default(),
            status,
            fatal,
            passed: false,
            recovering: false,
            wait_warning: WaitWarning::default(),
            session_presentation: crate::receiver_view::SessionPresentation::default(),
            started: now,
            last_slow: now - Duration::from_secs(2),
            stage_key: String::new(),
            stage_since: now,
            toast: String::new(),
            toast_until: now,
            exit: None,
            last_error: String::new(),
            last_notification: now - Duration::from_secs(120),
            shutdown: false,
            hidden: true,
            startup_pending: true,
            previous_devices: None,
            previous_bindings: None,
            previous_checks: None,
            system_theme: desktop::system_dark(),
            system_language: desktop::system_locale_id(),
            last_system_poll: now,
            last_ui_sync: now,
            last_dropped_logs: 0,
            persisted_remembered_device,
        })
    }
    fn locale(&self) -> &'static str {
        match &self.runtime.config.options.language {
            Language::System => self
                .system_language
                .as_deref()
                .and_then(i18n::resolve)
                .unwrap_or(i18n::locale::SOURCE),
            selected => selected.locale(),
        }
    }
    fn tr(&self, key: &str) -> String {
        rust_i18n::t!(key, locale = self.locale()).into_owned()
    }
    fn say(&mut self, message: String) {
        self.toast = message;
        self.toast_until = Instant::now() + Duration::from_secs(6);
    }
    fn command_error(&mut self, error: &anyhow::Error) {
        let message = match error.downcast_ref::<HandoffError>() {
            Some(HandoffError::Busy) => self.tr(keys::RUNTIME_BUSY),
            Some(HandoffError::Stopped) => self.tr(keys::RUNTIME_STOPPED),
            Some(HandoffError::QueueFull) => self.tr(keys::RUNTIME_QUEUE_FULL),
            None => format!("{error:#}"),
        };
        self.error(&message);
    }
    fn error(&mut self, message: &str) {
        tracing::error!("{message}");
        self.say(self.tr(keys::ERROR_OPERATION));
        self.notify_error(&self.tr(keys::ERROR_OPERATION));
    }
    fn notify_error(&mut self, summary: &str) {
        if self.runtime.config.options.notifications
            && self.last_notification.elapsed() > Duration::from_secs(60)
        {
            self.desktop.notify("TapRelay", summary);
            self.last_notification = Instant::now();
        }
    }
    fn flush(&mut self, force: bool) {
        // Do not race the worker's atomic group save with an older UI snapshot.
        if self.fatal.is_some()
            || self.runtime.saving_foreground_app_rules()
            || self.runtime.saving_custom_mappings()
        {
            return;
        }
        self.track_remembered_device();
        if self.runtime.take_bindings_changed() {
            self.saves.changed();
        }
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
        self.runtime.capture().is_some()
    }
    fn cancel_capture(&mut self) -> Result<()> {
        self.runtime
            .apply_binding_command(BindingCommand::CancelCapture)
    }
    fn binding_action(&mut self, ui: &mut Presentation, command: BindingCommand) -> Result<()> {
        if self.exit.is_some() {
            return Ok(());
        }
        let result = self.runtime.apply_binding_command(command);
        self.sync(ui);
        result
    }
    fn action(&mut self, ui: &mut Presentation, name: Action, value: &str) -> Result<()> {
        self.runtime.poll();
        if self.exit.is_some() && !matches!(name, Action::Show) {
            return Ok(());
        }
        if !matches!(name, Action::Show | Action::Resume) {
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
            )
        {
            return Ok(());
        }
        if self.mapping_draft.is_some()
            && matches!(
                name,
                Action::Navigate
                    | Action::EditMapping
                    | Action::Close
                    | Action::Quit
                    | Action::Wizard
                    | Action::Elevate
            )
            && !self.mapping_leave(ui, Some((name, value.to_owned())))?
        {
            return Ok(());
        }
        match name {
            Action::EditMapping => {
                self.mapping_command(ui, "edit", value)?;
            }
            Action::Show => {
                self.show_window(ui)?;
            }
            Action::Close => {
                if self.runtime.config.options.close_to_tray && self.fatal.is_none() {
                    self.cancel_capture()?;
                    self.save_geometry(ui);
                    self.flush(true);
                    if !self.runtime.config.options.close_hint_seen {
                        self.desktop
                            .notify("TapRelay", &self.tr(keys::TRAY_CLOSE_HINT));
                        self.runtime.config.options.close_hint_seen = true;
                        self.saves.changed();
                    }
                    ui.window_command = Some(WindowCommand::Hide);
                    self.hidden = true;
                } else {
                    self.action(ui, Action::Quit, "")?;
                }
            }
            Action::Quit => {
                self.runtime.shutdown()?;
                self.exit = Some(ExitIntent::Quit);
            }
            Action::Navigate => {
                if !self.runtime.stopped() {
                    self.cancel_capture()?;
                }
                ui.mapping.open = false;
                ui.page = action::page(value)?.clamp(0, 4);
                ui.groups.error = "".into();
                ui.groups.confirming_delete = false;
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
                if name == Action::Resume {
                    self.runtime.resume()?;
                } else {
                    self.runtime.start_bluetooth()?;
                }
                self.recovering = true;
                self.started = Instant::now();
                self.stage_since = Instant::now();
                self.last_error.clear();
            }
            Action::BluetoothSettings => self.runtime.bluetooth_settings()?,
            Action::Repository => desktop::open("https://github.com/AuroraZiling/TapRelay")?,
            Action::License => {
                desktop::open("https://github.com/AuroraZiling/TapRelay/blob/main/LICENSE")?
            }
            Action::ThirdPartyNotices => desktop::open(
                "https://github.com/AuroraZiling/TapRelay/blob/main/THIRD_PARTY_NOTICES.md",
            )?,
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
                ui.mode = 1;
                ui.wizard_page = 0;
                self.runtime.config.wizard.dismissed = false;
                self.runtime.config.wizard.page = 0;
                self.saves.changed();
            }
            Action::WizardNext | Action::WizardBack => {
                if name == Action::WizardNext
                    && ui.wizard_page == 2
                    && !taprelay_core::devices::receiver_next_allowed(&self.runtime.state)
                {
                    return Ok(());
                }
                let page =
                    (ui.wizard_page + if name == Action::WizardNext { 1 } else { -1 }).clamp(0, 3);
                // Preserve step IDs saved by the former four-step wizard.
                let page = if page == 1 {
                    if name == Action::WizardNext { 2 } else { 0 }
                } else {
                    page
                };
                ui.wizard_page = page;
                self.runtime.config.wizard.page = page as u8;
                self.saves.changed();
            }
            Action::WizardFinish => {
                self.cancel_capture()?;
                self.runtime.config.wizard.dismissed = true;
                self.saves.changed();
                ui.mode = 2;
                ui.page = 0;
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
            Action::SetPassthroughOverlay => {
                self.runtime.config.options.passthrough_overlay = action::toggle(value)?;
                self.saves.changed();
            }
            Action::SetPassthroughReverseScroll => {
                self.runtime
                    .set_passthrough_reverse_scroll(action::toggle(value)?)?;
                self.saves.changed();
            }
            Action::SetPassthroughMouseReportRate => {
                self.runtime
                    .set_passthrough_mouse_report_rate(value.parse()?)?;
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
                self.cancel_capture()?;
                self.exit = Some(ExitIntent::Elevate);
            }
        }
        self.sync(ui);
        Ok(())
    }
    fn tick(&mut self, ui: &mut Presentation) {
        if self.shutdown {
            return;
        }
        let now = Instant::now();
        if self.fatal.is_none() || self.exit.is_some() {
            self.runtime.tick();
        } else {
            self.runtime.poll();
        }
        self.track_remembered_device();
        let mut passthrough_notice = None;
        while let Some(notice) = self.runtime.take_notice() {
            match notice {
                Notice::Error(error) => self.error(&error),
                Notice::Test(TestStatus::Succeeded) => self.say(self.tr(keys::TEST_SENT)),
                Notice::Test(TestStatus::Failed(error)) => self.error(&error),
                Notice::Test(_) => {}
                Notice::Passthrough(notice) => passthrough_notice = Some(notice),
            }
        }
        #[cfg(windows)]
        {
            self.overlay.update(
                passthrough_notice,
                self.runtime.config.options.passthrough_overlay && self.exit.is_none(),
                self.locale(),
                ui.overlay_background,
                ui.overlay_foreground,
            );
        }
        #[cfg(not(windows))]
        let _ = passthrough_notice;
        if self.exit.is_some() {
            if let Err(error) = self.finish_exit(ui) {
                self.command_error(&error);
                self.exit = None;
            }
            self.sync(ui);
            return;
        }
        let state = &self.runtime.state;
        let passed = state.adapter && state.peripheral && state.service && state.broadcasting;
        if passed || state.last_error.is_some() {
            self.recovering = false;
        }
        if !self.passed && passed && self.fatal.is_none() {
            self.passed = true;
            ui.mode = if self.runtime.config.wizard.dismissed {
                2
            } else {
                1
            };
            ui.wizard_page = self.runtime.config.wizard.page.into();
            if self.runtime.config.options.auto_listen
                && let Err(e) = self.runtime.set_listening(true)
            {
                self.error(&e.to_string());
            }
            if self.runtime.config.options.start_hidden && self.runtime.config.wizard.dismissed {
                ui.window_command = Some(WindowCommand::Hide);
                self.hidden = true;
            }
        }
        if let Some(e) = &self.runtime.state.last_error
            && self.last_error != *e
        {
            let e = e.clone();
            self.last_error = e.clone();
            if self.passed
                && !self.runtime.state.profile_switching
                && (!self.runtime.state.adapter
                    || (!self.runtime.state.service && !self.runtime.state.service_paused))
            {
                tracing::error!("{e}");
                self.notify_error(
                    &self
                        .tr(crate::feedback::ErrorScope::Connection
                            .summary_key(&self.runtime.state)),
                );
            }
        }
        let warning_enabled = self.passed
            && self.runtime.config.options.connection_wait_warning
            && self.runtime.config.options.notifications
            && self.runtime.config.wizard.dismissed;
        if self.wait_warning.update(
            self.runtime
                .state
                .selected
                .as_deref()
                .filter(|_| !self.runtime.state.hid_suspended),
            self.runtime.state.ready,
            warning_enabled,
            now,
        ) {
            let message = self.tr(keys::RECEIVER_TIMEOUT);
            self.say(message.clone());
            self.notify_error(&message);
        }
        self.flush(false);
        if let Err(error) = self.present_startup(ui) {
            self.error(&format!("{error:#}"));
        }
        if !self.hidden || self.last_ui_sync.elapsed() >= Duration::from_secs(1) {
            self.sync(ui);
            self.last_ui_sync = now;
        }
        if self.last_slow.elapsed() >= Duration::from_secs(1) {
            self.last_slow = now;
            let tip = format!(
                "TapRelay · {} · {}",
                ui.device_name.clone(),
                ui.stage.clone()
            );
            self.desktop.update(
                ui.state_color as u8,
                &tip,
                self.runtime.listening,
                i18n::tray_labels(self.locale()),
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
    fn show_window(&mut self, ui: &mut Presentation) -> Result<()> {
        ui.window_command = Some(WindowCommand::Show);
        self.hidden = false;
        self.startup_pending = false;
        Ok(())
    }

    fn present_startup(&mut self, ui: &mut Presentation) -> Result<()> {
        if !self.startup_pending {
            return Ok(());
        }
        self.sync(ui);
        if !crate::startup_checks::should_present(
            self.passed,
            !ui.problem.clone().is_empty(),
            self.started.elapsed(),
        ) {
            return Ok(());
        }
        if self.passed
            && self.runtime.config.options.start_hidden
            && self.runtime.config.wizard.dismissed
        {
            self.startup_pending = false;
            return Ok(());
        }
        self.show_window(ui)
    }

    fn sync(&mut self, ui: &mut Presentation) {
        if self.last_system_poll.elapsed() >= Duration::from_secs(1) {
            self.system_theme = desktop::system_dark();
            self.system_language = desktop::system_locale_id();
            self.last_system_poll = Instant::now();
        }
        let locale = self.locale();
        ui.locale = locale.to_owned();
        self.sync_group_operation(ui);
        self.sync_mapping_operation(ui);
        let icons_changed = self.foreground_app_icons.borrow_mut().poll();
        if icons_changed {
            ui.groups.icon_revision = ui.groups.icon_revision.wrapping_add(1);
        }
        ui.groups.busy = self.runtime.busy() || self.runtime.stopped() || self.recording();
        let o = &self.runtime.config.options;
        let dark = match o.theme {
            ThemeSetting::System => self.system_theme,
            ThemeSetting::Dark => true,
            ThemeSetting::Light => false,
        };
        ui.dark = dark;
        ui.theme_choice = match o.theme {
            ThemeSetting::System => 0,
            ThemeSetting::Light => 1,
            ThemeSetting::Dark => 2,
        };
        ui.language_options = i18n::language_options(locale);
        ui.language_choice = match o.language {
            Language::System => 0,
            ref selected => {
                1 + i18n::locale::all()
                    .iter()
                    .position(|compiled| compiled == selected.locale())
                    .unwrap_or_default() as i32
            }
        };
        ui.auto_start = o.autostart;
        ui.auto_listen = o.auto_listen;
        ui.passthrough_overlay = o.passthrough_overlay;
        ui.passthrough_reverse_scroll = o.passthrough_reverse_scroll;
        ui.passthrough_mouse_report_rate = i32::from(o.passthrough_mouse_report_rate);
        ui.start_hidden = o.start_hidden;
        ui.close_to_tray = o.close_to_tray;
        ui.always_admin = o.always_admin;
        ui.notifications = o.notifications;
        ui.connection_wait_warning = o.connection_wait_warning;
        ui.elevated = self.status.elevated;
        ui.app_version = crate::version::VERSION.into();
        ui.data_directory = self
            .path
            .parent()
            .unwrap()
            .to_string_lossy()
            .as_ref()
            .into();
        let s = &self.runtime.state;
        let flags = [s.adapter, s.peripheral, s.service, s.broadcasting];
        ui.bluetooth_available = s.adapter_state == taprelay_core::devices::AdapterState::Available;
        ui.scanning = s.discovery == taprelay_core::devices::DiscoveryState::Scanning;
        ui.adapter_label = self.tr(crate::receiver_view::adapter_label_key(s.adapter_state));
        ui.receiver_next_allowed = taprelay_core::devices::receiver_next_allowed(s);
        ui.device_selected = s.selected.is_some();
        ui.discovery_status = self.tr(crate::receiver_view::page_status_key(s));
        let timeout = !self.passed && self.started.elapsed() > Duration::from_secs(30);
        let scope = if self.fatal.is_some() {
            crate::feedback::ErrorScope::Startup
        } else if self.runtime.stopped() {
            crate::feedback::ErrorScope::Runtime
        } else {
            crate::feedback::ErrorScope::Connection
        };
        let failure = self
            .fatal
            .clone()
            .or_else(|| {
                self.runtime
                    .stopped()
                    .then(|| self.tr(keys::RUNTIME_STOPPED))
            })
            .or_else(|| s.last_error.clone())
            .or_else(|| {
                (self.passed
                    && !self.recovering
                    && !s.profile_switching
                    && (flags[..2].iter().any(|v| !*v)
                        || (!s.service_paused && flags[2..].iter().any(|v| !*v))))
                .then(|| self.tr(keys::RECEIVER_UNAVAILABLE).to_owned())
            })
            .or_else(|| timeout.then(|| self.tr(keys::STARTUP_TIMEOUT)));
        if self.fatal.is_some() {
            ui.mode = 3;
        }
        ui.problem = if failure.is_some() {
            self.tr(scope.summary_key(s))
        } else {
            "".into()
        };
        ui.problem_retry_allowed = scope.can_retry() && !self.runtime.stopped();
        ui.problem_busy = self.runtime.busy() || self.recovering;
        ui.busy = (!self.passed || self.recovering) && failure.is_none();
        let checks_key = (flags, self.fatal.is_some(), failure.is_some(), locale);
        if self.previous_checks != Some(checks_key) {
            self.previous_checks = Some(checks_key);
            ui.checks =
                crate::startup_checks::rows(locale, self.fatal.is_some(), flags, failure.is_some());
        }
        let target = s.selected_target();
        let (key, stage) = if s.profile_switching && failure.is_none() {
            ("switching", self.tr(keys::RECEIVER_SWITCHING))
        } else if self.recovering && failure.is_none() {
            ("prepare", self.tr(keys::RECEIVER_PREPARING))
        } else if self.fatal.is_some()
            || self.runtime.stopped()
            || (self.passed
                && (!s.adapter
                    || (!s.service_paused
                        && !s.profile_switching
                        && (!s.service || !s.broadcasting))))
        {
            ("fault", self.tr(keys::RECEIVER_FAULT))
        } else if s.activity == TransportActivity::ResolvingDevice {
            ("identity", self.tr(keys::RECEIVER_RESOLVING))
        } else if s.selected.is_none() {
            ("unselected", self.tr(keys::RECEIVER_CHOOSE))
        } else {
            let status = self.session_presentation.key(s, Instant::now());
            (status, self.tr(status))
        };
        let has_bindings = self
            .runtime
            .config
            .functions
            .values()
            .any(|function| function.enabled && !function.shortcuts.is_empty())
            || self
                .runtime
                .config
                .custom_mappings
                .iter()
                .any(|mapping| mapping.enabled);
        let (display_key, signal) =
            crate::receiver_view::relay_presentation(key, self.runtime.listening, has_bindings);
        let stage = if display_key != key {
            self.tr(display_key)
        } else {
            stage
        }
        .to_owned();
        let key = display_key;
        if self.stage_key != key {
            tracing::info!(previous=%self.stage_key,elapsed_ms=self.stage_since.elapsed().as_millis(),next=key,"Connection stage changed");
            self.stage_key = key.into();
            self.stage_since = Instant::now();
        }
        ui.stage = stage;
        ui.ready = s.ready && !s.profile_switching && key != keys::RECEIVER_SUSPENDED;
        ui.listening = self.runtime.listening;
        ui.state_color = if (!self.passed && failure.is_some()) || key == "fault" {
            3
        } else {
            signal
        };
        let receiver_none = self.tr(keys::RECEIVER_NONE);
        ui.device_name = s
            .selected_target()
            .map(|t| t.name.as_str())
            .unwrap_or(&receiver_none)
            .into();
        let diagnostics = format!(
            "Adapter={} · Peripheral={} · Service={} · Advertising={}\nLink={:?} · Subscription={:?} · Input={} · HID={:?}\n{}: {}",
            s.adapter,
            s.peripheral,
            s.service,
            s.broadcasting,
            target.map(|t| t.link),
            target.map(|t| t.subscribed),
            s.input,
            s.hid_profile,
            self.tr(keys::DIAGNOSTICS_INPUTS),
            self.runtime.matched
        );
        ui.diagnostics = diagnostics;
        let keyboard_layout = crate::platform::keyboard_layout();
        if self.previous_bindings != Some((self.runtime.bindings_revision, locale, keyboard_layout))
        {
            self.previous_bindings =
                Some((self.runtime.bindings_revision, locale, keyboard_layout));
            self.sync_foreground_app_groups(ui);
            self.sync_mapping_rows(ui);
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
                    gesture: self.tr(keys::BINDINGS_GESTURE_PRESS),
                    action: self.tr(definition.tap_action.name_key()),
                }];
                if let Some(hold) = definition.hold_action {
                    gestures.push(GestureAction {
                        gesture: self.tr(keys::BINDINGS_GESTURE_HOLD),
                        action: self.tr(hold.name_key()),
                    });
                }
                if definition.repeats() {
                    gestures.push(GestureAction {
                        gesture: self.tr(keys::BINDINGS_GESTURE_HOLD),
                        action: self.tr(keys::BINDINGS_GESTURE_REPEAT),
                    });
                }
                let shortcuts = config
                    .shortcuts
                    .iter()
                    .enumerate()
                    .map(|(slot, shortcut)| {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        ShortcutItem {
                            text: keys.join("+"),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            slot: slot as i32,
                            enabled: config.enabled,
                        }
                    })
                    .collect::<Vec<_>>();
                if config.enabled {
                    for shortcut in &config.shortcuts {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        summary.push(BindingRow {
                            text: keys.join("+"),
                            function_name: self.tr(definition.name_key),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            enabled: true,
                        });
                    }
                }
                let row = FunctionBindingRow {
                    scope_label: self
                        .runtime
                        .config
                        .foreground_app_rules
                        .assignments
                        .get(&definition.id)
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| {
                                    self.runtime.config.foreground_app_rules.groups.get(id)
                                })
                                .map(|group| group.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .unwrap_or_else(|| self.tr(keys::COMMON_ALL_APPS)),
                    scope_editable: definition.category == function::CategoryId::Media,
                    id: definition.id.stable_id().into(),
                    label: self.tr(definition.name_key),
                    gestures,
                    enabled: config.enabled,
                    shortcuts,
                };
                rows.push(row);
            }
            ui.binding.rows = rows;
            for mapping in &self.runtime.config.custom_mappings {
                if mapping.enabled {
                    for shortcut in &mapping.shortcuts {
                        let keys = i18n::shortcut_labels(self.locale(), shortcut);
                        summary.push(BindingRow {
                            text: keys.join("+"),
                            function_name: mapping.name.clone(),
                            keys: keys.into_iter().collect::<Vec<String>>(),
                            enabled: true,
                        });
                    }
                }
            }
            ui.bindings = summary;
        }
        if self
            .previous_devices
            .as_ref()
            .is_none_or(|(targets, selected, previous, adapter)| {
                targets != &s.targets
                    || selected != &s.selected
                    || *previous != locale
                    || *adapter != s.adapter_state
            })
        {
            self.previous_devices = Some((
                s.targets.clone(),
                s.selected.clone(),
                locale,
                s.adapter_state,
            ));
            let rows: Vec<_> = s
                .targets
                .iter()
                .map(|d| crate::receiver_view::row(d, s, |key| self.tr(key)))
                .collect();
            ui.paired_devices = rows
                .iter()
                .filter(|r| r.paired)
                .cloned()
                .collect::<Vec<_>>();
        }
        let capture = self
            .runtime
            .capture()
            .filter(|capture| matches!(capture.kind, CaptureKind::Function(_)));
        let target = capture.and_then(|session| match session.kind {
            CaptureKind::Function(target) => Some(target),
            _ => None,
        });
        ui.binding.capture = BindingCaptureState {
            function_id: target
                .map(|target| target.id.stable_id())
                .unwrap_or("")
                .into(),
            slot: target.map_or(-1, |target| target.slot as i32),
            text: match capture {
                Some(session) if session.phase == CapturePhase::WaitingForRelease => {
                    self.tr(keys::CAPTURE_RELEASE)
                }
                Some(session) if !session.preview.is_empty() => {
                    i18n::capture_preview(self.locale(), session)
                }
                _ => self.tr(keys::CAPTURE_RECORDING),
            },
            error: match capture.and_then(|session| session.error.as_ref()) {
                Some(CaptureError::Invalid) => self.tr(keys::CAPTURE_INVALID),
                Some(CaptureError::Duplicate(id)) => format!(
                    "{}: {}",
                    self.tr(keys::CAPTURE_DUPLICATE),
                    self.tr(function::function_definition(*id).name_key)
                ),
                Some(CaptureError::Commit(error)) => error.clone(),
                Some(CaptureError::DuplicateMapping(name)) => {
                    format!("{}: {}", self.tr(keys::CAPTURE_DUPLICATE), name)
                }
                None => String::new(),
            },
        };
        ui.input_count = self.runtime.matched;
        ui.toast = if self.runtime.slow() {
            self.tr(keys::RUNTIME_SLOW)
        } else if self.exit.is_some() {
            self.tr(keys::RUNTIME_FINISHING)
        } else if self.runtime.progress_pending() {
            self.tr(keys::RUNTIME_BUSY)
        } else if matches!(self.runtime.test, TestStatus::Pending(_)) {
            self.tr(keys::TEST_SENDING)
        } else if Instant::now() < self.toast_until {
            self.toast.clone()
        } else {
            "".into()
        };
    }
    fn finish_exit(&mut self, ui: &mut Presentation) -> Result<()> {
        match self.exit {
            Some(ExitIntent::Quit) if self.runtime.stopped() => {
                self.save_geometry(ui);
                self.flush(true);
                self.shutdown = true;
                ui.window_command = Some(WindowCommand::Quit);
            }
            Some(ExitIntent::Elevate) if !self.runtime.busy() => {
                self.save_geometry(ui);
                self.runtime.config.save(&self.path)?;
                self.exit = None;
                if !self.status.elevated && administrator::restart()? {
                    self.runtime.shutdown()?;
                    self.exit = Some(ExitIntent::Quit);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn save_geometry(&mut self, ui: &mut Presentation) {
        self.runtime.config.window = ui.geometry.clone();
        self.saves.changed();
    }
}

mod bootstrap;
pub(crate) mod presentation;
mod ripple;
mod theme;
mod view;
pub use bootstrap::run;
use presentation::{Presentation, WindowCommand};
