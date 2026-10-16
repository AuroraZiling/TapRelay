use crate::{
    config::{Device, Language, Theme as ThemeSetting},
    gui::action::{self, Action},
    gui::feedback::WaitWarning,
    gui::presentation::startup::State as StartupState,
    gui::presentation::{
        BindingCaptureState, BindingRow, FunctionBindingRow, GestureAction, Presentation,
        ShortcutItem, WindowCommand,
    },
    i18n::{self, keys},
    logging,
    platform::administrator,
    platform::desktop::{self, Desktop},
    runtime::TestStatus,
    runtime::commands::BindingCommand,
    runtime::worker::{HandoffError, Notice, RuntimeHandle},
    runtime::{CaptureError, CaptureKind, CapturePhase},
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

mod actions;
mod desktop_shell;
mod groups;
mod lifecycle;
mod mappings;
#[cfg(test)]
mod startup_tests;
mod sync;

pub(super) use desktop_shell::Shell;

enum ExitIntent {
    Quit,
    Elevate,
}

pub(super) struct Controller {
    pub(super) mapping_draft: Option<mappings::MappingDraft>,
    pending_mappings: Option<(
        Vec<taprelay_core::mapping::CustomMapping>,
        mappings::MappingSave,
    )>,
    mapping_navigation: Option<(Action, String)>,
    mapping_undo: Option<(Vec<taprelay_core::mapping::CustomMapping>, Instant)>,
    last_mapping_capture: u64,
    #[cfg(windows)]
    pub(super) overlay: crate::gui::overlay::Overlay,
    pub(super) foreground_app_icons: Rc<RefCell<groups::IconCache>>,
    group_draft: Option<groups::GroupDraft>,
    pending_group_rules: Option<taprelay_core::foreground_app::ForegroundAppRules>,
    pub(super) runtime: RuntimeHandle,
    pub(super) path: PathBuf,
    logs: Option<tracing_appender::non_blocking::ErrorCounter>,
    pub(super) desktop: Shell,
    pub(super) save_pending: bool,
    status: administrator::Status,
    pub(super) fatal: Option<String>,
    pub(super) passed: bool,
    recovering: bool,
    wait_warning: WaitWarning,
    session_presentation: crate::gui::presentation::receiver::SessionPresentation,
    pub(super) started: Instant,
    last_slow: Instant,
    stage_key: String,
    stage_since: Instant,
    pub(super) pending_notification: Option<String>,
    exit: Option<ExitIntent>,
    last_error: String,
    last_notification: Instant,
    shutdown: bool,
    pub(super) hidden: bool,
    pub(super) startup_pending: bool,
    previous_devices: Option<(
        Vec<Target>,
        Option<String>,
        &'static str,
        taprelay_core::devices::AdapterState,
    )>,
    previous_bindings: Option<(u64, &'static str, usize)>,
    previous_checks: Option<([bool; 4], bool, bool, &'static str)>,
    pub(super) system_theme: bool,
    system_language: Option<String>,
    pub(super) last_system_poll: Instant,
    last_ui_sync: Instant,
    last_dropped_logs: usize,
    last_remembered_device: Option<Device>,
}
impl Controller {
    pub(super) fn new(
        runtime: RuntimeHandle,
        path: PathBuf,
        logs: Option<tracing_appender::non_blocking::ErrorCounter>,
        desktop: Shell,
        status: administrator::Status,
        fatal: Option<String>,
    ) -> Result<Self> {
        let last_remembered_device = runtime.config.remembered_device.clone();
        let now = Instant::now();
        Ok(Self {
            mapping_draft: None,
            pending_mappings: None,
            mapping_navigation: None,
            mapping_undo: None,
            last_mapping_capture: 0,
            #[cfg(windows)]
            overlay: crate::gui::overlay::Overlay::default(),
            group_draft: None,
            foreground_app_icons: Rc::new(RefCell::new(groups::IconCache::new())),
            pending_group_rules: None,
            runtime,
            path,
            logs,
            desktop,
            save_pending: false,
            status,
            fatal,
            passed: false,
            recovering: false,
            wait_warning: WaitWarning::default(),
            session_presentation: crate::gui::presentation::receiver::SessionPresentation::default(
            ),
            started: now,
            last_slow: now - Duration::from_secs(2),
            stage_key: String::new(),
            stage_since: now,
            pending_notification: None,
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
            last_remembered_device,
        })
    }

    pub(super) fn locale(&self) -> &'static str {
        match &self.runtime.config.options.language {
            Language::System => self
                .system_language
                .as_deref()
                .and_then(i18n::resolve)
                .unwrap_or(i18n::locale::SOURCE),
            selected => selected.locale(),
        }
    }

    pub(super) fn tr(&self, key: &str) -> String {
        rust_i18n::t!(key, locale = self.locale()).into_owned()
    }

    pub(super) fn say(&mut self, message: String) {
        self.pending_notification = Some(message);
    }

    pub(super) fn passthrough_feedback(&mut self, notice: &crate::runtime::passthrough::Notice) {
        let (prefix, device, suffix) = notice.text_parts(self.locale());
        let message = format!("{prefix}{device}{suffix}");
        if self.runtime.config.options.notifications {
            self.desktop.notify("TapRelay", &message);
        }
        self.say(message);
    }

    pub(super) fn command_error(&mut self, error: &anyhow::Error) {
        let message = match error.downcast_ref::<HandoffError>() {
            Some(HandoffError::Busy) => self.tr(keys::RUNTIME_BUSY),
            Some(HandoffError::Stopped) => self.tr(keys::RUNTIME_STOPPED),
            Some(HandoffError::QueueFull) => self.tr(keys::RUNTIME_QUEUE_FULL),
            None => format!("{error:#}"),
        };
        self.error(&message);
    }

    pub(super) fn error(&mut self, message: &str) {
        tracing::error!("{message}");
        self.say(self.tr(keys::ERROR_OPERATION));
        self.notify_error(&self.tr(keys::ERROR_OPERATION));
    }

    pub(super) fn notify_error(&mut self, summary: &str) {
        if self.runtime.config.options.notifications
            && self.last_notification.elapsed() > Duration::from_secs(60)
        {
            self.desktop.notify("TapRelay", summary);
            self.last_notification = Instant::now();
        }
    }

    pub(super) fn save_config(&mut self) {
        // Do not race the worker's atomic group save with an older UI snapshot.
        if self.fatal.is_some()
            || self.runtime.saving_foreground_app_rules()
            || self.runtime.saving_custom_mappings()
        {
            return;
        }
        if self.last_remembered_device != self.runtime.config.remembered_device {
            self.last_remembered_device = self.runtime.config.remembered_device.clone();
            self.save_pending = true;
        }
        self.save_pending |= self.runtime.take_bindings_changed();
        if std::mem::take(&mut self.save_pending)
            && let Err(e) = self.runtime.config.save(&self.path)
        {
            self.error(&format!("{}: {e:#}", self.tr(keys::ERROR_SAVE)));
        }
    }
}
