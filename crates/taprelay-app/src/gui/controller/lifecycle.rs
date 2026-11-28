use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[cfg(windows)]
#[derive(Debug)]
pub(super) struct UpdateUserHandoffError;
#[cfg(windows)]
impl std::fmt::Display for UpdateUserHandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Could not continue the update as the installing user")
    }
}

#[cfg(windows)]
impl std::error::Error for UpdateUserHandoffError {}

enum UpdatePhase {
    Save,
    StopBluetooth(Receiver<Result<()>>),
    StopRuntime,
    Handoff(Receiver<Result<()>>),
}

pub(super) struct UpdateExit {
    launch: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
    phase: UpdatePhase,
    was_listening: bool,
}

impl UpdateExit {
    fn advance(&mut self, controller: &mut Controller, ui: &mut Presentation) -> Result<bool> {
        let runtime = &mut controller.runtime;
        match &self.phase {
            UpdatePhase::Save if !runtime.busy() => {
                self.was_listening = runtime.listening;
                controller.save_geometry(ui);
                controller.runtime.config.save(&controller.path)?;
                controller.save_pending = false;
                self.phase = UpdatePhase::StopBluetooth(controller.runtime.stop_for_update()?);
            }
            UpdatePhase::StopBluetooth(stopped) if !runtime.busy() => match stopped.try_recv() {
                Ok(result) => {
                    result?;
                    runtime.shutdown()?;
                    self.phase = UpdatePhase::StopRuntime;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => bail!("Bluetooth shutdown was not confirmed"),
            },
            UpdatePhase::StopRuntime if runtime.exited() => {
                // The worker's final snapshot can contain a newly remembered device.
                controller.save_geometry(ui);
                controller.runtime.config.save(&controller.path)?;
                controller.save_pending = false;
                let launch = self.launch.take().expect("update handoff runs once");
                let (done, result) = mpsc::channel();
                std::thread::Builder::new()
                    .name("taprelay-update-handoff".into())
                    .spawn(move || {
                        let _ = done.send(launch());
                    })?;
                self.phase = UpdatePhase::Handoff(result);
            }
            UpdatePhase::Handoff(result) => match result.try_recv() {
                Ok(result) => return result.map(|_| true),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => bail!("Update handoff stopped unexpectedly"),
            },
            _ => {}
        }
        Ok(false)
    }
}

impl Controller {
    pub(in crate::gui) fn tick(&mut self, ui: &mut Presentation) {
        if self.shutdown {
            return;
        }
        let now = Instant::now();
        if self.exit.is_none() && self.fatal.is_none() {
            self.poll_updates(now);
        }
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
        self.passthrough_feedback(passthrough_notice, ui);
        if self.exit.is_some() {
            let updating = matches!(self.exit, Some(ExitIntent::ApplyUpdate(_)));
            if let Err(error) = self.finish_exit(ui) {
                if updating {
                    self.update_restart_failed(&error);
                } else {
                    self.command_error(&error);
                }
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
        if matches!(self.exit, Some(ExitIntent::ApplyUpdate(_))) {
            let Some(ExitIntent::ApplyUpdate(mut exit)) = self.exit.take() else {
                unreachable!()
            };
            match exit.advance(self, ui) {
                Ok(true) => {
                    self.shutdown = true;
                    ui.window_command = Some(WindowCommand::Quit);
                }
                Ok(false) => self.exit = Some(ExitIntent::ApplyUpdate(exit)),
                Err(error) => {
                    if !matches!(exit.phase, UpdatePhase::Save)
                        && let Err(recovery) = self.runtime.recover_after_update(exit.was_listening)
                    {
                        tracing::error!(%recovery, "Could not restore Bluetooth after interrupted update");
                    }
                    return Err(error);
                }
            }
            return Ok(());
        }
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

    pub(in crate::gui) fn begin_update_restart(
        &mut self,
        prepared: crate::updates::PreparedUpdate,
    ) -> Result<()> {
        self.begin_update_exit(move || prepared.apply())
    }

    #[cfg(windows)]
    pub(in crate::gui) fn begin_user_update(&mut self, tag: &str) -> Result<()> {
        anyhow::ensure!(
            crate::updates::is_stable_tag(tag),
            "Invalid update release tag"
        );
        let tag = tag.to_owned();
        self.begin_update_exit(move || {
            administrator::restart_for_update(&tag).context(UpdateUserHandoffError)
        })
    }

    fn begin_update_exit(
        &mut self,
        launch: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<()> {
        anyhow::ensure!(
            self.exit.is_none() && self.mapping_draft.is_none() && self.group_draft.is_none(),
            HandoffError::Busy
        );
        anyhow::ensure!(
            self.fatal.is_none() && !self.runtime.stopped(),
            HandoffError::Stopped
        );
        self.exit = Some(ExitIntent::ApplyUpdate(UpdateExit {
            launch: Some(Box::new(launch)),
            phase: UpdatePhase::Save,
            was_listening: false,
        }));
        Ok(())
    }

    pub(in crate::gui) fn save_geometry(&mut self, ui: &mut Presentation) {
        self.runtime.config.window = ui.geometry.clone();
        self.save_pending = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        gui::controller::startup_tests::{controller, ready},
    };

    #[test]
    fn update_saves_current_geometry_and_joins_runtime_before_asking_updater_to_restart() {
        let (mut controller, _dir) = controller(ready());
        let mut ui = Presentation::default();
        ui.geometry.x = Some(240);
        ui.geometry.y = Some(160);
        controller.runtime.config.options.auto_check_updates = false;
        let path = controller.path.clone();
        let (launch, launched) = mpsc::channel();
        let (ack, accepted) = mpsc::channel();
        controller
            .begin_update_exit(move || {
                launch.send(Config::load(&path)?)?;
                accepted.recv_timeout(Duration::from_secs(5))?;
                Ok(())
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let saved = loop {
            controller.tick(&mut ui);
            if let Ok(saved) = launched.try_recv() {
                break saved;
            }
            assert!(Instant::now() < deadline, "updater was not called");
            std::thread::yield_now();
        };
        assert!(controller.runtime.exited());
        assert!(!saved.options.auto_check_updates);
        assert_eq!(saved.window, ui.geometry);
        assert!(!controller.shutdown);
        assert_ne!(ui.window_command, Some(WindowCommand::Quit));
        ack.send(()).unwrap();
        while !controller.shutdown {
            controller.tick(&mut ui);
            assert!(Instant::now() < deadline, "handoff did not complete");
            std::thread::yield_now();
        }
        assert_eq!(ui.window_command, Some(WindowCommand::Quit));
    }

    #[test]
    fn update_save_failure_leaves_current_instance_and_unsaved_settings_intact() {
        let (mut controller, dir) = controller(ready());
        let mut ui = Presentation::default();
        ui.geometry.x = Some(200);
        controller.path = dir.path().join("missing").join("config.json");
        controller.runtime.config.options.auto_check_updates = false;
        let (launch, launched) = mpsc::channel();
        controller
            .begin_update_exit(move || Ok(launch.send(())?))
            .unwrap();
        controller.tick(&mut ui);
        assert!(controller.exit.is_none());
        assert!(!controller.runtime.stopped());
        assert!(!controller.shutdown);
        assert!(controller.save_pending);
        assert!(!controller.runtime.config.options.auto_check_updates);
        assert_eq!(controller.runtime.config.window.x, Some(200));
        assert!(matches!(
            launched.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }
}
