use super::*;

impl Controller {
    pub(in crate::gui) fn tick(&mut self, ui: &mut Presentation) {
        if self.shutdown {
            return;
        }
        let now = Instant::now();
        if self.fatal.is_none() || self.exit.is_some() {
            self.runtime.tick();
        } else {
            self.runtime.poll();
        }
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
        if self.exit.is_none()
            && let Some(notice) = &passthrough_notice
        {
            self.passthrough_feedback(notice);
        }
        {
            self.overlay.update(
                passthrough_notice,
                self.runtime.config.options.passthrough_overlay && self.exit.is_none(),
                self.locale(),
                ui.overlay_background,
                ui.overlay_foreground,
            );
        }
        if self.exit.is_some() {
            if let Err(error) = self.finish_exit(ui) {
                self.command_error(&error);
                self.exit = None;
            }
            self.sync(ui);
            return;
        }
        let state = &self.runtime.state;
        // macOS permissions may be denied or awaiting a system prompt. Keep
        // configuration accessible while reporting transport readiness in the
        // receiver page
        let passed = cfg!(target_os = "macos")
            || (state.adapter && state.peripheral && state.service && state.broadcasting);
        if passed || state.last_error.is_some() {
            self.recovering = false;
        }
        if !self.passed
            && passed
            && self.fatal.is_none()
            && !self.runtime.stopped()
            && (cfg!(target_os = "macos") || state.last_error.is_none())
        {
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
                self.notify_error(&self.tr(
                    crate::gui::feedback::ErrorScope::Connection.summary_key(&self.runtime.state),
                ));
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
        self.save_config();
        if let Err(error) = self.present_startup(ui) {
            self.error(&format!("{error:#}"));
        }
        if !self.hidden || self.last_ui_sync.elapsed() >= Duration::from_secs(1) {
            self.sync(ui);
            self.last_ui_sync = now;
        }
        if self.last_slow.elapsed() >= Duration::from_secs(1) {
            self.last_slow = now;
            let tip = rust_i18n::t!(
                keys::TRAY_TOOLTIP,
                locale = self.locale(),
                device = &ui.device_name,
                status = &ui.stage
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
        }
    }

    pub(in crate::gui) fn show_window(&mut self, ui: &mut Presentation) -> Result<()> {
        ui.window_command = Some(WindowCommand::Show);
        self.hidden = false;
        self.startup_pending = false;
        Ok(())
    }

    pub(in crate::gui) fn present_startup(&mut self, ui: &mut Presentation) -> Result<()> {
        if !self.startup_pending {
            return Ok(());
        }
        self.sync(ui);
        match self.startup_state() {
            StartupState::Checking => Ok(()),
            StartupState::Ready
                if self.runtime.config.options.start_hidden
                    && self.runtime.config.wizard.dismissed =>
            {
                self.startup_pending = false;
                Ok(())
            }
            StartupState::Ready | StartupState::Failed => self.show_window(ui),
        }
    }

    pub(in crate::gui) fn startup_state(&self) -> StartupState {
        if self.failure().is_some() {
            StartupState::Failed
        } else if self.passed {
            StartupState::Ready
        } else {
            StartupState::Checking
        }
    }

    pub(in crate::gui) fn failure(&self) -> Option<String> {
        let state = &self.runtime.state;
        self.fatal
            .clone()
            .or_else(|| {
                self.runtime
                    .stopped()
                    .then(|| self.tr(keys::RUNTIME_STOPPED))
            })
            .or_else(|| state.last_error.clone())
            .or_else(|| {
                (self.passed
                    && !self.recovering
                    && !state.profile_switching
                    && (!state.adapter
                        || !state.peripheral
                        || (!state.service_paused && (!state.service || !state.broadcasting))))
                    .then(|| self.tr(keys::RECEIVER_UNAVAILABLE))
            })
            .or_else(|| {
                (!self.passed && self.started.elapsed() > Duration::from_secs(30))
                    .then(|| self.tr(keys::STARTUP_TIMEOUT))
            })
    }

    pub(in crate::gui) fn finish_exit(&mut self, ui: &mut Presentation) -> Result<()> {
        match self.exit {
            Some(ExitIntent::Quit) if self.runtime.stopped() => {
                self.save_geometry(ui);
                self.save_config();
                self.shutdown = true;
                ui.window_command = Some(WindowCommand::Quit);
            }
            #[cfg(windows)]
            Some(ExitIntent::Elevate) if !self.runtime.busy() => {
                self.save_geometry(ui);
                self.runtime.config.save(&self.path)?;
                self.save_pending = false;
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

    pub(in crate::gui) fn save_geometry(&mut self, ui: &mut Presentation) {
        self.runtime.config.window = ui.geometry.clone();
        self.save_pending = true;
    }
}
