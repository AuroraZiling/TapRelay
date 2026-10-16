use super::*;

impl Controller {
    pub(in crate::gui) fn recording(&self) -> bool {
        self.runtime.capture().is_some()
    }

    pub(in crate::gui) fn cancel_capture(&mut self) -> Result<()> {
        self.runtime
            .apply_binding_command(BindingCommand::CancelCapture)
    }

    pub(in crate::gui) fn binding_action(
        &mut self,
        ui: &mut Presentation,
        command: BindingCommand,
    ) -> Result<()> {
        if self.exit.is_some() {
            return Ok(());
        }
        let result = self.runtime.apply_binding_command(command);
        self.sync(ui);
        result
    }

    pub(in crate::gui) fn action(
        &mut self,
        ui: &mut Presentation,
        name: Action,
        value: &str,
    ) -> Result<()> {
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
                    if !self.runtime.config.options.close_hint_seen {
                        self.desktop
                            .notify("TapRelay", &self.tr(keys::TRAY_CLOSE_HINT));
                        self.runtime.config.options.close_hint_seen = true;
                        self.save_pending = true;
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
                self.save_pending = true;
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
                self.save_pending = true;
            }
            Action::WizardFinish => {
                self.cancel_capture()?;
                self.runtime.config.wizard.dismissed = true;
                self.save_pending = true;
                ui.mode = 2;
                ui.page = 0;
            }
            Action::Theme => {
                self.runtime.config.options.theme = ThemeSetting::from_name(value)
                    .with_context(|| format!("Unknown theme: {value}"))?;
                self.save_pending = true;
            }
            Action::Language => {
                self.runtime.config.options.language = Language::from_name(value)
                    .with_context(|| format!("Unknown language: {value}"))?;
                self.save_pending = true;
            }
            Action::SetStartHidden => {
                self.runtime.config.options.start_hidden = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetAutoListen => {
                self.runtime.config.options.auto_listen = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetPassthroughOverlay => {
                self.runtime.config.options.passthrough_overlay = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetPassthroughReverseScroll => {
                self.runtime
                    .set_passthrough_reverse_scroll(action::toggle(value)?)?;
                self.save_pending = true;
            }
            Action::SetPassthroughMouseReportRate => {
                self.runtime
                    .set_passthrough_mouse_report_rate(value.parse()?)?;
                self.save_pending = true;
            }
            Action::SetCloseToTray => {
                self.runtime.config.options.close_to_tray = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetAlwaysAdmin => {
                self.runtime.config.options.always_admin = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetNotifications => {
                self.runtime.config.options.notifications = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetConnectionWaitWarning => {
                self.runtime.config.options.connection_wait_warning = action::toggle(value)?;
                self.save_pending = true;
            }
            Action::SetAutostart => {
                // Read back what the OS accepted instead of trusting the request.
                self.runtime.config.options.autostart =
                    self.desktop.set_autostart(action::toggle(value)?)?;
                self.save_pending = true;
            }
            Action::Elevate => {
                self.cancel_capture()?;
                self.exit = Some(ExitIntent::Elevate);
            }
        }
        self.sync(ui);
        self.save_config();
        Ok(())
    }
}
