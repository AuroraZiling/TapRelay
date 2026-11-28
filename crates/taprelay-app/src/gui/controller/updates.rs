use super::{Controller, Presentation, Shell, desktop};
use crate::{
    gui::presentation::updates::{UpdateOffer, UpdateStatus},
    updates::{self, Check, Notes, PreparedUpdate, Target},
};
use anyhow::{Result, anyhow};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

const STARTUP_DELAY: Duration = Duration::from_secs(15);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

// Dropping a receiver also discards late results from a superseded check or offer.
fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

fn finished<T>(task: &mut Option<Receiver<T>>) -> Option<Result<T>> {
    let result = match task.as_ref()?.try_recv() {
        Ok(value) => Ok(value),
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(anyhow!("Update task stopped unexpectedly")),
    };
    *task = None;
    Some(result)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Automatic,
    Manual,
    #[cfg(windows)]
    Confirmed,
}

pub(super) struct Updates {
    next_check: Instant,
    check: Option<Receiver<Result<Check>>>,
    intent: Intent,
    status: UpdateStatus,
    target: Option<Target>,
    dialog: bool,
    notes: Option<Arc<Notes>>,
    requested_notes: bool,
    notes_task: Option<Receiver<Option<Notes>>>,
    download: Option<Receiver<Result<PreparedUpdate>>>,
    progress: Option<Receiver<i16>>,
    #[cfg(test)]
    offline: Option<Offline>,
}

#[cfg(test)]
#[derive(Default)]
struct Offline {
    latest: Option<Target>,
    opened: Vec<String>,
}

impl Updates {
    pub(super) fn new(now: Instant, _desktop: &Shell) -> Self {
        Self {
            next_check: now + STARTUP_DELAY,
            check: None,
            intent: Intent::Automatic,
            status: UpdateStatus::Idle,
            target: None,
            dialog: false,
            notes: None,
            requested_notes: false,
            notes_task: None,
            download: None,
            progress: None,
            #[cfg(test)]
            offline: matches!(_desktop, Shell::Headless { .. }).then(Offline::default),
        }
    }

    pub(super) fn auto_changed(&mut self, enabled: bool, now: Instant) {
        if enabled {
            self.next_check = now + STARTUP_DELAY;
        } else if self.intent == Intent::Automatic {
            if self.check.take().is_some() {
                self.status = UpdateStatus::Idle;
            }
            self.dismiss();
        }
    }

    pub(super) fn check(&mut self, manual: bool, now: Instant) {
        if self.check.is_some() {
            if manual && self.intent == Intent::Automatic {
                self.intent = Intent::Manual;
            }
            return;
        }
        if self.dialog || self.status.committing() {
            return;
        }
        self.next_check = now + INTERVAL;
        self.intent = if manual {
            Intent::Manual
        } else {
            Intent::Automatic
        };
        self.status = UpdateStatus::Checking;
        self.target = None;
        self.notes = None;
        self.notes_task = None;
        self.requested_notes = false;
        #[cfg(test)]
        if let Some(offline) = &self.offline {
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(offline
                .latest
                .clone()
                .map_or(Check::Current, Check::Available)))
                .unwrap();
            self.check = Some(rx);
            return;
        }
        self.check = Some(background(updates::check));
    }

    fn discovery(&mut self, now: Instant, auto: bool, skipped: Option<&str>) {
        if auto && now >= self.next_check {
            self.check(false, now);
        }
        if let Some(result) = finished(&mut self.check) {
            self.status = match result.and_then(|check| check) {
                Ok(Check::Current) => UpdateStatus::Current,
                Ok(Check::Unsupported) => UpdateStatus::Unsupported,
                Ok(Check::Available(target)) => {
                    self.dialog =
                        self.intent != Intent::Automatic || skipped != Some(&target.tag());
                    self.target = Some(target);
                    UpdateStatus::Available
                }
                Err(error) => {
                    tracing::warn!("Update check failed: {error:#}");
                    UpdateStatus::CheckFailed
                }
            };
        }
    }

    pub(super) fn load_notes(&mut self) {
        let Some(target) = self
            .target
            .clone()
            .filter(|_| self.dialog && !self.requested_notes)
        else {
            return;
        };
        self.requested_notes = true;
        #[cfg(test)]
        if self.offline.is_some() {
            return;
        }
        self.notes_task = Some(background(move || updates::notes(&target)));
    }

    pub(super) fn dismiss(&mut self) -> Option<String> {
        if !self.dialog || self.status.committing() {
            return None;
        }
        self.dialog = false;
        Some(self.target.as_ref()?.tag())
    }

    fn open_url(&mut self, url: String) -> Result<()> {
        #[cfg(test)]
        if let Some(offline) = &mut self.offline {
            offline.opened.push(url);
            return Ok(());
        }
        desktop::open(&url)?;
        Ok(())
    }

    pub(super) fn open_release_notes(&mut self) -> Result<()> {
        if self.dialog
            && let Some(target) = &self.target
        {
            self.open_url(target.release_url())?;
        }
        Ok(())
    }

    pub(super) fn present(&self, ui: &mut Presentation) {
        ui.update_status = self.status;
        ui.update_dialog_open = self.dialog;
        ui.update_offer = self.target.as_ref().map(|target| UpdateOffer {
            tag: target.tag(),
            portable: target.portable_url().is_some(),
            show_notes: self.intent == Intent::Manual || self.requested_notes,
            notes_loading: self.notes_task.is_some(),
            notes: self.notes.clone(),
        });
    }
}

