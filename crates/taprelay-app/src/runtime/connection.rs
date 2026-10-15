use super::*;

impl Runtime {
    pub fn start_bluetooth(&mut self) -> Result<()> {
        match self.start_bluetooth_with(platform::transport) {
            Err(error) if self.startup_restore.is_some() => {
                tracing::warn!(%error, retry_seconds = 5, "Bluetooth startup will retry");
                Ok(())
            }
            result => result,
        }
    }

    pub(super) fn start_bluetooth_with(
        &mut self,
        create: impl FnOnce(Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>>,
    ) -> Result<()> {
        self.startup_retry_at = Some(Instant::now() + std::time::Duration::from_secs(5));
        self.state.ready = false;
        self.state.service = false;
        self.state.broadcasting = false;
        self.state.selected = None;
        self.state.last_error = None;
        self.error = None;
        self.invalidate(RouterReason::SessionChanged);
        if self
            .transport
            .as_ref()
            .is_some_and(|transport| transport.is_finished())
        {
            self.transport.take();
        }
        let result = if let Some(transport) = &self.transport {
            transport
                .restart()
                .map(|generation| self.required_generation = generation)
        } else {
            self.required_generation = 0;
            create(self.startup_restore.clone()).map(|transport| self.transport = Some(transport))
        };
        if let Err(error) = &result {
            self.state.last_error = Some(error.to_string());
        }
        result
    }

    pub(super) fn retry_startup_with(
        &mut self,
        now: Instant,
        create: impl FnOnce(Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>>,
    ) {
        if self.startup_restore.is_none()
            || self
                .transport
                .as_ref()
                .is_some_and(|transport| !transport.is_finished())
            || self.startup_retry_at.is_none_or(|due| now < due)
        {
            return;
        }
        tracing::info!("Retrying Bluetooth worker for startup restore");
        if let Err(error) = self.start_bluetooth_with(create) {
            tracing::warn!(%error, retry_seconds = 5, "Bluetooth startup will retry");
            self.state.last_error = Some(error.to_string());
        }
        self.startup_retry_at = Some(now + std::time::Duration::from_secs(5));
    }

    pub fn pair(&mut self, id: String) -> Result<()> {
        self.startup_restore = None;
        if self.state.selected.as_ref() == Some(&id)
            && self.state.pairing_handoff == taprelay_core::devices::PairingHandoff::WaitingForOS
        {
            return Ok(());
        }
        self.choose(id.clone())?;
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .pair(id)?;
        self.state.pairing_handoff = taprelay_core::devices::PairingHandoff::WaitingForOS;
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<()> {
        self.startup_restore = None;
        self.remembered_device = None;
        let result = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .disconnect();
        self.state.ready = false;
        self.state.selected = None;
        self.invalidate(RouterReason::SessionChanged);
        self.required_generation = result?;
        Ok(())
    }

    pub fn bluetooth_settings(&self) -> Result<()> {
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .bluetooth_settings()
    }

    pub fn refresh(&self) -> Result<()> {
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .refresh()
    }

    pub fn choose(&mut self, id: String) -> Result<()> {
        let restoring = self.startup_restore.take().is_some();
        if !restoring
            && self.state.selected.as_ref() == Some(&id)
            && self.state.selected_target().is_some_and(|target| {
                matches!(
                    target.connection,
                    taprelay_core::devices::Connection::AwaitingHostSubscription
                        | taprelay_core::devices::Connection::Synchronizing
                        | taprelay_core::devices::Connection::Connected
                )
            })
        {
            return Ok(());
        }
        self.required_generation = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .select(id.clone())?;
        self.state.ready = false;
        self.state.selected = Some(id);
        self.invalidate(RouterReason::SessionChanged);
        Ok(())
    }
}
