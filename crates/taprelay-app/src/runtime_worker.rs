//! UI facade. The engine and native adapters are constructed and destroyed on
//! one worker; UI work never holds a lock needed by input delivery.
use crate::{
    action::BindingCommand,
    config::{Config, Device},
    feedback::TestStatus,
    runtime::{CaptureSession, Runtime},
};
use anyhow::{Context, Result};
use std::{
    ops::{Deref, DerefMut},
    sync::mpsc,
    thread,
    time::Duration,
};
use taprelay_core::{function::FunctionConfigs, input::InputEvent, state::Snapshot};

type Command = Box<dyn FnOnce(&mut Runtime) + Send>;

pub struct View {
    functions: FunctionConfigs,
    remembered_device: Option<Device>,
    pub state: Snapshot,
    pub matched: u64,
    pub error: Option<String>,
    pub listening: bool,
    capture: Option<CaptureSession>,
    pub test: TestStatus,
    pub bindings_revision: u64,
}
impl View {
    pub fn capture(&self) -> Option<&CaptureSession> {
        self.capture.as_ref()
    }
    fn take(runtime: &mut Runtime) -> Self {
        Self {
            functions: runtime.functions.clone(),
            remembered_device: runtime.remembered_device.clone(),
            state: runtime.state.clone(),
            matched: runtime.matched,
            error: runtime.error.take(),
            listening: runtime.listening,
            capture: runtime.capture().cloned(),
            test: runtime.test.clone(),
            bindings_revision: runtime.bindings_revision,
        }
    }
}
pub struct RuntimeHandle {
    pub config: Config,
    view: View,
    bindings_changed: bool,
    pub window_keys: Vec<InputEvent>,
    commands: Option<mpsc::SyncSender<Command>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Deref for RuntimeHandle {
    type Target = View;
    fn deref(&self) -> &View {
        &self.view
    }
}
impl DerefMut for RuntimeHandle {
    fn deref_mut(&mut self) -> &mut View {
        &mut self.view
    }
}
impl RuntimeHandle {
    pub fn new(config: Config) -> Result<Self> {
        let runtime_config = config.clone();
        let (tx, rx) = mpsc::sync_channel::<Command>(64);
        let (started, ready) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("taprelay-runtime".into())
            .spawn(move || {
                let mut runtime = Runtime::new(runtime_config);
                if started.send(View::take(&mut runtime)).is_err() {
                    return;
                }
                loop {
                    // Drain input before configuration commands: a revision barrier
                    // must not overtake physical edges already accepted by the hook.
                    runtime.tick();
                    match rx.try_recv() {
                        Ok(command) => command(&mut runtime),
                        Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {
                            thread::park_timeout(Duration::from_millis(50));
                        }
                    }
                }
                runtime.shutdown();
            })?;
        let view = ready.recv().context("Runtime worker failed to start")?;
        Ok(Self {
            config,
            view,
            bindings_changed: false,
            window_keys: vec![],
            commands: Some(tx),
            worker: Some(worker),
        })
    }

    fn update(
        &mut self,
        command: impl FnOnce(&mut Runtime) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.commands
            .as_ref()
            .context("Runtime stopped")?
            .try_send(Box::new(move |runtime| {
                let result = command(runtime);
                let _ = tx.send((result, View::take(runtime)));
            }))
            .map_err(|_| anyhow::anyhow!("Runtime command queue unavailable"))?;
        self.worker
            .as_ref()
            .context("Runtime stopped")?
            .thread()
            .unpark();
        let (result, view) = rx
            .recv_timeout(Duration::from_secs(5))
            .context("Runtime command timed out")?;
        self.apply(view);
        result
    }
    fn apply(&mut self, mut view: View) {
        self.bindings_changed |= view.bindings_revision != self.view.bindings_revision;
        self.config.functions = std::mem::take(&mut view.functions);
        self.config.remembered_device = view.remembered_device.take();
        if view.error.is_none() {
            view.error = self.view.error.take();
        }
        self.view = view;
    }
    pub fn tick(&mut self) {
        let keys = std::mem::take(&mut self.window_keys);
        if let Err(error) = self.update(move |runtime| {
            runtime.window_keys.extend(keys);
            Ok(())
        }) {
            self.view.error = Some(error.to_string());
        }
    }
    pub fn take_bindings_changed(&mut self) -> bool {
        std::mem::take(&mut self.bindings_changed)
    }
    pub fn consume_ui_input(&mut self) {
        if let Err(e) = self.update(|r| {
            r.consume_ui_input();
            Ok(())
        }) {
            self.view.error = Some(e.to_string());
        }
    }
    pub fn apply_binding_command(&mut self, command: BindingCommand) -> Result<()> {
        self.update(move |runtime| runtime.apply_binding_command(command))
    }
    pub fn shutdown(&mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            if worker.join().is_err() {
                self.view.error = Some("Runtime worker panicked".into());
            }
        }
    }
    pub fn start_bluetooth(&mut self) -> Result<()> {
        self.update(move |r| r.start_bluetooth())
    }
    pub fn set_listening(&mut self, on: bool) -> Result<()> {
        self.update(move |r| r.set_listening(on))
    }
    pub fn set_passthrough_reverse_scroll(&mut self, reverse: bool) -> Result<()> {
        self.update(move |runtime| {
            runtime.set_passthrough_reverse_scroll(reverse);
            Ok(())
        })?;
        self.config.options.passthrough_reverse_scroll = reverse;
        Ok(())
    }
    pub fn pair(&mut self, id: String) -> Result<()> {
        self.update(move |r| r.pair(id))
    }
    pub fn choose(&mut self, id: String) -> Result<()> {
        self.update(move |r| r.choose(id))
    }
    pub fn disconnect(&mut self) -> Result<()> {
        self.update(move |r| r.disconnect())
    }
    pub fn bluetooth_settings(&mut self) -> Result<()> {
        self.update(move |r| r.bluetooth_settings())
    }
    pub fn refresh(&mut self) -> Result<()> {
        self.update(move |r| r.refresh())
    }
    pub fn send(&mut self) -> Result<()> {
        self.update(move |r| r.send())
    }
}
impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taprelay_core::function::FunctionId;

    #[test]
    fn worker_updates_preserve_ui_preferences_and_unread_results() {
        let mut handle = RuntimeHandle::new(Config::default()).unwrap();
        handle.config.options.theme = crate::config::Theme::Dark;
        handle.config.window.width = 1100.;
        handle.config.wizard.dismissed = true;
        let preferences = serde_json::to_value(&handle.config).unwrap();
        let result = handle.update(|runtime| {
            runtime
                .functions
                .get_mut(&FunctionId::MediaMute)
                .unwrap()
                .enabled = true;
            runtime.remembered_device = Some(Device::from_target(&Default::default()));
            runtime.error = Some("pending notification".into());
            runtime.bindings_changed();
            anyhow::bail!("command failed")
        });
        assert_eq!(result.unwrap_err().to_string(), "command failed");
        handle.tick();
        let updated = serde_json::to_value(&handle.config).unwrap();
        for field in ["options", "window", "wizard"] {
            assert_eq!(updated[field], preferences[field]);
        }
        assert!(handle.config.functions[&FunctionId::MediaMute].enabled);
        assert!(handle.config.remembered_device.is_some());
        assert_eq!(handle.error.as_deref(), Some("pending notification"));
        assert!(handle.take_bindings_changed());
        assert!(!handle.take_bindings_changed());
        handle.tick();
        assert!(!handle.take_bindings_changed());
    }
}