impl Controller {
    pub(in crate::gui) fn poll_updates(&mut self, now: Instant) {
        let options = &self.runtime.config.options;
        let state = &mut self.updates;
        // A tray-only session can refresh a pending offer without changing the version
        // under a visible confirmation button.
        if options.auto_check_updates
            && now >= state.next_check
            && self.hidden
            && !state.status.committing()
        {
            state.dialog = false;
        }
        state.discovery(
            now,
            options.auto_check_updates,
            options.skipped_update_version.as_deref(),
        );
        #[cfg(windows)]
        if state.intent == Intent::Confirmed && state.check.is_none() {
            state.intent = Intent::Automatic;
            if state.dialog
                && let Some(target) = state.target.clone()
            {
                state.start_download(target);
            } else if state.status == UpdateStatus::CheckFailed {
                self.say(self.tr("updates.check_failed"));
                return;
            }
        }
        if let Some(result) = finished(&mut state.notes_task) {
            state.notes = result.ok().flatten().map(Arc::new);
        }
        if let Some(progress) = &state.progress
            && let Some(value) = progress.try_iter().last()
        {
            state.status = UpdateStatus::Downloading(value.clamp(0, 100) as u8);
        }
        if let Some(result) = finished(&mut state.download) {
            state.progress = None;
            match result.and_then(|result| result) {
                Ok(prepared) => self.restart_downloaded_update(prepared),
                Err(error) => {
                    tracing::warn!("Update download failed: {error:#}");
                    state.status = UpdateStatus::DownloadFailed;
                }
            }
        }
    }

    pub(super) fn accept_update(&mut self) -> Result<()> {
        let state = &mut self.updates;
        let Some(target) = state
            .target
            .clone()
            .filter(|_| state.dialog && !state.status.committing())
        else {
            return Ok(());
        };
        if let Some(url) = target.portable_url() {
            state.open_url(url)?;
            state.dismiss();
        } else {
            #[cfg(windows)]
            if self.status.elevated {
                state.status = UpdateStatus::Applying;
                if let Err(error) = self.begin_user_update(&target.tag()) {
                    self.update_restart_failed(&error);
                }
                return Ok(());
            }
            state.start_download(target);
        }
        Ok(())
    }

    fn restart_downloaded_update(&mut self, prepared: PreparedUpdate) {
        self.updates.status = UpdateStatus::Applying;
        if let Err(error) = self.begin_update_restart(prepared) {
            self.update_restart_failed(&error);
        }
    }

    pub(super) fn update_restart_failed(&mut self, error: &anyhow::Error) {
        tracing::error!("Could not restart for update: {error:#}");
        self.updates.status = UpdateStatus::ApplyFailed;
        #[cfg(windows)]
        if error.is::<super::lifecycle::UpdateUserHandoffError>() {
            self.say(self.tr("updates.user_handoff_failed"));
        }
    }

    #[cfg(windows)]
    pub(in crate::gui) fn resume_confirmed_update(&mut self, tag: &str) {
        let tag = tag.to_owned();
        let state = &mut self.updates;
        state.check = Some(background(move || updates::check_confirmed(&tag)));
        state.intent = Intent::Confirmed;
        state.status = UpdateStatus::Checking;
        state.next_check = Instant::now() + INTERVAL;
    }

    #[cfg(test)]
    pub(in crate::gui) fn offer_update_for_test(
        &mut self,
        ui: &mut Presentation,
        target: Target,
        notes: Option<Notes>,
        manual: bool,
    ) {
        let state = &mut self.updates;
        state.offline.as_mut().unwrap().latest = Some(target);
        state.check(manual, Instant::now());
        state.discovery(Instant::now(), false, None);
        state.notes = notes.map(Arc::new);
        self.sync(ui);
    }

    #[cfg(test)]
    pub(in crate::gui) fn set_notes_loading_for_test(&mut self, loading: bool) {
        self.updates.notes_task = loading.then(|| mpsc::channel().1);
    }
}

impl Updates {
    fn start_download(&mut self, target: Target) {
        let (tx, rx) = mpsc::channel();
        self.progress = Some(rx);
        self.status = UpdateStatus::Downloading(0);
        #[cfg(test)]
        if self.offline.is_some() {
            self.download = Some(background(|| Err(anyhow!("No test package supplied"))));
            return;
        }
        self.download = Some(background(move || updates::download(&target, tx)));
    }
}

#[cfg(test)]
mod tests;
