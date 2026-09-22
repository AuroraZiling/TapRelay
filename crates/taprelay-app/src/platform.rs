use anyhow::Result;
use taprelay_core::{
    command::QueuedCommand,
    function::FunctionConfigs,
    input_router::{PhysicalInput, RouteResult, RoutedInput, RouterReason},
    ports::BackendError,
    state::Snapshot,
};
use tokio::sync::{mpsc, oneshot};
pub trait InputSource {
    fn attach_passthrough(&self, _link: Option<taprelay_core::passthrough::InputLink>) {}
    fn passthrough_error(&self) -> Option<String> {
        None
    }
    fn is_finished(&self) -> bool;
    fn configure(
        &self,
        _functions: &FunctionConfigs,
        _listening: bool,
        _recording: bool,
        _revision: u64,
    ) -> RouteResult {
        RouteResult::default()
    }
    fn terminate(&self, _reason: RouterReason) -> RouteResult {
        RouteResult::default()
    }
    /// Replay an input that the synchronous platform hook consumed. The
    /// default keeps non-Windows fakes and the recorder independent of native
    /// input injection.
    fn replay(&self, _input: PhysicalInput) -> Result<()> {
        Ok(())
    }
    fn failure(&self) -> Option<String> {
        None
    }
}
pub trait Transport {
    fn input_link(&self) -> Option<taprelay_core::passthrough::InputLink> {
        None
    }
    fn snapshot(&mut self) -> Option<Snapshot>;
    fn is_finished(&self) -> bool {
        false
    }
    fn refresh(&self) -> Result<()>;
    fn restart(&self) -> Result<u64>;
    fn select(&self, id: String) -> Result<u64>;
    fn pair(&self, _id: String) -> Result<()> {
        anyhow::bail!("Pairing handoff unavailable")
    }
    fn disconnect(&self) -> Result<u64> {
        anyhow::bail!("Disconnect unavailable")
    }
    fn bluetooth_settings(&self) -> Result<()> {
        anyhow::bail!("Bluetooth settings unavailable")
    }
    fn invalidate(&self);
    fn send(&self, command: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>>;
}
#[cfg(windows)]
impl InputSource for taprelay_windows::input::InputHandle {
    fn attach_passthrough(&self, link: Option<taprelay_core::passthrough::InputLink>) {
        self.attach_passthrough(link);
    }
    fn passthrough_error(&self) -> Option<String> {
        self.passthrough_error()
    }
    fn is_finished(&self) -> bool {
        self.is_finished()
    }
    fn failure(&self) -> Option<String> {
        self.failure()
    }
    fn replay(&self, input: PhysicalInput) -> Result<()> {
        Ok(self.replay(input)?)
    }
    fn configure(
        &self,
        functions: &FunctionConfigs,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult {
        self.configure(functions, listening, recording, revision)
    }
    fn terminate(&self, reason: RouterReason) -> RouteResult {
        self.terminate(reason)
    }
}
/// A closed watch can still contain an unread terminal error. Always read it,
/// and never let a dead worker leave an old ready state visible.
pub(crate) fn poll_snapshot(
    state: &mut tokio::sync::watch::Receiver<Snapshot>,
) -> Option<Snapshot> {
    match state.has_changed() {
        Ok(false) => None,
        Ok(true) => Some(state.borrow_and_update().clone()),
        Err(_) => {
            let mut last = state.borrow_and_update().clone();
            taprelay_core::devices::revoke_session(&mut last);
            last.service = false;
            last.broadcasting = false;
            last.last_error
                .get_or_insert_with(|| "Bluetooth worker stopped; retry to restart".into());
            Some(last)
        }
    }
}

pub fn input(sender: mpsc::Sender<RoutedInput>) -> Result<Box<dyn InputSource>> {
    #[cfg(windows)]
    {
        Ok(Box::new(taprelay_windows::input::InputHandle::start(
            sender,
        )?))
    }
    #[cfg(not(windows))]
    {
        let _ = sender;
        anyhow::bail!("Input is not implemented on this platform")
    }
}
pub fn key_name(key: u8) -> String {
    #[cfg(windows)]
    {
        taprelay_windows::input::key_name(key)
    }
    #[cfg(not(windows))]
    {
        taprelay_core::input::key_name(key)
    }
}
pub fn keyboard_layout() -> usize {
    #[cfg(windows)]
    {
        taprelay_windows::input::keyboard_layout()
    }
    #[cfg(not(windows))]
    {
        0
    }
}
pub fn transport(remembered: Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>> {
    #[cfg(windows)]
    {
        Ok(Box::new(crate::bluetooth_worker::Handle::start(
            remembered,
        )?))
    }
    #[cfg(not(windows))]
    {
        let _ = remembered;
        anyhow::bail!("Bluetooth is not implemented on this platform")
    }
}
#[cfg(windows)]
pub use taprelay_windows::InstanceLock;
#[cfg(windows)]
pub use taprelay_windows::desktop;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_watch_preserves_terminal_error_and_revokes_ready() {
        let (tx, mut rx) = tokio::sync::watch::channel(Snapshot::default());
        tx.send_replace(Snapshot {
            ready: true,
            selected: Some("old-receiver".into()),
            service: true,
            last_error: Some("Apartment failed".into()),
            ..Default::default()
        });
        drop(tx);
        let state = poll_snapshot(&mut rx).unwrap();
        assert_eq!(state.last_error.as_deref(), Some("Apartment failed"));
        assert!(!state.ready && !state.service);
        assert!(state.selected.is_none() && state.selected_target().is_none());
    }
}
