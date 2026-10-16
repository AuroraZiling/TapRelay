pub(crate) mod administrator;
pub(crate) mod bluetooth_worker;
pub(crate) mod capture_key;

use anyhow::Result;
use taprelay_core::{
    command::QueuedCommand,
    function::FunctionConfigs,
    input_router::{PhysicalInput, RouteResult, RoutedInput, RouterReason},
    ports::BackendError,
    state::Snapshot,
};
pub use taprelay_windows::{
    InstanceLock, desktop, foreground_apps,
    input::{key_name, keyboard_layout},
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
        functions: &FunctionConfigs,
        mappings: &[taprelay_core::mapping::CustomMapping],
        rules: &taprelay_core::foreground_app::ForegroundAppRules,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult;
    fn terminate(&self, reason: RouterReason) -> RouteResult;
    fn replay(&self, input: PhysicalInput) -> Result<()>;
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
    fn invalidate(&self);
    fn send(&self, command: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>>;
}
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
    fn configure(
        &self,
        functions: &FunctionConfigs,
        mappings: &[taprelay_core::mapping::CustomMapping],
        rules: &taprelay_core::foreground_app::ForegroundAppRules,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult {
        self.configure(functions, mappings, rules, listening, recording, revision)
    }
    fn terminate(&self, reason: RouterReason) -> RouteResult {
        self.terminate(reason)
    }
    fn replay(&self, input: PhysicalInput) -> Result<()> {
        Ok(self.replay(input)?)
    }
    fn failure(&self) -> Option<String> {
        self.failure()
    }
}

pub fn input(sender: mpsc::Sender<RoutedInput>) -> Result<Box<dyn InputSource>> {
    Ok(Box::new(taprelay_windows::input::InputHandle::start(
        sender,
    )?))
}
pub fn transport(remembered: Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>> {
    Ok(Box::new(bluetooth_worker::Handle::start(remembered)?))
}
